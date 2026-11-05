// The slice of the Soulseek wire format the stand-ins need, matched to Soulseek.NET 10.0.2 (the library inside
// slskd 0.26.2): src/Messaging/MessageBuilder.cs, MessageReader.cs and the message classes under Messaging/Messages.
//
// Every message is framed as a 4-byte little-endian length, then the body. Server and peer bodies start with a
// 4-byte code; peer init bodies ('P'/'F' connection openers) start with a 1-byte code. Strings are a 4-byte length
// and UTF-8 bytes. A transfer ('F') connection drops framing after its init: raw token, raw offset, raw file bytes.

export const Server = { Login: 1, SetListenPort: 2, GetPeerAddress: 3, GetStatus: 7, ConnectToPeer: 18, GetUserStats: 36, CheckPrivileges: 92 };
export const Peer = { TransferRequest: 40, TransferResponse: 41, QueueDownload: 43, PlaceInQueueResponse: 44, PlaceInQueueRequest: 51 };
export const Init = { PierceFirewall: 0, PeerInit: 1 };
export const Direction = { Download: 0, Upload: 1 };

export const trace = process.env.CONTRACT_TRACE ? (...a) => console.log('  [trace]', ...a) : () => {};

/** Builds one framed message. `code` is written as 4 bytes, or as 1 byte for peer init messages. */
export class Builder {
  #parts = [];
  constructor(code, { initCode = false } = {}) {
    this.#parts.push(initCode ? Buffer.from([code]) : int32(code));
  }
  byte(v) { this.#parts.push(Buffer.from([v])); return this; }
  int(v) { this.#parts.push(int32(v)); return this; }
  long(v) { const b = Buffer.alloc(8); b.writeBigInt64LE(BigInt(v)); this.#parts.push(b); return this; }
  string(s) { const b = Buffer.from(s, 'utf8'); this.#parts.push(int32(b.length), b); return this; }
  // Soulseek.NET reads an IPv4 address as 4 bytes and reverses them, so the wire carries the octets backwards.
  ip(dotted) { this.#parts.push(Buffer.from(dotted.split('.').map(Number).reverse())); return this; }
  build() { const body = Buffer.concat(this.#parts); return Buffer.concat([int32(body.length), body]); }
}

/** Reads fields from one message body (the length prefix already removed). */
export class Reader {
  #pos = 0;
  constructor(body) { this.body = body; }
  get more() { return this.#pos < this.body.length; }
  byte() { return this.body.readUInt8(this.#pos++); }
  int() { const v = this.body.readInt32LE(this.#pos); this.#pos += 4; return v; }
  long() { const v = this.body.readBigInt64LE(this.#pos); this.#pos += 8; return Number(v); }
  string() { const n = this.int(); const s = this.body.toString('utf8', this.#pos, this.#pos + n); this.#pos += n; return s; }
}

export function int32(v) { const b = Buffer.alloc(4); b.writeInt32LE(v); return b; }

/**
 * Turns a socket's data events into `await read(n)`, which is how both framed messages and the raw transfer
 * handshake are read. One read at a time; a closed socket rejects the pending and later reads.
 */
export class SocketReader {
  #buf = Buffer.alloc(0);
  #pending = null;
  #closed = null;

  constructor(socket) {
    socket.on('data', (d) => { this.#buf = Buffer.concat([this.#buf, d]); this.#pump(); });
    socket.on('close', () => { this.#closed ??= new Error('connection closed'); this.#pump(); });
    socket.on('error', (e) => { this.#closed = e; this.#pump(); });
  }

  read(n) {
    return new Promise((resolve, reject) => { this.#pending = { n, resolve, reject }; this.#pump(); });
  }

  async frame() {
    const length = (await this.read(4)).readInt32LE(0);
    return this.read(length);
  }

  #pump() {
    const p = this.#pending;
    if (!p) return;
    if (this.#buf.length >= p.n) {
      this.#pending = null;
      const out = this.#buf.subarray(0, p.n);
      this.#buf = this.#buf.subarray(p.n);
      p.resolve(out);
    } else if (this.#closed) {
      this.#pending = null;
      p.reject(this.#closed);
    }
  }
}
