#!/usr/bin/env node
// The slskd contract: what lull relies on slskd for, against the slskd that ships.
//
// - Seeking ahead (ADR-0014, "A contract test guards the resume"). lull sizes slskd's incomplete file to X before
//   it enqueues a download. That works only because slskd 0.26.x, before each attempt, reads that file's length
//   and, with `transfers.download.retry.partial: resume` (the default), asks the peer to start at that length and
//   appends (DownloadService.cs, Retry.Do around Client.DownloadAsync).
// - Streaming (ADR-0062). lull reads slskd's incomplete file while slskd writes it, and keeps it open while slskd
//   moves it to its downloads folder. Windows enforces how slskd opens the file: stock slskd forbids every other
//   reader there, so lull ships its own build for Windows (app/slskd/).
//
// It runs slskd against a stand-in Soulseek server and peer and checks:
//   resume     the default settings, incomplete file pre-sized to X: the peer is asked for X, and the finished file
//              is the pre-fill up to X and the peer's bytes from X on;
//   overwrite  `retry.partial: overwrite`, same setup: the peer is asked for 0 and the file is the peer's whole file;
//   stream     the peer sends slowly; while slskd downloads, the incomplete file is opened and read the way lull
//              reads it (Node's open, like Rust's, shares read, write and delete), and the bytes so far are the
//              peer's; the file stays open while slskd finishes and moves it, then reads to the end as the peer's.
// Exits non-zero with one sentence saying what differed.
//
// Usage:
//   node tests/slskd-contract/run.mjs            the image deploy/compose.yaml pins (needs Docker)
//   node tests/slskd-contract/run.mjs --native   the binary app/build.rs pins for this machine, as the desktop app
//                                                bundles it (Windows)
// CONTRACT_TRACE=1 prints the protocol messages.

import { spawn, spawnSync } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:net';
import { access, mkdir, mkdtemp, open, readFile, readdir, realpath, rm, stat, writeFile } from 'node:fs/promises';
import { arch, tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { startServer } from './server.mjs';
import { startPeer } from './peer.mjs';

const NATIVE = process.argv.includes('--native');

const ME = 'contract-lull'; // slskd's login on the stand-in server
const PEER = 'contract-peer';
// A SoulseekQt-style share root (@@xxxxx) that slskd strips from the incomplete path, then ordinary folders.
const REMOTE = '@@lullc\\Music\\Contract Artist\\Contract Album\\01 - Seek Ahead.mp3';
const STREAMED = '@@lullc\\Music\\Contract Artist\\Contract Album\\02 - Play As It Arrives.mp3';
const MiB = 1024 * 1024;
const SIZE = 3 * MiB + 12345; // not a round number, so an off-by-a-block offset shows
const X = 1 * MiB; // where lull would seek to
const PREFILL = 0xab;
const READ_AT = 256 * 1024; // how much of the streamed file has arrived when it is first read

class ContractError extends Error {}

const CASES = [
  {
    name: 'resume',
    remote: REMOTE,
    yml: null, // slskd's default (resume) is what lull relies on, so the case sets nothing
    presize: X,
    expectOffset: X,
    expected: (file) => Buffer.concat([Buffer.alloc(X, PREFILL), file.subarray(X)]),
    meaning: 'slskd no longer resumes from the incomplete file\'s length, so lull cannot seek ahead this way',
  },
  {
    name: 'overwrite',
    remote: REMOTE,
    yml: 'transfers:\n  download:\n    retry:\n      partial: overwrite\n',
    presize: X,
    expectOffset: 0,
    expected: (file) => file,
    meaning: 'with retry.partial: overwrite slskd should start again from 0, the signal lull detects',
  },
  {
    name: 'stream',
    remote: STREAMED,
    yml: null,
    presize: 0,
    expectOffset: 0,
    expected: (file) => file,
    meaning: 'lull cannot play a song while slskd downloads it',
    readWhileDownloading: true,
  },
];

const cleanups = new Set(); // what to stop if we are interrupted
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    for (const stop of cleanups) stop();
    process.exit(130);
  });
}

