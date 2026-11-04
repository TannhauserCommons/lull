#!/usr/bin/env node
// The slskd resume contract (ADR-0014, "A contract test guards the resume").
//
// lull seeks ahead by sizing slskd's incomplete file to X before it enqueues a download. That works only because
// slskd 0.26.x, before each attempt, reads that file's length and, with `transfers.download.retry.partial: resume`
// (the default), asks the peer to start at that length and appends (DownloadService.cs, Retry.Do around
// Client.DownloadAsync). This runs the pinned slskd image against a stand-in Soulseek server and peer and checks:
//   resume     the default settings, incomplete file pre-sized to X: the peer is asked for X, and the finished file
//              is the pre-fill up to X and the peer's bytes from X on;
//   overwrite  `retry.partial: overwrite`, same setup: the peer is asked for 0 and the file is the peer's whole file.
// Exits non-zero with one sentence saying what differed.
//
// Usage: node tests/slskd-contract/run.mjs   (needs Docker; CONTRACT_TRACE=1 prints the protocol messages)

import { spawn, spawnSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { randomBytes } from 'node:crypto';
import { startServer } from './server.mjs';
import { startPeer } from './peer.mjs';

// The image the compose kit pins, read from it: one pin, and this tests the one that ships.
const IMAGE = (await readFile(new URL('../../deploy/compose.yaml', import.meta.url), 'utf8')).match(/image:\s*(slskd\/slskd:\S+)/)?.[1];
if (!IMAGE) throw new Error('deploy/compose.yaml names no slskd/slskd image');

const ME = 'contract-lull'; // slskd's login on the stand-in server
const PEER = 'contract-peer';
// A SoulseekQt-style share root (@@xxxxx) that slskd strips from the incomplete path, then ordinary folders.
const REMOTE = '@@lullc\\Music\\Contract Artist\\Contract Album\\01 - Seek Ahead.mp3';
const MiB = 1024 * 1024;
const SIZE = 3 * MiB + 12345; // not a round number, so an off-by-a-block offset shows
const X = 1 * MiB; // where lull would seek to
const PREFILL = 0xab;

class ContractError extends Error {}

const CASES = [
  {
    name: 'resume',
    yml: null, // slskd's default (resume) is what lull relies on, so the case sets nothing
    expectOffset: X,
    expected: (file) => Buffer.concat([Buffer.alloc(X, PREFILL), file.subarray(X)]),
    meaning: 'slskd no longer resumes from the incomplete file\'s length, so lull cannot seek ahead this way',
  },
  {
    name: 'overwrite',
    yml: 'transfers:\n  download:\n    retry:\n      partial: overwrite\n',
    expectOffset: 0,
    expected: (file) => file,
    meaning: 'with retry.partial: overwrite slskd should start again from 0, the signal lull detects',
  },
];

const live = new Set(); // containers to remove if we are interrupted
for (const signal of ['SIGINT', 'SIGTERM']) {
  process.on(signal, () => {
    for (const name of live) spawnSync('docker', ['rm', '-f', name], { stdio: 'ignore' });
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
  const network = await networkMode();
  console.log(`slskd contract: ${IMAGE.split('@')[0]}, ${network.name} networking`);
  await ensureImage();

  const file = deterministicBytes(SIZE);
  const listenPort = await freePort();
  const server = await startServer({ host: network.bind });
  const peer = await startPeer({
    username: PEER,
    host: network.bind,
    files: new Map([[REMOTE, file]]),
    downloader: { host: '127.0.0.1', port: listenPort }, // slskd's listen port: the host's own, or published there
  });
  server.setPeer(PEER, network.peerIp, peer.port);

  try {
    for (const c of CASES) await runCase(c, { network, server, peer, file, listenPort });
  } finally {
    server.close();
    peer.close();
  }
  console.log(`\nPASS: slskd resumes a download from a pre-sized incomplete file (asked for ${X}), and starts again from 0 with retry.partial: overwrite.`);
}

async function runCase(c, { network, server, peer, file, listenPort }) {
  const log = (msg) => console.log(`[${c.name}] ${msg}`);
  const work = await mkdtemp(join(tmpdir(), 'slskd-contract-'));
  const name = `lull-slskd-contract-${process.pid}-${c.name}`;
  const httpPort = await freePort();
  const apiKey = randomBytes(16).toString('hex');
  const api = `http://127.0.0.1:${httpPort}`;

  try {
    // slskd's folders on a bind mount, so this process can pre-size the incomplete file and read the result.
    await mkdir(join(work, 'app'));
    await mkdir(join(work, 'transfers', 'downloads'), { recursive: true });
    if (c.yml) await writeFile(join(work, 'app', 'slskd.yml'), c.yml);

    const incomplete = join(work, 'transfers', 'incomplete', ...incompleteSegments(PEER, REMOTE));
    await mkdir(join(incomplete, '..'), { recursive: true });
    await writeFile(incomplete, Buffer.alloc(X, PREFILL));
    log(`pre-sized ${incomplete.slice(work.length)} to ${X} bytes`);

    const loggedIn = nextEvent(server.events, 'login', (u) => u === ME);
    const offsets = [];
    const onOffset = (e) => offsets.push(e.offset);
    peer.events.on('offset', onOffset);

    await docker([
      'run', '-d', '--name', name,
      ...network.runArgs(httpPort, listenPort),
      '-v', `${join(work, 'app')}:/app`,
      '-v', `${join(work, 'transfers')}:/transfers`,
      '-e', `SLSKD_SLSK_ADDRESS=${network.serverHost}`,
      '-e', `SLSKD_SLSK_PORT=${server.port}`,
      '-e', `SLSKD_SLSK_USERNAME=${ME}`,
      '-e', 'SLSKD_SLSK_PASSWORD=contract',
      '-e', `SLSKD_SLSK_LISTEN_PORT=${listenPort}`,
      '-e', `SLSKD_HTTP_PORT=${httpPort}`,
      '-e', 'SLSKD_NO_HTTPS=true',
      '-e', `SLSKD_API_KEY=${apiKey}`,
      '-e', 'SLSKD_INCOMPLETE_DIR=/transfers/incomplete',
      '-e', 'SLSKD_DOWNLOADS_DIR=/transfers/downloads',
      '-e', 'SLSKD_NO_VERSION_CHECK=true',
      '-e', 'SLSKD_SLSK_NO_DNET=true',
      IMAGE,
    ]);
    live.add(name);
    const exited = docker(['wait', name]).then((code) => code.trim(), () => 'unknown');
    const step = (what, ms, promise) => withDeadline(what, ms, promise, exited);

    await step('slskd to log in to the stand-in server', 120_000, loggedIn);
    log('slskd logged in');
    await step('slskd\'s API', 60_000, until(async () => {
      const r = await fetch(`${api}/api/v0/application`, { headers: { 'X-API-Key': apiKey } }).catch(() => null);
      return r?.ok;
    }));

    const served = peer.nextServed(REMOTE);
    const res = await fetch(`${api}/api/v0/transfers/downloads/${encodeURIComponent(PEER)}`, {
      method: 'POST',
      headers: { 'X-API-Key': apiKey, 'Content-Type': 'application/json' },
      body: JSON.stringify([{ filename: REMOTE, size: SIZE }]),
    });
    const body = await res.text();
    if (!res.ok) throw new ContractError(`slskd refused the enqueue (HTTP ${res.status}): ${body}`);
    if (JSON.parse(body || '{}').failed?.length) throw new ContractError(`slskd did not enqueue the file: ${body}`);
    log('enqueued');

    const { offset, connection } = await step('the peer to send the file', 120_000, served);
    log(`peer was asked for offset ${offset} (transfer connection ${connection === 'outbound' ? 'opened by the peer' : 'opened by slskd'})`);

    const state = await step('slskd to finish the download', 60_000, until(async () => {
      const t = await transferOf(api, apiKey);
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
    log(`ok: offset ${offset}, ${got.length} bytes as expected`);
    peer.events.off('offset', onOffset);
  } catch (e) {
    if (live.has(name)) {
      const logs = await docker(['logs', '--tail', '60', name]).catch(() => '');
      console.error(`--- last lines of slskd's log (${c.name}) ---\n${logs}---`);
    }
    throw e;
  } finally {
    if (live.has(name)) await docker(['rm', '-f', name]).catch(() => {});
    live.delete(name);
    await rm(work, { recursive: true, force: true }).catch(() => {});
  }
}

/**
 * Where slskd puts a download's incomplete file, relative to its incomplete directory, on Linux (the container):
 * <username>/<remote directories>/<filename>, each segment sanitized and the remote path's root stripped.
 * Mirrors FileSafety.SanitizePathSegment, GetDirectoryNameSafely and GetFileNameSafely in slskd 0.26.2.
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

async function transferOf(api, apiKey) {
  const r = await fetch(`${api}/api/v0/transfers/downloads/${encodeURIComponent(PEER)}`, { headers: { 'X-API-Key': apiKey } });
  if (!r.ok) return null;
  const user = await r.json();
  return user.directories?.flatMap((d) => d.files).find((f) => f.filename === REMOTE) ?? null;
}

/**
 * How slskd in a container and the stand-ins on this machine reach each other.
 * - Linux with a native engine (CI): slskd on the host's network; everyone is 127.0.0.1.
 * - Docker Desktop (Windows, macOS): slskd reaches this machine as host.docker.internal; its API and listen port
 *   are published on 127.0.0.1. The peer address handed to slskd must be an IP, so it is resolved in the image.
 */
async function networkMode() {
  const os = (await docker(['info', '--format', '{{.OperatingSystem}}'])).trim();
  const forced = process.env.CONTRACT_NETWORK; // 'host' or 'desktop', when the guess is wrong
  if (forced === 'host' || (!forced && process.platform === 'linux' && !/Docker Desktop/i.test(os))) {
    return {
      name: 'host',
      bind: '127.0.0.1',
      serverHost: '127.0.0.1',
      peerIp: '127.0.0.1',
      // Run as this user, so the files slskd writes are ours to read and delete.
      runArgs: () => ['--network', 'host', '--user', `${process.getuid()}:${process.getgid()}`],
    };
  }
  await ensureImage();
  const resolved = await docker(['run', '--rm', '--entrypoint', 'getent', IMAGE, 'ahostsv4', 'host.docker.internal']);
  const peerIp = resolved.split(/\s+/)[0];
  if (!/^\d+\.\d+\.\d+\.\d+$/.test(peerIp)) throw new Error(`could not resolve host.docker.internal in the image: ${resolved}`);
  return {
    name: `Docker Desktop (this machine is ${peerIp} to slskd)`,
    bind: '127.0.0.1',
    serverHost: 'host.docker.internal',
    peerIp,
    runArgs: (httpPort, listenPort) => ['-p', `127.0.0.1:${httpPort}:${httpPort}`, '-p', `127.0.0.1:${listenPort}:${listenPort}`],
  };
}

let imageReady = null;
function ensureImage() {
  imageReady ??= docker(['image', 'inspect', IMAGE]).catch(() => {
    console.log(`pulling ${IMAGE}`);
    return docker(['pull', IMAGE]);
  });
  return imageReady;
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

/** Awaits `promise`, failing with a sentence if it takes longer than `ms` or the container exits first. */
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
