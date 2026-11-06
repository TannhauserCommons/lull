#!/usr/bin/env node
// Renders lull's built-in packs (ADR-0081) from their source in packs/ into web/packs/<id>/: their sounds, and
// pack.json with lull's own easter eggs (eggs.mjs) declared the way any pack declares them. A design's pack may also
// declare eggs of its own (`export const eggs = [...]` in packs/<id>.mjs, the same declarations), which only it has.
//
//   node scripts/sounds/render.mjs            render every pack: pack.json and sounds/*.ogg (needs ffmpeg with libopus)
//   node scripts/sounds/render.mjs --check    fail when a pack's source no longer renders the samples that were
//                                             encoded (fingerprints.json), or a pack breaks the format's limits
//   node scripts/sounds/render.mjs --sampler <dir>   one MP3 per pack, every sound in turn, to listen to them all
//
// The synthesis is seeded, so a pack renders the same samples on every machine; fingerprints.json holds a hash of
// each sound's 16-bit samples, which --check (part of `node --test tests/web`) compares without needing ffmpeg.

import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as d from './dsp.mjs';
import { EGGS } from './eggs.mjs';

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, '..', '..');
const OUT = join(ROOT, 'web', 'packs');
const VOCAB = JSON.parse(readFileSync(join(OUT, 'vocabulary.json'), 'utf8'));
const FINGERPRINTS = join(HERE, 'fingerprints.json');
export const PACKS = ['default', 'lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
/** `LULL_PACKS=petal,lull` renders only those, while working on them. */
const ONLY = process.env.LULL_PACKS ? process.env.LULL_PACKS.split(',') : PACKS;

const EGG_NAMES = new Set(EGGS.map((e) => e.name));

/** Renders one pack's sounds: name -> { pcm: Int16Array, loop, egg }. */
export async function renderPack(id) {
  const mod = await import(new URL(`./packs/${id}.mjs`, import.meta.url));
  const rng = d.seeded(seedOf(id));
  const { sounds, eggs } = mod.sounds(rng);
  // the pack's own eggs: names of their own, never one of lull's (eggs.mjs), so each is that design's alone
  const own = mod.eggs || [], names = new Set(EGG_NAMES);
  for (const e of own) {
    if (names.has(e.name)) throw new Error(`${id}: egg ${e.name} is declared twice (or is one of lull's own)`);
    names.add(e.name);
  }
  const out = {}, L = VOCAB.limits;
  const add = (name, value, egg) => {
    if (egg ? !names.has(name) : !VOCAB.events.includes(name)) throw new Error(`${id}/${name} is no ${egg ? 'egg eggs.mjs or the pack declares' : 'sound event'}`);
    const loop = !(value instanceof Float32Array);
    let buf = loop ? value.loop : value;
    // below 110 Hz a phone or laptop speaker plays nothing, and it would only take headroom from what it plays
    buf = d.biquad(d.biquad(buf.slice(), 'highpass', 110), 'highpass', 110);
    buf = loop ? d.peakTo(buf, -3) : d.peakTo(d.trim(buf), -3);
    const sec = buf.length / d.RATE, max = loop ? L.loopSeconds : egg || name === 'boot' ? L.longSeconds : L.cueSeconds;
    if (loop && !(egg || VOCAB.loops.includes(name))) throw new Error(`${id}/${name} may not loop`);
    if (sec > max) throw new Error(`${id}/${name} is ${sec.toFixed(2)} s; at most ${max} s`);
    out[name] = { pcm: toInt16(buf), loop, egg };
  };
  // a pack's module gives each sound under its event's or its egg's name
  for (const [name, v] of Object.entries({ ...sounds, ...eggs })) add(name, v, names.has(name));
  return { meta: mod.meta, sounds: out, own };
}

function seedOf(id) { let h = 2166136261; for (const c of id) h = Math.imul(h ^ c.charCodeAt(0), 16777619); return h >>> 0; }

function toInt16(buf) {
  const pcm = new Int16Array(buf.length);
  for (let i = 0; i < buf.length; i++) pcm[i] = Math.max(-32768, Math.min(32767, Math.round(buf[i] * 32767)));
  return pcm;
}

const fingerprint = (pcm) => createHash('sha256').update(Buffer.from(pcm.buffer, pcm.byteOffset, pcm.byteLength)).digest('hex').slice(0, 16);

function wav(pcm) {
  const data = Buffer.from(pcm.buffer, pcm.byteOffset, pcm.byteLength), h = Buffer.alloc(44);
  h.write('RIFF', 0); h.writeUInt32LE(36 + data.length, 4); h.write('WAVE', 8); h.write('fmt ', 12);
  h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22); h.writeUInt32LE(d.RATE, 24);
  h.writeUInt32LE(d.RATE * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34); h.write('data', 36);
  h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

function ffmpeg(args) { execFileSync('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-y', ...args], { stdio: 'inherit' }); }

/** pack.json: the pack's sounds, and the eggs it declares: all of lull's own in `default`, and in a design's pack
    the ones it has a sound or a slot (slots/egg.<name>.html) for, so its sound and its slot win over default's
    (ADR-0081: the higher pack's egg of a name wins), then the design's own eggs. */
function manifest(meta, sounds, own = []) {
  const entry = (name, s) => (s.loop ? { file: `sounds/${name}.ogg`, loop: true } : `sounds/${name}.ogg`);
  const events = Object.fromEntries(VOCAB.events.filter((e) => sounds[e]).map((e) => [e, entry(e, sounds[e])]));
  const slotted = (e) => existsSync(join(OUT, meta.id, 'slots', `egg.${e.name}.html`));
  const eggs = EGGS.filter((e) => meta.id === 'default' || sounds[e.name] || slotted(e))
    .concat(own)
    .map((e) => (sounds[e.name] ? { ...e, sound: entry(e.name, sounds[e.name]) } : e));
  return { id: meta.id, name: meta.name, author: 'lull', api: VOCAB.api, description: meta.description, sounds: events, eggs };
}