main().then(
  () => process.exit(0),
  (e) => {
    console.error(`\nFAIL: ${e instanceof ContractError ? e.message : e.stack}`);
    process.exit(1);
  },
);

async function main() {
  const slskd = NATIVE ? await nativeSlskd() : await dockerSlskd();
  console.log(`slskd contract: ${slskd.name}`);

  const file = deterministicBytes(SIZE);
  const listenPort = await freePort();
  const server = await startServer({ host: slskd.network.bind });
  const peer = await startPeer({
    username: PEER,
    host: slskd.network.bind,
    files: new Map([[REMOTE, file], [STREAMED, file]]),
    paced: new Set([STREAMED]),
    downloader: { host: '127.0.0.1', port: listenPort }, // slskd's listen port: the host's own, or published there
  });
  server.setPeer(PEER, slskd.network.peerIp, peer.port);

  try {
    for (const c of CASES) await runCase(c, { slskd, server, peer, file, listenPort });
  } finally {
    server.close();
    peer.close();
  }
  console.log(`\nPASS: slskd resumes a download from a pre-sized incomplete file (asked for ${X}), starts again from 0 with retry.partial: overwrite, and lets its file be read while it downloads and moves it.`);
}

async function runCase(c, { slskd, server, peer, file, listenPort }) {
  const log = (msg) => console.log(`[${c.name}] ${msg}`);
  // Its long name: slskd refuses a file whose path Path.GetFullPath changes, and on Windows that expands 8.3 names,
  // which the temp folder can have (C:\Users\RUNNER~1\... on GitHub's runners). The app's folders come from the
  // known-folder API, which gives long names.
  const work = await realpath(await mkdtemp(join(tmpdir(), 'slskd-contract-')));
  const httpPort = await freePort();
  const apiKey = randomBytes(16).toString('hex');
  const api = `http://127.0.0.1:${httpPort}`;
  let running = null;
  let reading = null;

  try {
    // slskd's folders where this process can pre-size the incomplete file and read the result.
    await mkdir(join(work, 'app'));
    await mkdir(join(work, 'transfers', 'downloads'), { recursive: true });
    if (c.yml) await writeFile(join(work, 'app', 'slskd.yml'), c.yml);

    const incomplete = join(work, 'transfers', 'incomplete', ...incompleteSegments(PEER, c.remote));
    await mkdir(join(incomplete, '..'), { recursive: true });
    if (c.presize) {
      await writeFile(incomplete, Buffer.alloc(c.presize, PREFILL));
      log(`pre-sized ${incomplete.slice(work.length)} to ${c.presize} bytes`);
    }

    const loggedIn = nextEvent(server.events, 'login', (u) => u === ME);
    const offsets = [];
    const onOffset = (e) => { if (e.filename === c.remote) offsets.push(e.offset); };
    peer.events.on('offset', onOffset);

    running = await slskd.start({
      work,
      httpPort,
      listenPort,
      env: {
        SLSKD_SLSK_ADDRESS: slskd.network.serverHost,
        SLSKD_SLSK_PORT: String(server.port),
        SLSKD_SLSK_USERNAME: ME,
        SLSKD_SLSK_PASSWORD: 'contract',
        SLSKD_SLSK_LISTEN_PORT: String(listenPort),
        SLSKD_HTTP_PORT: String(httpPort),
        SLSKD_NO_HTTPS: 'true',
        SLSKD_API_KEY: apiKey,
        SLSKD_NO_VERSION_CHECK: 'true',
        SLSKD_SLSK_NO_DNET: 'true',
      },
    });
    const step = (what, ms, promise) => withDeadline(what, ms, promise, running.exited);

    await step('slskd to log in to the stand-in server', 120_000, loggedIn);
    log('slskd logged in');
    await step('slskd\'s API', 60_000, until(async () => {
      const r = await fetch(`${api}/api/v0/application`, { headers: { 'X-API-Key': apiKey } }).catch(() => null);
      return r?.ok;
    }));

    const served = peer.nextServed(c.remote);
    const res = await fetch(`${api}/api/v0/transfers/downloads/${encodeURIComponent(PEER)}`, {
      method: 'POST',
      headers: { 'X-API-Key': apiKey, 'Content-Type': 'application/json' },
      body: JSON.stringify([{ filename: c.remote, size: SIZE }]),
    });
    const body = await res.text();
    if (!res.ok) throw new ContractError(`slskd refused the enqueue (HTTP ${res.status}): ${body}`);
    if (JSON.parse(body || '{}').failed?.length) throw new ContractError(`slskd did not enqueue the file: ${body}`);
    log('enqueued');

    if (c.readWhileDownloading) {
      await step(`${READ_AT} bytes of the download in slskd's incomplete file`, 120_000, until(async () => (await stat(incomplete)).size >= READ_AT));
      reading = await openAsLullDoes(incomplete, c.meaning);
      const head = Buffer.alloc(READ_AT);
      const { bytesRead } = await reading.read(head, 0, READ_AT, 0);
      const t = await transferOf(api, apiKey, c.remote);
      if (t?.state?.includes('Completed')) {
        throw new ContractError(`${c.name}: slskd finished before the file was read, so this didn't read it while it downloaded.`);
      }
      if (bytesRead !== READ_AT || firstDifference(head, file.subarray(0, READ_AT)) !== -1) {
        throw new ContractError(`${c.name}: the first ${READ_AT} bytes read while slskd downloaded aren't the peer's (${bytesRead} read).`);
      }
      log(`read ${READ_AT} bytes of the incomplete file while slskd downloaded it (${t?.state})`);
    }

    const { offset, connection } = await step('the peer to send the file', 120_000, served);
    log(`peer was asked for offset ${offset} (transfer connection ${connection === 'outbound' ? 'opened by the peer' : 'opened by slskd'})`);

    const state = await step('slskd to finish the download', 60_000, until(async () => {
      const t = await transferOf(api, apiKey, c.remote);
      return t?.state?.includes('Completed') && t;
    }));
    if (!state.state.includes('Succeeded')) {
      throw new ContractError(`${c.name}: slskd ended the download as "${state.state}" (${state.exception ?? 'no reason given'})`);
    }

    // slskd moves the file into downloads/ after the transfer reports success, so wait for it to land.
    const downloads = join(work, 'transfers', 'downloads');
    const landed = await step('the finished file in the downloads folder', 30_000, until(async () => {
      const all = await readdir(downloads, { recursive: true });
      const hit = all.find((p) => basename(p) === basename(incomplete));
      return hit && join(downloads, hit);
    }));

    // The checks. Offset first: it is the cause, the file contents the effect.
    if (offsets.length !== 1) {
      throw new ContractError(`${c.name}: expected one transfer from the peer, saw ${offsets.length} (offsets ${offsets.join(', ')}).`);
    }
    if (offset !== c.expectOffset) {
      throw new ContractError(`${c.name}: the peer was asked for offset ${offset}, expected ${c.expectOffset}: ${c.meaning}.`);
    }
    const got = await readFile(landed);
    const want = c.expected(file);
    if (got.length !== want.length) {
      throw new ContractError(`${c.name}: the downloaded file is ${got.length} bytes, expected ${want.length}.`);
    }
    const at = firstDifference(got, want);
    if (at !== -1) {
      const where = at < X ? `in the first ${X} bytes` : `after byte ${X}`;
      throw new ContractError(`${c.name}: the downloaded file differs from what was expected at byte ${at} (${where}): ${c.meaning}.`);
    }
    if (reading) {
      // The file lull kept open through the move, read to its end: what lull plays after slskd lets go.
      const all = Buffer.alloc(want.length);
      const { bytesRead } = await reading.read(all, 0, all.length, 0);
      if (bytesRead !== want.length || firstDifference(all, want) !== -1) {
        throw new ContractError(`${c.name}: the file held open while slskd moved it doesn't read as the peer's file (${bytesRead} of ${want.length} bytes).`);
      }
      log(`read the whole file through the handle held open while slskd moved it`);
    }
    log(`ok: offset ${offset}, ${got.length} bytes as expected`);
    peer.events.off('offset', onOffset);
  } catch (e) {
    if (running) console.error(`--- last lines of slskd's log (${c.name}) ---\n${await running.logs()}---`);
    throw e;
  } finally {
    await reading?.close().catch(() => {});
    if (running) await running.stop();
    await rm(work, { recursive: true, force: true }).catch(() => {});
  }
}

