// Performance budgets for one route, on Lighthouse's mobile profile (simulated slow 4G, the CPU slowed
// to the budgets' device): `sh lighthouse.sh`, which runs it for every route in budgets.json, one at a
// time, because lhci drops the #fragment from URLs and would treat the routes as one page, and sets
// LHCI_CPU_SLOWDOWN for the runner it is on. Why the budgets sit where they do: docs/context/quality.md#budgets.
const fs = require('fs');
const path = require('path');

const base = process.env.LULL_URL || 'http://127.0.0.1:8080';
const route = process.env.LHCI_ROUTE;
const { routes } = require('./budgets.json');
if (!routes[route]) throw new Error(`LHCI_ROUTE must be one of: ${Object.keys(routes).join(', ')}`);
const b = routes[route];
const cpu = +process.env.LHCI_CPU_SLOWDOWN;
if (!(cpu > 0)) throw new Error('LHCI_CPU_SLOWDOWN is set by lighthouse.sh for the runner it is on: run that');

// The device cookie global-setup saved, once the hub has pairing (ADR-0002); empty before.
let cookie = '';
try {
  cookie = fs.readFileSync(path.join(__dirname, '.auth/cookie.txt'), 'utf8').trim();
} catch {
  // no pairing yet: the hub is open
}

module.exports = {
  ci: {
    collect: {
      url: [`${base}/#/${route}`],
      numberOfRuns: 3,
      settings: {
        chromeFlags: '--no-sandbox --headless=new',
        throttling: { cpuSlowdownMultiplier: cpu },
        extraHeaders: cookie ? JSON.stringify({ Cookie: cookie }) : undefined,
      },
    },
    assert: {
      assertions: {
        // shipped bytes: shipped.mjs, after this (total-byte-weight counts the hub's live answers too, which vary)
        'largest-contentful-paint': ['error', { maxNumericValue: b.lcpMs, aggregationMethod: 'median' }],
        'total-blocking-time': ['error', { maxNumericValue: b.tbtMs, aggregationMethod: 'median' }],
        'cumulative-layout-shift': ['error', { maxNumericValue: b.cls, aggregationMethod: 'median' }],
      },
    },
    upload: { target: 'filesystem', outputDir: path.join(__dirname, 'lhci', route.replace(/\//g, '-')) },
  },
};
