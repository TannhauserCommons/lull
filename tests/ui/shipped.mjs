// A route's shipped bytes against its budget (budgets.json shippedBytes), from the reports lhci just wrote for it:
// `node shipped.mjs <route>`, run by lighthouse.sh after each route. What lull ships (the page, scripts, styles, fonts,
// icons, covers) is the same bytes on every run; the hub's live answers beside it (its reads and its event stream)
// are not: how many events arrive before Lighthouse stops, a status read live. Counting both moved a route's total by
// up to 250 B between runs of the same code. Nor are Chrome's own fetches of the page's icons (type Other), made after
// the page has loaded: they race the end of the trace, and ending last they read 0 B (21,642 B gone from a route). So
// this counts every request but those, the median of the runs.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const route = process.argv[2];
const { routes } = JSON.parse(readFileSync(join(here, 'budgets.json'), 'utf8'));
if (!routes[route]) throw new Error(`a route in budgets.json: ${Object.keys(routes).join(', ')}`);
const LEFT_OUT = new Set(['Fetch', 'XHR', 'EventSource', 'Other']);

const runs = JSON.parse(readFileSync(join(here, 'lhci', route.replace(/\//g, '-'), 'manifest.json'), 'utf8'));
const bytes = runs.map((run) => JSON.parse(readFileSync(run.jsonPath, 'utf8')).audits['network-requests'].details.items
  .filter((r) => !LEFT_OUT.has(r.resourceType)).reduce((sum, r) => sum + r.transferSize, 0)).sort((a, b) => a - b);
if (!bytes.length) throw new Error(`no Lighthouse reports for ${route}`);
const median = bytes[Math.floor(bytes.length / 2)], budget = routes[route].shippedBytes;
console.log(`shipped bytes: ${median} (runs ${bytes.join(', ')}), budget ${budget}`);
if (median > budget) { console.error(`  ✘ shipped bytes over budget by ${median - budget} B`); process.exit(1); }
