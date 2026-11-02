// A stand-in for slskd's REST API (v0), so the hub under test has a Soulseek that is logged in,
// shares its library and gets one fixed answer to every search: `node stub-slskd.mjs [port] [api key]`.
// The visual tests need a connected `soulseek` status and deterministic searches, never the real
// network. A search nobody answers is how lull spots a wedged slskd (crates/soulseek/src/recover.rs),
// so a stub that found nothing would put a "searches got no answers" banner over every page after a
// few searches. Only what crates/soulseek/src/slskd.rs calls is answered; anything else is a loud 404.

import { createServer } from 'node:http';

const port = Number(process.argv[2] ?? 5030);
const key = process.argv[3] ?? 'ui-test-key';
const SEARCH = {
  id: '00000000-0000-4000-8000-000000000001',
  state: 'Completed, ResponseLimitReached',
  responseCount: 1,
  fileCount: 1,
  endedAt: '2026-11-01T12:00:00Z',
};
// One peer with one song, the same for every query.
const RESPONSES = [
  {
    username: 'ui-peer',
    hasFreeUploadSlot: true,
    queueLength: 0,
    uploadSpeed: 2_000_000,
    files: [{ filename: '@@ui\\Music\\Quiet Harbour\\Tidewater (2019)\\01 - Low Tide.mp3', size: 9_600_000, bitRate: 320, length: 240 }],
  },
];

const routes = {
  'GET /application': () => ({
    version: { current: '0.26.2' },
    server: { username: 'ui-test', isLoggedIn: true, isTransitioning: false, state: 'Connected, LoggedIn' },
    user: { username: 'ui-test' },
  }),
  'GET /options': () => ({ directories: { incomplete: '/transfers/incomplete', downloads: '/transfers/complete' } }),
  'GET /shares': () => ({ local: [{ id: 'music', isExcluded: false, directories: 4, files: 14 }] }),
  'PUT /shares': () => null,
  'GET /logs/live': () => [],
  'GET /events': () => [],
  'POST /searches': () => SEARCH,
  'GET /searches/*': (parts) => (parts[2] === 'responses' ? RESPONSES : SEARCH),
  'PUT /searches/*': () => null,
  'DELETE /searches/*': () => null,
  'GET /transfers/downloads/*': () => ({ username: '', directories: [] }),
};

createServer((req, res) => {
  const path = new URL(req.url, 'http://stub').pathname.replace(/^\/api\/v0/, '');
  const parts = path.split('/').filter(Boolean);
  const handler = routes[`${req.method} ${path}`] ?? routes[`${req.method} /${parts[0]}/*`];
  req.resume();
  if (req.headers['x-api-key'] !== key) return send(res, 401, { error: 'bad api key' });
  if (!handler) {
    console.error(`stub slskd: unhandled ${req.method} ${path}`);
    return send(res, 404, { error: `stub slskd has no ${req.method} ${path}` });
  }
  const body = handler(parts);
  return body === null ? send(res, 204) : send(res, 200, body);
}).listen(port, '127.0.0.1', () => console.log(`stub slskd on 127.0.0.1:${port}`));

function send(res, status, body) {
  res.writeHead(status, { 'content-type': 'application/json' });
  res.end(body === undefined ? undefined : JSON.stringify(body));
}
