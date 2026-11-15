// The verdict on shipped bytes: a route may grow over its merge-base by its allowance, and never past its ceiling.
// Both live in scripts/budgets/shipped-bytes.json, each with its reason. Pure functions; run.mjs measures and prints.

const n = new Intl.NumberFormat('en-US');
const bytes = (v) => `${n.format(v)} B`;
const signed = (v) => `${v < 0 ? '-' : '+'}${n.format(Math.abs(v))}`;

// Fails loudly on a budgets file that is not what the check reads: every limit a whole number of bytes with a reason.
export function validateLimits(limits) {
  const entry = (what, e) => {
    if (!Number.isInteger(e?.bytes) || e.bytes < 0) throw new Error(`${what}: bytes must be a whole number of bytes`);
    if (typeof e.why !== 'string' || !e.why.trim()) throw new Error(`${what}: a limit comes with its reason (why)`);
  };
  entry('allowance', limits.allowance);
  const routes = Object.entries(limits.routes ?? {});
  if (!routes.length) throw new Error('the budgets file has no routes');
  for (const [route, r] of routes) {
    entry(`${route} ceiling`, r.ceiling);
    if (r.allowance) entry(`${route} allowance`, r.allowance);
  }
}

// What changed in a route's start files: [{ path, delta, added }], the biggest growth first; shrinking files last.
export function changedFiles(base, change) {
  const before = new Map((base?.files ?? []).map((f) => [f.path, f.bytes]));
  const after = new Map(change.files.map((f) => [f.path, f.bytes]));
  const out = [];
  for (const path of new Set([...before.keys(), ...after.keys()])) {
    const delta = (after.get(path) ?? 0) - (before.get(path) ?? 0);
    if (delta) out.push({ path, delta, added: !before.has(path), removed: !after.has(path) });
  }
  return out.sort((a, b) => b.delta - a.delta || a.path.localeCompare(b.path));
}

// `base` and `change` are measureTree's { route: { bytes, files } } (base null: no merge-base to compare with);
// `limits` is the budgets file. One row per route with what is wrong with it, if anything.
export function decide({ base, change, limits }) {
  validateLimits(limits);
  const wanted = Object.keys(limits.routes);
  for (const route of Object.keys(change)) {
    if (!wanted.includes(route)) throw new Error(`${route} is measured but has no limits in the budgets file`);
  }
  return wanted.map((route) => {
    const now = change[route];
    if (!now) throw new Error(`${route} has limits in the budgets file but was not measured`);
    const spec = limits.routes[route];
    const before = base?.[route];
    const allowance = (spec.allowance ?? limits.allowance).bytes;
    const growth = before ? now.bytes - before.bytes : 0;
    const problems = [];
    if (growth > allowance) problems.push({ kind: 'growth', growth, allowance, why: (spec.allowance ?? limits.allowance).why });
    if (now.bytes > spec.ceiling.bytes) problems.push({ kind: 'ceiling', over: now.bytes - spec.ceiling.bytes, ceiling: spec.ceiling.bytes, why: spec.ceiling.why });
    return { route, base: before?.bytes ?? null, change: now.bytes, growth, allowance, ceiling: spec.ceiling.bytes, problems, files: changedFiles(before, now) };
  });
}

// The table, then what to do about each route that is over. Lines to print; `ok` is whether the check passes.
export function report(rows, file) {  // `file`: where the limits are, named in the message
  const head = ['route', 'base', 'change', 'growth', 'allowance', 'ceiling', ''];
  const cells = rows.map((r) => [r.route, r.base === null ? 'new' : n.format(r.base), n.format(r.change),
    r.base === null ? '' : signed(r.growth), n.format(r.allowance), n.format(r.ceiling), r.problems.length ? 'OVER' : 'ok']);
  const width = head.map((h, i) => Math.max(h.length, ...cells.map((c) => c[i].length)));
  const line = (c) => c.map((v, i) => (i === 0 || i === c.length - 1 ? v.padEnd(width[i]) : v.padStart(width[i]))).join('  ').trimEnd();
  const lines = ['budgets: shipped bytes of each route\'s start, as the hub sends them (response headers not counted)', line(head), ...cells.map(line)];
  const over = rows.filter((r) => r.problems.length);
  // Routes that share a start fail the same way: say it once, naming them.
  const groups = new Map();
  for (const r of over) {
    const key = JSON.stringify([r.base, r.change, r.problems.map(({ why, ...p }) => p), r.files]);
    groups.set(key, [...(groups.get(key) ?? []), r]);
  }
  for (const [, [first, ...rest]] of groups) {
    const names = [first, ...rest].map((r) => r.route).join(', ');
    lines.push('');
    for (const p of first.problems) {
      if (p.kind === 'growth') lines.push(`budgets: ${names}: grew ${bytes(p.growth)} over the merge-base (${bytes(first.base)} to ${bytes(first.change)}); the allowance is ${bytes(p.allowance)}`);
      else lines.push(`budgets: ${names}: ${bytes(first.change)} is ${bytes(p.over)} over the ceiling of ${bytes(p.ceiling)}`);
    }
    const grown = first.files.filter((f) => f.delta > 0).slice(0, 8);
    if (grown.length) lines.push(`  what grew: ${grown.map((f) => `${f.path} ${signed(f.delta)} B${f.added ? ' (new at start)' : ''}`).join(', ')}`);
    const shrunk = first.files.filter((f) => f.delta < 0).slice(-3);
    if (shrunk.length) lines.push(`  what shrank: ${shrunk.map((f) => `${f.path} ${signed(f.delta)} B${f.removed ? ' (no longer at start)' : ''}`).join(', ')}`);
  }
  if (over.length) {
    const grew = over.find((r) => r.problems.some((p) => p.kind === 'growth'));
    if (grew) lines.push('', `budgets: the allowance: ${grew.problems.find((p) => p.kind === 'growth').why}`);
    lines.push('',
      'budgets: over budget. First cut the growth: load code where it is used instead of at start (docs/context/web.md, Code',
      'loads where it is used), and leave out what no first load needs. If it is still needed, attach an impact note to the',
      'change (what grew, what a listener would feel, what you tried) and wait for the owner\'s yes or no. A label or a commit',
      'footer never passes this check.',
      `With the yes, a commit that only raises the allowance or ceiling in ${file}, with the owner's reason, lands first:`,
      'the check judges a change by the limits of its merge-base, so a change cannot raise the limit it is judged by.');
  }
  return { ok: !over.length, lines };
}