/** Opens slskd's incomplete file to read, failing with what it means for lull when slskd forbids that. */
async function openAsLullDoes(path, meaning) {
  try {
    return await open(path, 'r');
  } catch (e) {
    if (e.code === 'EBUSY' || e.code === 'EPERM') {
      throw new ContractError(`stream: slskd won't let another program read the file it is downloading (${e.code}, Windows' sharing violation, lull's "os error 32"): ${meaning}.`);
    }
    throw e;
  }
}

/**
 * Where slskd puts a download's incomplete file, relative to its incomplete directory:
 * <username>/<remote directories>/<filename>, each segment sanitized and the remote path's root stripped.
 * Mirrors FileSafety.SanitizePathSegment, GetDirectoryNameSafely and GetFileNameSafely in slskd 0.26.2; these names
 * are spelled the same on Linux and Windows.
 */
function incompleteSegments(username, remote) {
  const sanitize = (s) => {
    const out = s.replace(/[\0/\\]/g, '_');
    return out === '.' || out === '..' ? '_' : out;
  };
  const local = remote.replace(/\\/g, '/');
  const filename = sanitize(local.split('/').at(-1));
  const unrooted = local
    .replace(/\/+$/, '')
    .replace(/^[a-zA-Z]:[/\\]?/, '') // C:\
    .replace(/^[/\\]{2}[^/\\]+[/\\]?/, '') // \\server\
    .replace(/^@@[a-zA-Z0-9]{5,}[/\\]?/, ''); // SoulseekQt's @@abcde\
  const dirs = unrooted.split('/').filter((s) => s !== '').slice(0, -1).map(sanitize);
  return [sanitize(username), ...dirs, filename];
}

