// Page checks shared by the visual matrix (matrix.spec.mjs, the real hub) and the screens gate (gates/, the fixture
// hub): what a page must not do besides looking different from its golden.

// Console errors, page errors and failed requests, minus the ones `tolerated` names ({ url: regex, status: [...] }).
export function watch(page, tolerated = []) {
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
export async function settle(page, problems) {
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

// Null when every visible title has room for a word; else the narrowest. Tiles keep their size and
// grids gain columns (ADR-0007), so a sliver of a title means a grid counted its columns from the
// window rather than from its own width.
export function squeezed(page) {
  return page.evaluate(() => {
    const titles = document.querySelectorAll('.tile__title, .disc-pick__title, .trow__title, .disc-starter__title');
    const narrow = [...titles]
      .filter((el) => el.offsetParent && el.textContent.trim())
      .map((el) => ({ el, width: el.getBoundingClientRect().width }))
      // cut off and narrow: a short title ("Rising") is narrow because the word is, not because it was squeezed
      .filter((x) => x.width < 48 && x.el.scrollWidth > x.el.clientWidth + 1)
      .sort((a, b) => a.width - b.width)[0];
    return narrow ? `.${[...narrow.el.classList].join('.')} "${narrow.el.textContent.trim().slice(0, 30)}" is ${Math.round(narrow.width)}px wide` : null;
  });
}

// Null when nothing sticks out sideways; else the widest offender, to name it in the failure.
export function overflow(page) {
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
