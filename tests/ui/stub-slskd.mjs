// A stand-in for slskd's REST API (v0), so the hub under test has a Soulseek that is logged in,
// shares its library and finds nothing: `node stub-slskd.mjs [port] [api key]`. The visual tests
// need a connected `soulseek` status and deterministic searches, never the real network.
// Only what crates/soulseek/src/slskd.rs calls is answered; anything else is a loud 404.

import { createServer } from 'node:http';

const port = Number(process.argv[2] ?? 5030);
const key = process.argv[3] ?? 'ui-test-key';
const SEARCH = { id: '00000000-0000-4000-8000-000000000001', state: 'Completed, TimedOut', responseCount: 0, fileCount: 0 };

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
  'GET /searches/*': (parts) => (parts[2] === 'responses' ? [] : SEARCH),
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