async function transferOf(api, apiKey, remote) {
  const r = await fetch(`${api}/api/v0/transfers/downloads/${encodeURIComponent(PEER)}`, { headers: { 'X-API-Key': apiKey } });
  if (!r.ok) return null;
  const user = await r.json();
  return user.directories?.flatMap((d) => d.files).find((f) => f.filename === remote) ?? null;
}

// --- the slskd under test ---

/**
 * The image the compose kit pins, read from it: one pin, and this tests the one that ships.
 * - Linux with a native engine (CI): slskd on the host's network; everyone is 127.0.0.1.
 * - Docker Desktop (Windows, macOS): slskd reaches this machine as host.docker.internal; its API and listen port
 *   are published on 127.0.0.1. The peer address handed to slskd must be an IP, so it is resolved in the image.
 */
async function dockerSlskd() {
  const image = (await readFile(new URL('../../deploy/compose.yaml', import.meta.url), 'utf8')).match(/image:\s*(slskd\/slskd:\S+)/)?.[1];
  if (!image) throw new Error('deploy/compose.yaml names no slskd/slskd image');
  await docker(['image', 'inspect', image]).catch(() => {
    console.log(`pulling ${image}`);
    return docker(['pull', image]);
  });

  const os = (await docker(['info', '--format', '{{.OperatingSystem}}'])).trim();
  const forced = process.env.CONTRACT_NETWORK; // 'host' or 'desktop', when the guess is wrong
  let network;
  let runArgs;
  if (forced === 'host' || (!forced && process.platform === 'linux' && !/Docker Desktop/i.test(os))) {
    network = { name: 'host', bind: '127.0.0.1', serverHost: '127.0.0.1', peerIp: '127.0.0.1' };
    // Run as this user, so the files slskd writes are ours to read and delete.
    runArgs = () => ['--network', 'host', '--user', `${process.getuid()}:${process.getgid()}`];
  } else {
    const resolved = await docker(['run', '--rm', '--entrypoint', 'getent', image, 'ahostsv4', 'host.docker.internal']);
    const peerIp = resolved.split(/\s+/)[0];
    if (!/^\d+\.\d+\.\d+\.\d+$/.test(peerIp)) throw new Error(`could not resolve host.docker.internal in the image: ${resolved}`);
    network = { name: `Docker Desktop (this machine is ${peerIp} to slskd)`, bind: '127.0.0.1', serverHost: 'host.docker.internal', peerIp };
    runArgs = (httpPort, listenPort) => ['-p', `127.0.0.1:${httpPort}:${httpPort}`, '-p', `127.0.0.1:${listenPort}:${listenPort}`];
  }

  return {
    name: `${image.split('@')[0]}, ${network.name} networking`,
    network,
    async start({ work, httpPort, listenPort, env }) {
      const name = `lull-slskd-contract-${process.pid}-${httpPort}`;
      const all = { ...env, SLSKD_INCOMPLETE_DIR: '/transfers/incomplete', SLSKD_DOWNLOADS_DIR: '/transfers/downloads' };
      await docker([
        'run', '-d', '--name', name,
        ...runArgs(httpPort, listenPort),
        '-v', `${join(work, 'app')}:/app`,
        '-v', `${join(work, 'transfers')}:/transfers`,
        ...Object.entries(all).flatMap(([k, v]) => ['-e', `${k}=${v}`]),
        image,
      ]);
      const stop = () => spawnSync('docker', ['rm', '-f', name], { stdio: 'ignore' });
      cleanups.add(stop);
      return {
        exited: docker(['wait', name]).then((code) => code.trim(), () => 'unknown'),
        logs: () => docker(['logs', '--tail', '60', name]).catch(() => ''),
        async stop() { cleanups.delete(stop); await docker(['rm', '-f', name]).catch(() => {}); },
      };
    },
  };
}

