// The fixture hub keeps the hub contract (tests/hub-contract/shapes.mjs): every read the contract checks on the real
// hub answers from the fixture hub too, in the same shapes, and with no field the contract doesn't name. The real hub
// is held to the same shapes by tests/hub-contract/run.mjs, so a field one hub renames fails on that hub's side, and the
// UI gates can't pass against an API the hub no longer serves.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { after, before, test } from 'node:test';
import { READS, check, libraryIds } from '../hub-contract/shapes.mjs';

const server = fileURLToPath(new URL('../ui/fixture-hub/server.mjs', import.meta.url));
let hub, base;
before(async () => {
  // its own port, and none of the switches or folders a gate may have set in this environment
  const env = { ...process.env };
  for (const k of Object.keys(env)) if (/^(FIXTURE_|LULL_)/.test(k)) delete env[k];
  hub = spawn(process.execPath, [server, '0'], { env, stdio: ['ignore', 'pipe', 'inherit'] });
  let out = '';
  hub.stdout.setEncoding('utf8');
  while (!(base = /fixture hub on (http:\S+)/.exec(out)?.[1])) {
    const [chunk] = await Promise.race([once(hub.stdout, 'data'), once(hub, 'exit').then(() => { throw new Error('the fixture hub exited'); })]);
    out += chunk;
  }
});
after(() => hub?.kill());

const get = async (path, headers = {}) => {
  const res = await fetch(base + path, { headers });
  return { status: res.status, json: await res.json() };
};

test('the fixture hub answers every read of the hub contract in its shapes, and with nothing the contract does not name', async () => {
  const have = await libraryIds(get);
  const failed = [];
  for (const read of READS) {
    assert.ok(!read.needs || have[read.needs], `the fixture library has a ${read.needs}`);
    const path = read.path(have), { status, json } = await get(path);
    if (status !== 200) { failed.push(`${path}: status ${status}`); continue; }
    const { problems, empty } = check(json, read.shape, { closed: true });
    failed.push(...problems.map((p) => `${path} ${p}`));
    // the fixture's library is full: every list of objects it answers with has something in it to check
    failed.push(...empty.map((p) => `${path} ${p}: empty, so nothing in it was checked`));
  }
  assert.deepEqual(failed, []);
});

test('the feeds the fixture hub serves a gate on asking (a first run, where picks play from, related tags) keep it too', async () => {
  const feed = READS.find((r) => r.path() === '/api/feed');
  for (const shape of ['cold', 'avail', 'related']) {
    const headers = { 'x-fixture-client': `contract-${shape}` };
    await fetch(base + '/__fixture/feed-shape', { method: 'POST', headers: { ...headers, 'content-type': 'application/json' }, body: JSON.stringify({ shape }) });
    const { json } = await get('/api/feed', headers);
    assert.deepEqual(check(json, feed.shape, { closed: true }).problems, [], shape);
  }
});

test('the shapes check what they say: a missing, mistyped or unnamed field fails, an empty list is named', () => {
  const shape = { a: 'string', b: 'number?', c: [{ d: 'boolean' }] };
  assert.deepEqual(check({ a: 'x', b: null, c: [{ d: true }] }, shape, { closed: true }), { problems: [], empty: [] });
  assert.deepEqual(check({ a: 1, c: [{ d: 'no' }], z: 0 }, shape, { closed: true }).problems,
    ['.a: number, not string', '.b: missing', '.c[0].d: string, not boolean', '.z: not in the contract']);
  assert.deepEqual(check({ a: 'x', b: 1, c: [], z: 0 }, shape, { closed: false }), { problems: [], empty: ['.c'] });
});