async function renderAll() {
  const prints = existingPrints(), tmp = mkdtempSync(join(tmpdir(), 'lull-sounds-'));
  try {
    for (const id of ONLY) {
      const { meta, sounds, own } = await renderPack(id);
      const dir = join(OUT, id, 'sounds');
      // only what this script writes: a pack's pack.css and slots are written by hand
      rmSync(dir, { recursive: true, force: true });
      mkdirSync(dir, { recursive: true });
      prints[id] = {};
      for (const [name, s] of Object.entries(sounds)) {
        const src = join(tmp, `${id}-${name}.wav`);
        writeFileSync(src, wav(s.pcm));
        // Opus at 48 kHz mono; no metadata and bit-exact muxing, so the same samples give the same file
        ffmpeg(['-i', src, '-c:a', 'libopus', '-b:a', s.loop ? '24k' : '32k', '-ac', '1', '-application', 'audio',
          '-map_metadata', '-1', '-fflags', '+bitexact', '-flags:a', '+bitexact', join(dir, `${name}.ogg`)]);
        prints[id][name] = fingerprint(s.pcm);
      }
      writeFileSync(join(OUT, id, 'pack.json'), JSON.stringify(manifest(meta, sounds, own), null, 2) + '\n');
      console.log(`${id}: ${Object.keys(sounds).length} sounds, ${(dirBytes(dir) / 1024).toFixed(0)} KB`);
    }
  } finally { rmSync(tmp, { recursive: true, force: true }); }
  writeFileSync(FINGERPRINTS, JSON.stringify(prints, null, 2) + '\n');
  writeFileSync(join(OUT, 'index.json'), index());
}

/** web/packs/index.json: the built-in packs the page knows, which carry a hand-written pack.css, and the eggs that
    have a slot (slots/egg.<name>.html), since a built-in pack is static files and no hub lists them */
function index() {
  const slots = (id) => { try { return readdirSync(join(OUT, id, 'slots')).filter((f) => /^egg\..+\.html$/.test(f)).map((f) => f.slice(4, -5)).sort(); } catch { return []; } };
  return JSON.stringify({ packs: PACKS.map((id) => ({ id, css: existsSync(join(OUT, id, 'pack.css')), ...(slots(id).length ? { slots: slots(id) } : {}) })) }, null, 2) + '\n';
}

function existingPrints() { try { return JSON.parse(readFileSync(FINGERPRINTS, 'utf8')); } catch { return {}; } }

function dirBytes(dir) { return readdirSync(dir).reduce((s, f) => s + readFileSync(join(dir, f)).length, 0); }

/** Returns the problems: a pack whose source renders other samples than were encoded, or whose files differ. */
export async function check() {
  const want = JSON.parse(readFileSync(FINGERPRINTS, 'utf8')), problems = [];
  for (const id of PACKS) {
    const { meta, sounds, own } = await renderPack(id);
    for (const [name, s] of Object.entries(sounds)) {
      if (!want[id] || want[id][name] !== fingerprint(s.pcm)) problems.push(`${id}/${name} changed since it was rendered`);
    }
    for (const name of Object.keys(want[id] || {})) if (!sounds[name]) problems.push(`${id}/${name} is no longer made`);
    const json = readFileSync(join(OUT, id, 'pack.json'), 'utf8');
    if (json !== JSON.stringify(manifest(meta, sounds, own), null, 2) + '\n') problems.push(`${id}/pack.json is stale`);
  }
  if (readFileSync(join(OUT, 'index.json'), 'utf8') !== index()) problems.push('web/packs/index.json is stale');
  return problems;
}

/** One MP3 per pack: each sound with a short gap, each loop played for 6 s, in the order of the manifest. */
async function sampler(dir) {
  mkdirSync(dir, { recursive: true });
  const tmp = mkdtempSync(join(tmpdir(), 'lull-sampler-'));
  try {
    for (const id of ONLY) {
      const { sounds } = await renderPack(id), parts = [], names = [];
      for (const [name, s] of Object.entries(sounds)) {
        let pcm = s.pcm;
        if (s.loop) { const len = d.n(6), r = new Int16Array(len); for (let i = 0; i < len; i++) r[i] = pcm[i % pcm.length]; pcm = r; }
        names.push(name);
        parts.push(pcm, new Int16Array(d.n(0.8)));
      }
      const all = new Int16Array(parts.reduce((s, p) => s + p.length, 0));
      let o = 0; for (const p of parts) { all.set(p, o); o += p.length; }
      const src = join(tmp, `${id}.wav`);
      writeFileSync(src, wav(all));
      ffmpeg(['-i', src, '-c:a', 'libmp3lame', '-q:a', '2', join(dir, `${id}-sampler.mp3`)]);
      writeFileSync(join(dir, `${id}-sampler.txt`), names.join('\n') + '\n');
    }
  } finally { rmSync(tmp, { recursive: true, force: true }); }
}

const arg = process.argv[2];
if (process.argv[1] && process.argv[1].endsWith('render.mjs')) {
  if (arg === '--check') {
    const problems = await check();
    if (problems.length) { console.error(problems.join('\n') + '\nRun: node scripts/sounds/render.mjs'); process.exit(1); }
    console.log('every built-in sound pack is current');
  } else if (arg === '--sampler') await sampler(process.argv[3] || 'samplers');
  else await renderAll();
}
