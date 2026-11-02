// Before the matrix: wait for the hub, claim it with the setup code once pairing exists (ADR-0002),
// keep the device cookie for every test, and note one album, artist, tag and playlist to visit.

import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const base = process.env.LULL_URL ?? 'http://127.0.0.1:8080';
const setupCode = process.env.LULL_SETUP_CODE ?? 'ui-test-setup';

export default async function globalSetup() {
  await waitForHub();
  const cookie = await claim();
  const auth = join(here, '.auth');
  await mkdir(auth, { recursive: true });
  const origin = new URL(base);
  await writeFile(
    join(auth, 'state.json'),
    JSON.stringify({
      cookies: cookie
        ? [{ name: cookie.name, value: cookie.value, domain: origin.hostname, path: '/', httpOnly: true, secure: false, sameSite: 'Lax', expires: -1 }]
        : [],
      origins: [],
    }),
  );
  await writeFile(join(auth, 'ids.json'), JSON.stringify(await findIds(cookie)));
  await writeFile(join(auth, 'cookie.txt'), cookie ? `${cookie.name}=${cookie.value}` : '');
}

async function waitForHub() {
  for (let i = 0; i < 60; i++) {
    try {
      if ((await fetch(`${base}/health`)).ok) return;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 1000));
  }
  throw new Error(`no hub answered ${base}/health within 60 s`);
}

// Pairing is not in every build: before it lands /api/setup is a 404 and the hub is open.
async function claim() {
  const setup = await fetch(`${base}/api/setup`);
  if (!setup.ok) return null;
  const { claimed } = await setup.json();
  if (claimed) throw new Error('the hub is already claimed; start it with an empty data dir');
  const res = await fetch(`${base}/api/setup/claim`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ code: setupCode, name: 'ui test' }),
  });
  if (!res.ok) throw new Error(`claiming the hub with LULL_SETUP_CODE answered ${res.status}`);
  const match = /(lull_device)=([^;]+)/.exec(res.headers.get('set-cookie') ?? '');
  if (!match) throw new Error('the claim set no lull_device cookie');
  return { name: match[1], value: match[2] };
}

async function findIds(cookie) {
  const get = async (path) => {
    const res = await fetch(`${base}${path}`, { headers: cookie ? { cookie: `${cookie.name}=${cookie.value}` } : {} });
    return res.ok ? res.json() : { items: [] };
  };
  const first = async (path) => (await get(path)).items[0];
  return {
    album: await first('/api/library/albums?limit=1'),
    artist: await first('/api/library/artists?limit=1'),
    tag: (await first('/api/library/genres?limit=1'))?.id,
    playlist: (await first('/api/playlists?limit=1'))?.id,
  };
}
