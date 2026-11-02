// Every route x design x theme, on the phone and desktop projects (playwright.config.mjs): each
// page must settle without console errors, failed requests or sideways scrolling, and match its
// golden screenshot. One test per picture, so a failure names the exact route, design and theme.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { designs, detailRoutes, fixedRoutes, slug, themes } from './routes.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const { failures: tolerated } = JSON.parse(readFileSync(join(here, 'expected-failures.json'), 'utf8'));
// A fixed "now", so relative times ("3 minutes ago") and date ranges read the same on every run.
const NOW = new Date('2026-11-01T12:00:00Z');

for (const route of [...fixedRoutes, ...detailRoutes()]) {
  for (const design of designs) {
    for (const theme of themes) {
      test(`${slug(route)} / ${design} / ${theme}`, async ({ page }) => {
        const problems = watch(page);
        await page.clock.setFixedTime(NOW);
        // The stored preferences the UI reads before first paint (index.html): design and theme.
        await page.addInitScript(
          ([d, t]) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: t })),
          [design, theme],
        );
        await page.goto(`/#${route}`);
        await settle(page, problems);

        expect.soft(await overflow(page), 'content wider than the viewport').toBeNull();
        expect.soft(problems.console, 'console errors').toEqual([]);
        expect.soft(problems.requests, 'failed requests').toEqual([]);
        await expect(page).toHaveScreenshot(`${slug(route)}-${design}-${theme}.png`);
      });
    }
  }
}

// Console errors, page errors and failed requests, minus the ones expected-failures.json names.
function watch(page) {
  const problems = { console: [], requests: [], pending: new Set() };
  const isTolerated = (url, status) =>
    tolerated.some((t) => new RegExp(t.url).test(new URL(url).pathname) && (status === undefined || t.status.includes(status)));
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    // The browser logs every failed request as "Failed to load resource": judged by `response` below.
    if (m.text().startsWith('Failed to load resource')) return;
    problems.console.push(m.text());
  });
  page.on('pageerror', (e) => problems.console.push(`uncaught: ${e.message}`));
  page.on('request', (r) => {
    if (!isLive(r.url())) problems.pending.add(r);
  });
  const done = (r) => problems.pending.delete(r);
  page.on('requestfinished', done);
  page.on('requestfailed', (r) => {
    done(r);
    // Navigating away aborts what was in flight; that is not a failure of the page.
    if (r.failure()?.errorText !== 'net::ERR_ABORTED') problems.requests.push(`${r.method()} ${r.url()}: ${r.failure()?.errorText}`);
  });
  page.on('response', (res) => {
    if (res.status() >= 400 && !isTolerated(res.url(), res.status())) {
      problems.requests.push(`${res.request().method()} ${res.url()}: ${res.status()}`);
    }
  });
  return problems;
}

// Server-sent events and audio stay open for the life of the page.
function isLive(url) {
  const path = new URL(url).pathname;
  return path === '/api/events' || path.startsWith('/api/stream/') || path.startsWith('/api/library/stream/');
}

// Loaded, fonts in, every request answered, and the view's markup unchanged for a moment
// (screens render in steps as their data arrives; `networkidle` never fires with SSE open).
async function settle(page, problems) {
  await page.waitForLoadState('load');
  await page.evaluate(() => document.fonts.ready);
  let last = '';
  let stable = 0;
  for (let i = 0; i < 100 && stable < 4; i++) {
    await page.waitForTimeout(100);
    const now = await page.evaluate(() => `${document.querySelector('#view')?.innerHTML.length}:${document.images.length}`);
    stable = now === last && problems.pending.size === 0 ? stable + 1 : 0;
    last = now;
  }
  // Decode what has loaded; a lazy image below the fold never loads, and its decode() never settles.
  await page.evaluate(() =>
    Promise.race([
      Promise.all([...document.images].filter((i) => i.complete && i.naturalWidth).map((i) => i.decode().catch(() => {}))),
      new Promise((resolve) => setTimeout(resolve, 3000)),
    ]),
  );
}

// Null when nothing sticks out sideways; else the widest offender, to name it in the failure.
function overflow(page) {
  return page.evaluate(() => {
    const doc = document.documentElement;
    if (doc.scrollWidth <= doc.clientWidth && document.body.scrollWidth <= doc.clientWidth) return null;
    const wide = [...document.querySelectorAll('body *')]
      .map((el) => ({ el, right: el.getBoundingClientRect().right }))
      .filter((x) => x.right > doc.clientWidth + 1)
      .sort((a, b) => b.right - a.right)[0];
    const name = wide ? `${wide.el.tagName.toLowerCase()}.${[...wide.el.classList].join('.')}` : 'unknown';
    return `${doc.scrollWidth}px wide in a ${doc.clientWidth}px viewport, widest: ${name}`;
  });
}
