// The next release's draft (ADR-0098): what landed since the last release, by area, which areas' batches are
// complete, the proposed version and the 500-character notes. Node only, no packages.
//   node scripts/release-notes.mjs draft [previous draft body file]   the draft's body, on stdout
//   node scripts/release-notes.mjs notes <draft body file>             the notes in a draft, as a person left them
//   node scripts/release-notes.mjs version                             the proposed version
// Run from the repository root, with the tags fetched. A person keeps two things in the draft from run to run: the
// batch's name on its first line, and the notes, once they edit them.
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';

const LIMIT = 500; // what F-Droid and IzzyOnDroid show (fastlane changelogs)
const git = (...args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
const die = (msg) => { console.error(`release-notes: ${msg}`); process.exit(1); };

function areas() {
  return readFileSync('scripts/release-areas.tsv', 'utf8').split('\n')
    .filter((l) => l.trim() && !l.startsWith('#'))
    .map((l) => { const [id, name, scopes] = l.split('\t'); return { id, name, scopes: scopes.trim().split(/\s+/) }; });
}

// The switches still dark, by area (lull_core::flags: each names its area).
function darkSwitches() {
  const src = readFileSync('crates/core/src/flags.rs', 'utf8').split('#[cfg(test)]')[0];
  const out = [];
  for (const m of src.matchAll(/Flag\s*\{[^}]*?name:\s*"([^"]+)"[^}]*?area:\s*"([^"]+)"[^}]*\}/gs)) out.push({ name: m[1], area: m[2] });
  return out;
}

function lastRelease() {
  const tag = git('tag', '-l', 'v*', '--sort=-v:refname').split('\n')[0];
  if (!tag) die('no v* tag: fetch the tags first');
  return tag;
}

function landed(since) {
  const all = areas();
  const byScope = new Map(all.flatMap((a) => a.scopes.map((s) => [s, a])));
  const groups = new Map(all.filter((a) => a.id !== '-').map((a) => [a.id, { ...a, feats: [], fixes: [] }]));
  for (const subject of git('log', '--format=%s', `${since}..HEAD`).split('\n')) {
    const m = subject.match(/^(feat|fix)\(([^)]+)\)!?: (.+)$/);
    if (!m) continue;
    const area = byScope.get(m[2]);
    if (!area) die(`the scope "${m[2]}" (${subject}) isn't in scripts/release-areas.tsv: add it to an area`);
    if (area.id === '-') continue;
    groups.get(area.id)[m[1] === 'feat' ? 'feats' : 'fixes'].push(m[3]);
  }
  const dark = darkSwitches();
  for (const g of groups.values()) g.waiting = dark.filter((d) => d.area === g.id).map((d) => d.name);
  return [...groups.values()].filter((g) => g.feats.length || g.fixes.length);
}

function proposedVersion(since, groups) {
  const [x, y, z] = since.replace(/^v/, '').split('.').map(Number);
  const ready = groups.filter((g) => !g.waiting.length);
  return ready.some((g) => g.feats.length) ? `${x}.${y + 1}.0` : `${x}.${y}.${z + 1}`;
}

function draftNotes(batch, groups) {
  const lines = batch ? [batch.endsWith('.') ? batch : `${batch}.`] : [];
  for (const g of groups.filter((g) => !g.waiting.length)) for (const s of [...g.feats, ...g.fixes]) lines.push(`* ${/^lull\b/.test(s) ? s : s[0].toUpperCase() + s.slice(1)}`); // lull keeps its lower case
  const out = [];
  let n = 0;
  for (const l of lines) { if (n + l.length + 1 > LIMIT) break; out.push(l); n += l.length + 1; }
  return out.join('\n') || 'Smaller fixes.';
}

// The parts of a draft a person may change.
const BATCH = /^Batch:[ \t]*(.*)$/m;
const NOTES = /<!-- notes -->\n```\n([\s\S]*?)\n```\n<!-- drafted: (\d+) -->/;
const hash = (s) => [...s].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);

function kept(prev) {
  const batch = (prev.match(BATCH)?.[1] || '').trim();
  const m = prev.match(NOTES);
  // Notes a person edited differ from what was last drafted; those stay.
  const edited = m && String(hash(m[1])) !== m[2] ? m[1] : null;
  return { batch, edited };
}

function draft(prevFile) {
  const since = lastRelease();
  const groups = landed(since);
  const { batch, edited } = kept(prevFile ? readFileSync(prevFile, 'utf8') : '');
  const generated = draftNotes(batch, groups);
  const notes = edited ?? generated;
  if (notes.length > LIMIT) die(`the notes are ${notes.length} characters; the stores show ${LIMIT}`);
  const area = (g) => {
    const items = [...g.feats.map((s) => `- ${s}`), ...g.fixes.map((s) => `- fix: ${s}`)].join('\n');
    const state = g.waiting.length ? `waiting for ${g.waiting.join(', ')} (still switched off)` : 'complete';
    return `### ${g.name}: ${state}\n${items}`;
  };
  return [
    `Batch: ${batch}`,
    '',
    `<!-- Set the batch's name on the first line ("the new designs"); it leads the notes. Edit the notes below as`,
    `you like: both stay. The rest is rewritten after every landing (scripts/release-notes.mjs, ADR-0098). -->`,
    '',
    `**Proposed: ${proposedVersion(since, groups)}** (since ${since}, at ${git('rev-parse', '--short', 'HEAD')})`,
    '',
    groups.length ? groups.map(area).join('\n\n') : 'Nothing for listeners since the last release.',
    '',
    `**What's new** (${notes.length} of ${LIMIT} characters${edited ? ', as edited' : ''}):`,
    '<!-- notes -->',
    '```',
    notes,
    '```',
    `<!-- drafted: ${hash(generated)} -->`,
  ].join('\n');
}

const [cmd, file] = process.argv.slice(2);
if (cmd === 'draft') console.log(draft(file));
else if (cmd === 'notes') {
  const m = readFileSync(file, 'utf8').match(NOTES);
  if (!m) die(`${file} has no notes block`);
  console.log(m[1]);
} else if (cmd === 'version') console.log(proposedVersion(lastRelease(), landed(lastRelease())));
else die('usage: release-notes.mjs draft [previous] | notes <draft> | version');
