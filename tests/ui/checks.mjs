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

// Null when tags side by side (components.css, Tags) are one height on one centre line, with icons of one size; else the
// first row that is not, with each tag's height, centre and icon. A row's tags are those on one line of the same parent, past a wrapper that only
// holds one (the source badge's span): the wrapper's own line box once lifted the badge 1.25 px off its neighbours.
export function unevenTags(page) {
  return page.evaluate(() => {
    const TAGS = '.badge, .q-chip, .badge-new, .device__fact, .pbar__private, .mini__private';
    const shown = (el) => {
      const r = el.getBoundingClientRect(), s = getComputedStyle(el);
      return r.width > 1 && r.height > 1 && s.visibility !== 'hidden' && s.clipPath === 'none';
    };
    const rowOf = (el) => { let p = el.parentElement; while (p.children.length === 1 && p.parentElement) p = p.parentElement; return p; };
    const rows = new Map();
    for (const el of document.querySelectorAll(TAGS)) {
      if (!shown(el)) continue;
      const box = el.getBoundingClientRect(), ic = el.querySelector(':scope > .ic');
      const tag = { el, h: box.height, cy: (box.top + box.bottom) / 2, ic: ic && shown(ic) ? ic.getBoundingClientRect().width : null };
      const lines = rows.get(rowOf(el)) || [];
      const line = lines.find((l) => Math.abs(l[0].cy - tag.cy) < 6);
      line ? line.push(tag) : lines.push([tag]);
      rows.set(rowOf(el), lines);
    }
    for (const line of [...rows.values()].flat()) {
      const spread = (xs) => (xs.length ? Math.max(...xs) - Math.min(...xs) : 0);
      const hs = line.map((t) => t.h), cs = line.map((t) => t.cy), ics = line.map((t) => t.ic).filter((w) => w !== null);
      if (line.length > 1 && (spread(hs) > 0.5 || spread(cs) > 0.5 || spread(ics) > 0.5)) {
        return line.map((t) => `.${t.el.classList[0]} ${t.h.toFixed(1)}px tall, centre ${t.cy.toFixed(1)}${t.ic === null ? '' : `, icon ${t.ic.toFixed(1)}`}`).join(' | ');
      }
    }
    return null;
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

// What is wrong with the chrome (header, player bar, mini player, tab bar, the desktop app's mini) at the current width,
// as a list: a control outside its bar or the window, two controls on each other, text cut off without an ellipsis or
// spilling out, or a missing essential (play, previous, next, the player's "…", search). The header and the player
// bar fit themselves (layout.js, data-fit); this is what keeps a design from needing a width band to patch them.
// chromeNow is the check itself, self-contained, to run in the page where a test needs it at a given moment (the fit
// gate runs it inside the frame that draws a width).
export function chrome(page) { return page.evaluate(chromeNow); }
export const chromeNow = () => {
  const out = [];
  const vw = document.documentElement.clientWidth;
  const shown = (e) => {
    if (!e.getClientRects().length) return false;
    const r = e.getBoundingClientRect(); if (r.width < 2 || r.height < 2) return false;
    for (let a = e; a && a !== document.body; a = a.parentElement) {
      const s = getComputedStyle(a); if (s.visibility === 'hidden' || s.display === 'none' || +s.opacity === 0) return false;
    }
    return !e.closest('.vh, [hidden], [aria-hidden="true"]:not(.kbd)');
  };
  // what of an element shows: its box clipped by every ancestor that clips (a link in an ellipsised line is as wide
  // as its text, but shows only up to the ellipsis)
  const seen = (e, upTo) => {
    const r = e.getBoundingClientRect(); let { left, right, top, bottom } = r;
    for (let a = e.parentElement; a && a !== upTo; a = a.parentElement) {
      if (getComputedStyle(a).overflowX === 'visible') continue;
      const c = a.getBoundingClientRect();
      left = Math.max(left, c.left); right = Math.min(right, c.right); top = Math.max(top, c.top); bottom = Math.min(bottom, c.bottom);
    }
    return { left, right, top, bottom };
  };
  const label = (e) => String(e.getAttribute('aria-label') || e.getAttribute('data-act') || e.getAttribute('data-nav') || e.className || e.tagName).split(' ')[0].slice(0, 24);
  const regions = [['header', '.header'], ['bar', '#pbar'], ['mini', '#mini'], ['tabs', '.tabbar'], ['desktop mini', '[data-dk-player]']]
    .map(([n, s]) => [n, document.querySelector(s)]).filter(([, e]) => e && shown(e));
  for (const [name, reg] of regions) {
    const rr = reg.getBoundingClientRect();
    const ctrls = [...reg.querySelectorAll('button, a[href], input, [role="slider"], .kbd, .tray')].filter(shown)
      .filter((c) => !c.closest('.pbar__state, .tooltip'))         // the state tab rides above the bar on purpose
      .filter((c) => getComputedStyle(c).position !== 'fixed');   // a design may float a part elsewhere (glasshouse's tray)
    for (const c of ctrls) {
      const r = seen(c, reg);
      if (r.left < rr.left - 1 || r.right > rr.right + 1 || r.top < rr.top - 1 || r.bottom > rr.bottom + 1) out.push(`${name}: ${label(c)} sticks out`);
      else if (r.left < -1 || r.right > vw + 1) out.push(`${name}: ${label(c)} off screen`);
    }
    for (let i = 0; i < ctrls.length; i++) for (let j = i + 1; j < ctrls.length; j++) {
      const a = ctrls[i], b = ctrls[j];
      if (a.contains(b) || b.contains(a)) continue;
      const fa = a.closest('.search-field'); if (fa && fa === b.closest('.search-field')) continue;   // the field's own end buttons
      const p = seen(a, reg), q = seen(b, reg);
      if (Math.min(p.right, q.right) - Math.max(p.left, q.left) > 2 && Math.min(p.bottom, q.bottom) - Math.max(p.top, q.top) > 2) out.push(`${name}: ${label(a)} on ${label(b)}`);
    }
    const walker = document.createTreeWalker(reg, NodeFilter.SHOW_TEXT);
    for (let t = walker.nextNode(); t; t = walker.nextNode()) {
      const el = t.parentElement;
      if (!t.data.trim() || !el || !shown(el) || el.closest('.pbar__state, .tooltip, .seek__tip')) continue;
      const range = document.createRange(); range.selectNodeContents(t);
      const tr = range.getBoundingClientRect(); if (tr.width < 1) continue;
      const what = `"${t.data.trim().slice(0, 20)}"`;
      if (tr.left < rr.left - 1 || tr.right > rr.right + 1) { out.push(`${name}: text ${what} spills out`); continue; }
      for (let a = el; a && a !== reg.parentElement; a = a.parentElement) {
        const s = getComputedStyle(a); if (s.overflowX === 'visible') continue;
        if (s.textOverflow === 'ellipsis' || /\d/.test(s.webkitLineClamp || '')) break;   // it ends in an ellipsis here
        const ar = a.getBoundingClientRect();
        if (a.scrollWidth > a.clientWidth + 1 && (tr.right > ar.right + 1 || tr.left < ar.left - 1)) { out.push(`${name}: text ${what} cut off`); break; }
      }
    }
  }
  const bar = regions.find(([n]) => n === 'bar');
  if (bar) for (const act of ['play', 'prev', 'next', 'player-more']) {
    const e = bar[1].querySelector(`[data-act="${act}"]`); if (!e || !shown(e)) out.push(`bar: ${act} missing`);
  }
  const head = regions.find(([n]) => n === 'header');
  const search = head && head[1].querySelector('#header-search');
  if (head && vw >= 640 && !(search && shown(search))) out.push('header: search missing');
  return out;
};
