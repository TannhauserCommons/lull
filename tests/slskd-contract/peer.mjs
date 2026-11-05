// A stand-in peer that shares files and records the offset each download asks for.
//
// The flow, as Soulseek.NET 10.0.2 runs it (SoulseekClient.DownloadToStreamAsync, PeerMessageHandler,
// PeerConnectionManager, ListenerHandler):
// 1. slskd connects to us (the address the server gave it) and opens a message connection: PeerInit 'P'.
// 2. It sends TransferRequest (40, direction download, its token, filename).
// 3. We answer TransferResponse (41, its token, not allowed, "Queued"), as busy real peers do, then offer the
//    upload: TransferRequest (40, direction upload, our token, filename, size).
// 4. slskd answers TransferResponse (41, our token, allowed) and waits for us to connect to its listen port.
// 5. We open the transfer connection: PeerInit 'F', then our token as 4 raw bytes.
// 6. slskd writes the start offset as 8 raw bytes (little-endian). That is the number this test is about.
// 7. We send the file from that offset; slskd closes the connection when it has `size - offset` bytes.
// If our connection in step 5 failed, slskd would connect to us instead ("second chance"); that is served too.

import { createConnection, createServer } from 'node:net';
import { EventEmitter } from 'node:events';
import { Builder, Direction, Init, Peer, Reader, SocketReader, int32, trace } from './protocol.mjs';

/** A paced file arrives at 64 KiB every 100 ms, about 640 KiB a second: a few seconds for a song-sized file. */
const PACE = { bytes: 64 * 1024, everyMs: 100 };

/**
 * @param {object} o
 * @param {string} o.username   who we are on the stand-in network
 * @param {string} o.host       the interface to listen on
 * @param {Map<string, Buffer>} o.files   remote filename -> contents
 * @param {{host: string, port: number}} o.downloader   where slskd's listen port is reachable from here
 * @param {Set<string>} [o.paced]   files sent slowly (PACE), so a test can look at slskd's file while it arrives
 */
export async function startPeer({ username, host, files, downloader, paced = new Set() }) {
  const events = new EventEmitter();
  const sockets = new Set();
  const offered = new Map(); // our upload token -> filename, between steps 3 and 5
  let nextToken = 1000;

  const track = (socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.on('error', () => {}); // a reset after a finished transfer is not news; reads report what matters
    socket.setNoDelay(true);
  };

  const server = createServer((socket) => {
    track(socket);
    inbound(socket).catch((e) => trace('peer inbound ended:', e.message));
  });

  async function inbound(socket) {
    const reader = new SocketReader(socket);
    const r = new Reader(await reader.frame());
    const code = r.byte();
    if (code !== Init.PeerInit) throw new Error(`unexpected init code ${code}`);
    const from = r.string();
    const type = r.string();
    trace(`peer <- PeerInit ${type} from ${from}`);
    if (type === 'P') return messages(socket, reader);
    if (type === 'F') {
      // slskd's second-chance connection: its PeerInit, then our token as raw bytes.
      const token = (await reader.read(4)).readInt32LE(0);
      return transfer(socket, reader, token, 'inbound');
    }
  }

  async function messages(socket, reader) {
    for (;;) {
      const r = new Reader(await reader.frame());
      const code = r.int();
      trace('peer <-', code);

      if (code === Peer.TransferRequest) {
        const direction = r.int();
        const token = r.int();
        const filename = r.string();
        if (direction !== Direction.Download) continue;
        if (!files.has(filename)) {
          socket.write(new Builder(Peer.TransferResponse).int(token).byte(0).string('File not shared.').build());
          continue;
        }
        socket.write(new Builder(Peer.TransferResponse).int(token).byte(0).string('Queued').build());
        offer(socket, filename);
      } else if (code === Peer.QueueDownload) {
        const filename = r.string();
        if (files.has(filename)) offer(socket, filename);
      } else if (code === Peer.PlaceInQueueRequest) {
        const filename = r.string();
        socket.write(new Builder(Peer.PlaceInQueueResponse).string(filename).int(0).build());
      } else if (code === Peer.TransferResponse) {
        const token = r.int();
        const allowed = r.byte() === 1;
        const filename = offered.get(token);
        if (filename === undefined) continue;
        if (!allowed) { events.emit('refused', { filename, reason: r.more ? r.string() : '' }); continue; }
        connectTransfer(token).catch((e) => events.emit('error', e));
      }
    }
  }

  function offer(socket, filename) {
    const token = nextToken++;
    offered.set(token, filename);
    socket.write(new Builder(Peer.TransferRequest)
      .int(Direction.Upload).int(token).string(filename).long(files.get(filename).length)
      .build());
  }

  async function connectTransfer(token) {
    const socket = createConnection(downloader);
    track(socket);
    const reader = new SocketReader(socket);
    await new Promise((resolve, reject) => socket.once('connect', resolve).once('error', reject));
    socket.write(new Builder(Init.PeerInit, { initCode: true }).string(username).string('F').int(token).build());
    socket.write(int32(token));
    await transfer(socket, reader, token, 'outbound');
  }

  async function transfer(socket, reader, token, connection) {
    const filename = offered.get(token);
    if (filename === undefined) { socket.destroy(); return; }
    offered.delete(token);

    const offset = Number((await reader.read(8)).readBigInt64LE(0));
    const file = files.get(filename);
    events.emit('offset', { filename, offset, connection });

    const closed = new Promise((resolve) => socket.once('close', resolve));
    const rest = file.subarray(Math.min(offset, file.length));
    if (paced.has(filename)) {
      for (let at = 0; at < rest.length && !socket.destroyed; at += PACE.bytes) {
        socket.write(rest.subarray(at, at + PACE.bytes));
        await new Promise((r) => setTimeout(r, PACE.everyMs));
      }
    } else {
      socket.write(rest);
    }
    await closed; // slskd closes once it has read everything; closing first could race its last read
    events.emit('served', { filename, offset, bytes: file.length - offset, connection });
  }

  await new Promise((resolve, reject) => server.once('error', reject).listen(0, host, resolve));

  return {
    port: server.address().port,
    events,
    /** Resolves with the next completed transfer of `filename`: { offset, bytes, connection }. */
    nextServed(filename) {
      return new Promise((resolve, reject) => {
        const onServed = (e) => { if (e.filename === filename) { off(); resolve(e); } };
        const onRefused = (e) => { if (e.filename === filename) { off(); reject(new Error(`slskd refused the upload offer: ${e.reason}`)); } };
        const onError = (e) => { off(); reject(e); };
        const off = () => { events.off('served', onServed); events.off('refused', onRefused); events.off('error', onError); };
        events.on('served', onServed).on('refused', onRefused).on('error', onError);
      });
    },
    close() { server.close(); for (const s of sockets) s.destroy(); },
  };
}
