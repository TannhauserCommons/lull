// The hub contract (ADR-0090): what any installed hub must do, checked from outside against a running one, so
// the Linux package's hub is held to the same test as the others. It touches nothing outside the hub: no
// Soulseek account, no YouTube search, no network the hub would reach out to.
//
//   node tests/hub-contract/run.mjs <hub url> --pair <code> [--managed] [--youtube] [--mdns]
// --pair    a pair code (`lull pair`): the contract pairs itself as an app, then unpairs at the end
// --managed the hub runs its own slskd (the app's and the package's): it must reach it
// --youtube the hub ships yt-dlp (the package and the image): it must run
// --mdns    the hub answers _lull._tcp on this network
import dgram from 'node:dgram';

const args = process.argv.slice(2);
const base = args[0]?.replace(/\/$/, '');
const flag = (name) => args.includes(`--${name}`);
const option = (name) => args[args.indexOf(`--${name}`) + 1];
if (!base || !flag('pair')) throw new Error('usage: run.mjs <hub url> --pair <code> [--managed] [--youtube] [--mdns]');

let failed = 0;
async function check(name, fn) {
  try {
    await fn();
    console.log(`ok    ${name}`);
  } catch (e) {
    failed++;
    console.log(`FAIL  ${name}: ${e.message}`);
  }
}
function expect(cond, what) {
  if (!cond) throw new Error(what);
}
let token = '';
let deviceId = '';
async function call(path, { method = 'GET', body, auth = true } = {}) {
  const headers = {};
  if (auth && token) headers.authorization = `Bearer ${token}`;
  if (body !== undefined) headers['content-type'] = 'application/json';
  const res = await fetch(base + path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  const text = await res.text();
  let json = null;
  try {
    json = JSON.parse(text);
  } catch {}
  return { status: res.status, type: res.headers.get('content-type') ?? '', text, json };
}
const until = async (what, fn, seconds = 90) => {
  const end = Date.now() + seconds * 1000;
  let last;
  while (Date.now() < end) {
    last = await fn();
    if (last.ok) return last;
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`${what} (last: ${last?.why})`);
};

await check('/health answers with the version and every part', async () => {
  const { status, json } = await call('/health', { auth: false });
  expect(status === 200, `status ${status}`);
  expect(typeof json?.version === 'string' && json.version, 'no version');
  for (const part of ['soulseek', 'library', 'cache', 'youtube', 'lastfm']) {
    expect(json.services?.[part]?.state, `no ${part} in services`);
  }
});

await check('the web UI is served', async () => {
  const page = await call('/', { auth: false });
  expect(page.status === 200 && page.type.startsWith('text/html'), `/ is ${page.status} ${page.type}`);
  expect(/<script[^>]+src=/.test(page.text), 'the page loads no script');
  const src = page.text.match(/<script[^>]+src="\/?([^"]+)"/)?.[1];
  const script = await call(`/${src}`, { auth: false });
  expect(script.status === 200, `/${src} is ${script.status}`);
});

await check('/api is closed without a paired device', async () => {
  const { status } = await call('/api/settings', { auth: false });
  expect(status === 401, `status ${status}`);
});

await check('/api/setup is open and says who runs slskd', async () => {
  const { status, json } = await call('/api/setup', { auth: false });
  expect(status === 200 && typeof json?.claimed === 'boolean', `status ${status}`);
  if (flag('managed')) expect(json.soulseekManaged === true, 'soulseekManaged is not true');
});

await check('a pair code pairs an app', async () => {
  const { status, json } = await call('/api/pair', {
    method: 'POST',
    auth: false,
    body: { code: option('pair'), name: 'hub contract', kind: 'app' },
  });
  expect(status === 200 && json?.token, `status ${status}: ${JSON.stringify(json)}`);
  token = json.token;
  deviceId = json.device?.id ?? '';
});

const reads = [
  '/api/settings',
  '/api/config',
  '/api/devices',
  '/api/library/source',
  '/api/playlists',
  '/api/likes',
  '/api/history',
  '/api/stats',
  '/api/transfers',
  '/api/themes',
  '/api/packs',
  '/api/youtube',
  '/api/lastfm',
];
for (const path of reads) {
  await check(`${path} answers a paired app`, async () => {
    const { status, json } = await call(path);
    expect(status === 200 && json !== null, `status ${status}`);
  });
}

await check('a setting changes through /api/config and the parts restart with it', async () => {
  const changed = await call('/api/config', { method: 'PATCH', body: { LULL_PREFERRED_FORMATS: 'flac,opus' } });
  expect(changed.status === 200, `PATCH is ${changed.status}: ${changed.text}`);
  await until('the change shows in /api/config', async () => {
    const { json } = await call('/api/config');
    const item = json?.items?.find((i) => i.key === 'LULL_PREFERRED_FORMATS');
    return { ok: item?.value === 'flac,opus' && item?.source === 'stored', why: JSON.stringify(item) };
  });
  const back = await call('/api/config', { method: 'PATCH', body: { LULL_PREFERRED_FORMATS: null } });
  expect(back.status === 200, `PATCH back is ${back.status}`);
});

await check('Subsonic answers at /rest', async () => {
  const { status, json } = await call('/rest/ping.view?f=json', { auth: false });
  expect(status === 200 && json?.['subsonic-response'], `status ${status}`);
});

if (flag('managed')) {
  await check('the hub reaches its own slskd', async () => {
    await until('soulseek says what slskd says, not that slskd is unreachable', async () => {
      const { json } = await call('/health', { auth: false });
      const detail = json?.services?.soulseek?.detail ?? '';
      const reached = /^(slskd is not connected to Soulseek|Connected|Reconnecting)/.test(detail);
      return { ok: reached, why: detail };
    });
  });
}

if (flag('youtube')) {
  // The version is read in the background each time the parts start, and the settings change above restarted them.
  await check('yt-dlp runs', async () => {
    await until('yt-dlp says its version', async () => {
      const { json } = await call('/api/youtube');
      return { ok: Boolean(json?.ytdlp?.version), why: JSON.stringify(json?.ytdlp) };
    }, 60);
  });
}

if (flag('mdns')) {
  await check('the hub answers _lull._tcp on this network', async () => {
    const answered = await mdnsQuery('_lull._tcp.local', 5000);
    expect(answered, 'no answer within 5 s');
  });
}

if (token) {
  await check('the contract unpairs itself', async () => {
    const { status } = await call(`/api/devices/${deviceId}`, { method: 'DELETE' });
    expect(status === 204, `status ${status}`);
  });
}

console.log(failed ? `\n${failed} failed` : '\nthe hub keeps the contract');
process.exit(failed ? 1 : 0);

/** One multicast PTR query; true when an answer names the service. */
function mdnsQuery(name, ms) {
  const labels = name.split('.').flatMap((l) => [Buffer.from([l.length]), Buffer.from(l)]);
  const query = Buffer.concat([
    Buffer.from([0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
    ...labels,
    Buffer.from([0, 0, 12, 0, 1]),
  ]);
  return new Promise((resolve) => {
    const socket = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    const done = (ok) => {
      clearTimeout(timer);
      socket.close();
      resolve(ok);
    };
    const timer = setTimeout(() => done(false), ms);
    socket.on('message', (msg) => {
      const answers = msg.readUInt16BE(6);
      if ((msg[2] & 0x80) && answers > 0 && msg.includes(Buffer.from('_lull'))) done(true);
    });
    socket.bind(5353, () => {
      socket.addMembership('224.0.0.251');
      socket.send(query, 5353, '224.0.0.251');
    });
  });
}
