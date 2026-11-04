// A stand-in Soulseek server: just enough for slskd to log in and to find a peer's address.
//
// What Soulseek.NET 10.0.2 waits on (SoulseekClient.cs):
// - Login: ConnectInternalAsync writes Login + SetListenPort and waits for the Login reply. Nothing else gates it.
// - GetPeerAddress: GetUserEndPointAsync waits for the reply naming that user; IP 0.0.0.0 / port 0 means offline.
// - GetUserStats and CheckPrivileges: slskd asks both right after login (Application.Client_LoggedIn). Unanswered,
//   they only time out into a logged error, but answering keeps slskd's log clean.
// Everything else (SetListenPort, SharedFoldersAndFiles, HaveNoParents, ConnectToPeer for the indirect race, ...)
// needs no reply and is ignored.

import { createServer } from 'node:net';
import { EventEmitter } from 'node:events';
import { Builder, Reader, Server, SocketReader, trace } from './protocol.mjs';

export async function startServer({ host }) {
  const events = new EventEmitter();
  const peers = new Map(); // username -> { ip, port }
  const sockets = new Set();

  const server = createServer((socket) => {
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    socket.setNoDelay(true);
    serve(socket).catch((e) => trace('server connection ended:', e.message));
  });

  async function serve(socket) {
    const reader = new SocketReader(socket);
    for (;;) {
      const r = new Reader(await reader.frame());
      const code = r.int();
      trace('server <-', code);

      if (code === Server.Login) {
        const username = r.string();
        socket.write(new Builder(Server.Login)
          .byte(1).string('Welcome to the contract test').ip('127.0.0.1').string('').byte(0)
          .build());
        events.emit('login', username);
      } else if (code === Server.GetPeerAddress) {
        const username = r.string();
        const peer = peers.get(username) ?? { ip: '0.0.0.0', port: 0 };
        socket.write(new Builder(Server.GetPeerAddress).string(username).ip(peer.ip).int(peer.port).build());
      } else if (code === Server.GetUserStats) {
        const username = r.string();
        // Soulseek.NET reads: speed (int), upload count (long), files (int), directories (int).
        socket.write(new Builder(Server.GetUserStats).string(username).int(0).long(0).int(0).int(0).build());
      } else if (code === Server.CheckPrivileges) {
        socket.write(new Builder(Server.CheckPrivileges).int(0).build());
      }
    }
  }

  await new Promise((resolve, reject) => server.once('error', reject).listen(0, host, resolve));

  return {
    port: server.address().port,
    events,
    /** Where slskd will be told a user is; set before slskd asks. */
    setPeer(username, ip, port) { peers.set(username, { ip, port }); },
    close() { server.close(); for (const s of sockets) s.destroy(); },
  };
}
