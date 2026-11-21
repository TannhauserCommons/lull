// The verdict on shipped bytes: a route may grow over its merge-base by the allowance, and no more.
// The allowance lives in scripts/budgets/shipped-bytes.json with its reason. Pure functions; run.mjs measures and prints.

const n = new Intl.NumberFormat('en-US');
const bytes = (v) => `${n.format(v)} B`;
const signed = (v) => `${v < 0 ? '-' : '+'}${n.format(Math.abs(v))}`;

// Fails loudly on a budgets file that is not what the check reads: every allowance an object with a whole number of bytes
// (0 is "none allowed", not "off") and a reason. An allowance that is present but not that object (0, false, a string) is an
// error, never a skipped check.
export function validateLimits(limits) {
  const entry = (what, e) => {
    if (!Number.isInteger(e?.bytes) || e.bytes < 0) throw new Error(`${what}: bytes must be a whole number of bytes, 0 or more`);
    if (typeof e.why !== 'string' || !e.why.trim()) throw new Error(`${what}: a limit comes with its reason (why)`);
  };
  entry('allowance', limits.allowance);
  const routes = Object.entries(limits.routes ?? {});
  if (!routes.length) throw new Error('the budgets file has no routes');
  for (const [route, r] of routes) {
    if (r === null || typeof r !== 'object' || Array.isArray(r)) throw new Error(`${route}: a route is an object, empty or with its own allowance`);
    for (const key of Object.keys(r)) if (key !== 'allowance') throw new Error(`${route}: unknown limit "${key}"`);
    if (r.allowance !== undefined) entry(`${route} allowance`, r.allowance);
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

const measured = (m) => !!m && !m.gone;

// `base` and `change` are measureTree's { route: { bytes, files } | { gone: true } } (the change's holds the merge-base's routes
// too, to see whether their screens are still there); `limits` is the merge-base's budgets file and `ownLimits` the
// change's own. The merge-base's limits judge every route it knows, so a change can't loosen what judges it, with these
// exceptions, each the owner's own doing in the budgets file:
//   - nothing grew on any route both sides measured, and the change's file differs: that file judges, since the change
//     only moves limits (a route removed or renamed, the allowance changed);
//   - a route the merge-base doesn't know has nothing to grow from: it is listed as new, and the change's entry names it;
//   - a route the merge-base budgets and the change's file drops is listed as removed when no screen answers to its key in
//     the change's app (the screen is gone), and fails when one does.
// Returns { rows, notes }: a row per route with what is wrong with it, if anything.
export function decide({ base, change, limits, ownLimits = limits }) {
  validateLimits(limits);
  validateLimits(ownLimits);
  const grew = Object.keys(change).some((r) => measured(change[r]) && measured(base?.[r]) && change[r].bytes > base[r].bytes);
  const own = !grew && JSON.stringify(limits) !== JSON.stringify(ownLimits);
  const notes = own ? ["limits changed, nothing grew: this change's own budgets file judges it"] : [];
  const rows = [...new Set([...Object.keys(change), ...Object.keys(limits.routes)])].map((route) => {
    const inBase = Object.hasOwn(limits.routes, route), inOwn = Object.hasOwn(ownLimits.routes, route);
    const now = change[route], before = base?.[route];
    if (!measured(now)) {
      if (inOwn) throw new Error(`${route} has limits in the budgets file but was not measured`);
      return { route, removed: true, gone: true, base: measured(before) ? before.bytes : null, problems: [], files: [] };
    }
    if (!inBase && !inOwn) throw new Error(`${route} is measured but has no limits in the budgets file`);
    if (!inOwn) {
      return { route, removed: true, base: measured(before) ? before.bytes : null, change: now.bytes, problems: [{ kind: 'removed' }], files: changedFiles(before, now) };
    }
    const from = inBase && !own ? limits : ownLimits;
    const spec = from.routes[route];
    const allowance = (spec.allowance ?? from.allowance).bytes;
    const prior = inBase && measured(before) ? before : undefined;
    const growth = prior ? now.bytes - prior.bytes : 0;
    const problems = [];
    if (growth > allowance) problems.push({ kind: 'growth', growth, allowance, why: (spec.allowance ?? from.allowance).why });
    return { route, isNew: !inBase, base: prior?.bytes ?? null, change: now.bytes, growth, allowance, problems, files: changedFiles(prior, now) };
  });
  return { rows, notes };
}

// The table, then what to do about each route that is over. Lines to print; `ok` is whether the check passes.
export function report({ rows, notes = [] }, file) {  // `file`: where the limits are, named in the message
  const head = ['route', 'base', 'change', 'growth', 'allowance', ''];
  const cells = rows.map((r) => (r.removed
    ? [r.route, r.base === null ? '' : n.format(r.base), r.gone ? '' : n.format(r.change), '', '', r.gone ? 'removed (screen gone)' : 'REMOVED']
    : [r.route, r.isNew ? 'new' : r.base === null ? '' : n.format(r.base), n.format(r.change), r.isNew || r.base === null ? '' : signed(r.growth),
      n.format(r.allowance), r.problems.length ? 'OVER' : r.isNew ? 'ok (new)' : 'ok']));
  const width = head.map((h, i) => Math.max(h.length, ...cells.map((c) => c[i].length)));
  const line = (c) => c.map((v, i) => (i === 0 || i === c.length - 1 ? v.padEnd(width[i]) : v.padStart(width[i]))).join('  ').trimEnd();
  const lines = ['budgets: shipped bytes of each route\'s start, as the hub sends them (response headers not counted)', line(head), ...cells.map(line), ...notes.map((note) => `budgets: ${note}`)];
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
      if (p.kind === 'removed') lines.push(`budgets: ${names}: budgeted by the merge-base, no longer in this change's budgets file, and its screen still exists in this change's app`);
      else lines.push(`budgets: ${names}: grew ${bytes(p.growth)} over the merge-base (${bytes(first.base)} to ${bytes(first.change)}); the allowance is ${bytes(p.allowance)}`);
    }
    const grown = first.files.filter((f) => f.delta > 0).slice(0, 8);
    if (grown.length) lines.push(`  what grew: ${grown.map((f) => `${f.path} ${signed(f.delta)} B${f.added ? ' (new at start)' : ''}`).join(', ')}`);
    const shrunk = first.files.filter((f) => f.delta < 0).slice(-3);
    if (shrunk.length) lines.push(`  what shrank: ${shrunk.map((f) => `${f.path} ${signed(f.delta)} B${f.removed ? ' (no longer at start)' : ''}`).join(', ')}`);
  }
  const bytesOver = over.filter((r) => r.problems.some((p) => p.kind !== 'removed'));
  if (bytesOver.length) {
    const grew = bytesOver.find((r) => r.problems.some((p) => p.kind === 'growth'));
    if (grew) lines.push('', `budgets: the allowance: ${grew.problems.find((p) => p.kind === 'growth').why}`);
    lines.push('',
      'budgets: over budget. First cut the growth: load code where it is used instead of at start (docs/context/web.md, Code',
      'loads where it is used), and leave out what no first load needs. If it is still needed, attach an impact note to the',
      'change (what grew, what a listener would feel, what you tried) and wait for the owner\'s yes or no. A label or a commit',
      'footer never passes this check.',
      `With the yes, a commit that only raises the allowance in ${file}, with the owner's reason, lands first:`,
      'the check judges a change by the limits of its merge-base, so a change cannot raise the limit it is judged by.');
  }
  if (over.some((r) => r.removed)) {
    lines.push('', `budgets: a screen that still exists stays budgeted: put the route back in ${file}, or remove the screen`,
      'in the same change (a route whose screen is gone is listed as removed).');
  }
  return { ok: !over.length, lines };
}
