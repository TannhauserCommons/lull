// The desktop app as its own hub ("This computer is my hub") signs in to Soulseek with the helper copies it chose
// (ADR-0064), against the slskd contract's stand-in Soulseek server (tests/slskd-contract/server.mjs): no real account,
// nothing on the real network. Run it where the app is installed (the AUR check: this computer's yt-dlp, Deno and slskd)
// or built (the bundled copies), and say which it must have chosen.
//
//   node tests/desktop/hub-signin.mjs <lull binary> <identifier> <system|own|any>
//
// The app starts as its own hub, slskd beside it is pointed at the stand-in (SLSKD_SLSK_ADDRESS/PORT in the app's
// environment, which slskd inherits), the account goes in as Settings sends it (both keys in one PATCH /api/config,
// with the app's own device token), and the hub must say "Connected as contract-lull". hub.log's helpers line must
// name each helper's copy as expected.

import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { startServer } from '../slskd-contract/server.mjs';

const [binary, identifier, expect] = process.argv.slice(2);
if (!binary || !identifier || !['system', 'own', 'any'].includes(expect)) {
  throw new Error('usage: node tests/desktop/hub-signin.mjs <lull binary> <identifier> <system|own|any>');
}
const ME = 'contract-lull';
const CONFIG = join(process.env.XDG_CONFIG_HOME || join(homedir(), '.config'), identifier);
const DATA = join(process.env.XDG_DATA_HOME || join(homedir(), '.local', 'share'), identifier);
const HUB = 'http://127.0.0.1:8484';
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));
async function until(what, fn, ms) {
  const end = Date.now() + ms;
  let last;
  for (;;) {
    try { last = await fn(); if (last) return last; } catch (e) { last = e; }
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}${last instanceof Error ? ': ' + last.message : last ? ': ' + JSON.stringify(last) : ''}`);
    await sleep(500);
  }
}
const logs = () => {
  for (const f of [join(DATA, 'logs', 'hub.log')]) if (existsSync(f)) console.log(`--- ${f}\n${readFileSync(f, 'utf8').split('\n').slice(-60).join('\n')}`);
};

rmSync(CONFIG, { recursive: true, force: true });
rmSync(DATA, { recursive: true, force: true });
mkdirSync(CONFIG, { recursive: true });
writeFileSync(join(CONFIG, 'settings.json'), JSON.stringify({ hub: { mode: 'local' } }));

const server = await startServer({ host: '127.0.0.1' });
let loggedIn = null;
server.events.on('login', (user) => { loggedIn = user; });
const app = spawn(binary, [], {
  stdio: ['ignore', 'inherit', 'inherit'],
  env: { ...process.env, SLSKD_SLSK_ADDRESS: '127.0.0.1', SLSKD_SLSK_PORT: String(server.port), SLSKD_SLSK_NO_DNET: 'true', SLSKD_NO_VERSION_CHECK: 'true' },
});
let failed = false;
try {
  await until('the hub on this computer', async () => (await fetch(HUB + '/health')).ok, 90000);
  const token = await until('the app\'s own device token', () => JSON.parse(readFileSync(join(CONFIG, 'settings.json'), 'utf8')).token, 30000);
  // as Settings › Sources sends it: both keys in one request
  const r = await fetch(HUB + '/api/config', {
    method: 'PATCH', headers: { 'content-type': 'application/json', authorization: `Bearer ${token}` },
    body: JSON.stringify({ LULL_SOULSEEK_USERNAME: ME, LULL_SOULSEEK_PASSWORD: 'contract-password' }),
  });
  if (!r.ok) throw new Error(`the hub refused the account: ${r.status} ${await r.text()}`);
  const detail = await until('slskd signed in, as the hub says', async () => {
    const s = (await (await fetch(HUB + '/health')).json()).services.soulseek;
    return s && s.detail.startsWith(`Connected as ${ME}`) && s.detail;
  }, 120000);
  console.log(`soulseek: ${detail}; the stand-in server saw ${loggedIn} log in`);
  if (loggedIn !== ME) throw new Error(`the stand-in server saw no login as ${ME}`);

  const line = readFileSync(join(DATA, 'logs', 'hub.log'), 'utf8').split('\n').find((l) => l.includes('helpers: ')) || '';
  console.log(line.slice(line.indexOf('helpers: ')));
  if (expect !== 'any') {
    const whose = expect === 'system' ? "this computer's" : "lull's own";
    for (const program of ['yt-dlp', 'deno', 'slskd']) {
      const part = line.split(/helpers: |; /).find((p) => p.startsWith(program + ' ')) || '';
      if (!part.includes(`(${whose},`)) throw new Error(`${program} isn't ${whose} copy: ${part || 'not in the helpers line'}`);
    }
  }
  console.log(`ok: as its own hub, lull signed in to Soulseek with ${expect === 'any' ? 'the copies it chose' : expect === 'system' ? "this computer's helpers" : "lull's own helpers"}`);
} catch (e) {
  failed = true;
  console.log(`FAIL: ${e.message}`);
  logs();
} finally {
  app.kill();
  server.close();
}
process.exit(failed ? 1 : 0);