/**
 * The binary app/build.rs pins for this machine, downloaded and checked by its sha256 like build.rs does, and run
 * the way the desktop app runs it (app/src/desktop/slskd.rs): its folders on the command line, headless.
 */
async function nativeSlskd() {
  const target = { 'win32-x64': 'x86_64-pc-windows-msvc', 'win32-arm64': 'aarch64-pc-windows-msvc' }[`${process.platform}-${arch()}`];
  if (!target) throw new Error('--native runs the Windows app\'s slskd: run it on Windows (Docker covers Linux)');
  const buildRs = await readFile(new URL('../../app/build.rs', import.meta.url), 'utf8');
  const pin = [...buildRs.matchAll(/target: "([^"]+)",\s*url: "([^"]+)",\s*sha256: "([0-9a-f]{64})"/g)].find((m) => m[1] === target);
  if (!pin) throw new Error(`app/build.rs pins no slskd for ${target}`);
  const [, , url, sha256] = pin;

  const dir = join(tmpdir(), 'lull-slskd-contract', sha256);
  const exe = join(dir, 'slskd.exe');
  if (!(await access(exe).then(() => true, () => false))) {
    console.log(`downloading ${url}`);
    const r = await fetch(url);
    if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
    const zip = Buffer.from(await r.arrayBuffer());
    const got = createHash('sha256').update(zip).digest('hex');
    if (got !== sha256) throw new Error(`${url} has sha256 ${got}, not the pinned ${sha256}`);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'slskd.zip'), zip);
    // Windows' own tar reads zips (Git's, often first on PATH, doesn't).
    const untar = spawnSync(join(process.env.SystemRoot ?? 'C:\\Windows', 'System32', 'tar.exe'), ['-xf', join(dir, 'slskd.zip'), '-C', dir, 'slskd.exe'], { stdio: 'inherit' });
    if (untar.status !== 0) throw new Error('could not unpack slskd.exe');
  }

  return {
    name: `${basename(url)} (app/build.rs, ${target}), run natively`,
    network: { bind: '127.0.0.1', serverHost: '127.0.0.1', peerIp: '127.0.0.1' },
    async start({ work, httpPort, env }) {
      const child = spawn(exe, [
        '--app-dir', join(work, 'app'),
        '--http-ip-address', '127.0.0.1', '--http-port', String(httpPort),
        '--no-https', '--headless', '--no-version-check', '--no-logo', '--no-color',
        '--content-path', '.',
        '--downloads', join(work, 'transfers', 'downloads'),
        '--incomplete', join(work, 'transfers', 'incomplete'),
      ], { cwd: dirname(exe), env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
      const tail = [];
      const keep = (d) => { tail.push(...String(d).split('\n')); tail.splice(0, Math.max(0, tail.length - 60)); };
      child.stdout.on('data', keep);
      child.stderr.on('data', keep);
      const exited = new Promise((resolve) => child.once('exit', (code) => resolve(String(code))));
      const stop = () => child.kill();
      cleanups.add(stop);
      return {
        exited,
        logs: async () => tail.join('\n'),
        async stop() { cleanups.delete(stop); child.kill(); await exited; },
      };
    },
  };
}

// --- small helpers ---

/** Runs docker, resolving with stdout; rejects with stderr on a non-zero exit. */
function docker(args) {
  return new Promise((resolve, reject) => {
    const p = spawn('docker', args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    let err = '';
    p.stdout.on('data', (d) => { out += d; });
    p.stderr.on('data', (d) => { err += d; });
    p.on('error', reject);
    p.on('close', (code) => (code === 0 ? resolve(out) : reject(new Error(`docker ${args[0]} failed (${code}): ${err.trim()}`))));
  });
}

/** Awaits `promise`, failing with a sentence if it takes longer than `ms` or slskd exits first. */
function withDeadline(what, ms, promise, exited) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new ContractError(`timed out after ${ms / 1000} s waiting for ${what}.`)), ms);
    const settle = (fn) => (v) => { clearTimeout(timer); fn(v); };
    promise.then(settle(resolve), settle(reject));
    exited.then((code) => settle(reject)(new ContractError(`slskd exited (code ${code}) while waiting for ${what}.`)));
  });
}

/** Polls `check` until it returns something truthy, for state that has no event to await (HTTP, files). */
async function until(check, everyMs = 250) {
  for (;;) {
    const v = await check().catch(() => null);
    if (v) return v;
    await new Promise((r) => setTimeout(r, everyMs));
  }
}

function nextEvent(emitter, event, match) {
  return new Promise((resolve) => {
    const on = (v) => { if (match(v)) { emitter.off(event, on); resolve(v); } };
    emitter.on(event, on);
  });
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = createServer().once('error', reject).listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

/** The peer's file: xorshift32 bytes from a fixed seed, so every run serves the same content. */
function deterministicBytes(n) {
  const out = Buffer.alloc(n);
  let x = 0x1badf00d;
  for (let i = 0; i < n; i++) {
    x ^= x << 13; x ^= x >>> 17; x ^= x << 5;
    out[i] = x & 0xff;
  }
  return out;
}

function firstDifference(a, b) {
  if (a.equals(b)) return -1;
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return i;
  return a.length;
}
