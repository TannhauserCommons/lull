// Lining a better copy up (web/align.js) against tests/align.json, the cases the desktop's native Classic player runs
// too (app/src/desktop/classic_native/align.rs): a change to the measurement fails here until the cases are measured
// again (node tests/web/align-cases.mjs) and fails there until align.rs matches.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { recordings } from './align-cases.mjs';

const A = createRequire(import.meta.url)('../../web/align.js');
const { cases } = JSON.parse(readFileSync(new URL('../align.json', import.meta.url), 'utf8'));

for (const c of cases) {
  test(c.what, () => {
    const [a, b] = recordings(c), m = A.measure(a, b, c.rate, 1), e = c.expect;
    for (const k of ['lag', 'confidence', 'margin']) assert.ok(Math.abs(m[k] - e[k]) < 1e-9, `${k}: ${m[k]} vs ${e[k]}`);
    assert.equal(m.quiet, e.quiet);
    assert.equal(A.similar(m), e.similar);
    assert.equal(A.locked(m), e.locked);
    [A.rms(a), A.rms(b)].forEach((r, i) => assert.ok(Math.abs(r - e.levels[i]) < 1e-9, `level ${i}`));
  });
}
