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
// A design that sets its tags apart on purpose names `even-tags` in --gate-free on them or their row (gateFree).
export function unevenTags(page) {
  return page.evaluate((freeSrc) => {
    const free = (0, eval)('(' + freeSrc + ')');
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
        const said = line.map((t) => free(t.el, 'even-tags'));
        if (said.every((x) => x === 'free')) continue;
        return line.map((t) => `.${t.el.classList[0]} ${t.h.toFixed(1)}px tall, centre ${t.cy.toFixed(1)}${t.ic === null ? '' : `, icon ${t.ic.toFixed(1)}`}`).join(' | ')
          + (said.includes('no reason') ? ' (--gate-free names even-tags without a reason: say why)' : '');
      }
    }
    return null;
  }, gateFree.toString());
}

// Null when every text clamped to some lines also clips them; else the first that doesn't. A line clamp only shortens the
// box and puts the ellipsis on its last line: with overflow visible the lines after it still draw, over what follows.
export function unclippedClamps(page) {
  return page.evaluate(() => {
    const el = [...document.querySelectorAll('body *')].find((e) => {
      const s = getComputedStyle(e);
      return s.webkitLineClamp !== 'none' && s.overflowY === 'visible' && e.getClientRects().length;
    });
    return el ? `${el.tagName.toLowerCase()}.${[...el.classList].join('.')} "${el.textContent.trim().slice(0, 30)}"` : null;
  });
}

// Null when every cover shows whole in the rows and lists that scroll it; else the first one cut, with what draws round it
// and the side cut. A scroller clips at its padding box only because it scrolls, so what a design draws round a cover
// (a ring of box-shadow, an outline: Skydeck's round windows) is cut there unless the cover's own box holds it, or the
// row leaves room for it. Across the scroller's axis that is a defect wherever the cover sits; along it, only where the
// scroller's content starts or ends (a cover half scrolled out of a shelf is cut by design). A frame that clips on
// purpose (overflow hidden or clip) is left alone. clippedCoversNow is the check itself, to run inside the page.
export function clippedCovers(page) { return page.evaluate(clippedCoversNow); }
export const clippedCoversNow = () => {
  const px = (v) => parseFloat(v) || 0;
  // how far past each side of its box a cover draws: its outer shadows (offset, blur and spread: a shadow offset down
  // and to the right, a stack of sleeves, reaches past the bottom and right only) and its outline (every side)
  const reach = (s) => {
    const r = { top: 0, right: 0, bottom: 0, left: 0 };
    if (s.boxShadow !== 'none') for (const sh of s.boxShadow.split(/,(?![^(]*\))/)) {
      if (/inset/.test(sh)) continue;
      const [x = 0, y = 0, blur = 0, spread = 0] = sh.replace(/(rgba?|color)\([^)]*\)|#[0-9a-f]+/gi, '').trim().split(/\s+/).map(px);
      const out = spread + blur;
      r.top = Math.max(r.top, out - y); r.bottom = Math.max(r.bottom, out + y);
      r.left = Math.max(r.left, out - x); r.right = Math.max(r.right, out + x);
    }
    if (s.outlineStyle !== 'none') { const o = px(s.outlineWidth) + px(s.outlineOffset); for (const k in r) r[k] = Math.max(r[k], o); }
    for (const k in r) r[k] -= 0.5;
    return r;
  };
  const name = (e) => `${e.tagName.toLowerCase()}.${[...e.classList].join('.')}`;
  for (const c of document.querySelectorAll('.cover, .mosaic')) {
    if (!c.getClientRects().length || c.closest('[hidden], .vh')) continue;
    const s = getComputedStyle(c), b = c.getBoundingClientRect();
    if (s.visibility === 'hidden' || b.width < 8) continue;
    const ring = reach(s); if (Math.max(ring.top, ring.right, ring.bottom, ring.left) <= 0) continue;
    for (let a = c.parentElement; a && a !== document.body; a = a.parentElement) {
      const as = getComputedStyle(a), scroll = (o) => o === 'auto' || o === 'scroll';
      if (!scroll(as.overflowX) && !scroll(as.overflowY)) continue;
      const ab = a.getBoundingClientRect();
      const box = { left: ab.left + px(as.borderLeftWidth), right: ab.left + px(as.borderLeftWidth) + a.clientWidth, top: ab.top + px(as.borderTopWidth), bottom: ab.top + px(as.borderTopWidth) + a.clientHeight };
      const alongX = a.scrollWidth > a.clientWidth + 1, alongY = a.scrollHeight > a.clientHeight + 1;
      // where the cover sits in the scroller's content, from its start and end
      const fromTop = b.top - box.top + a.scrollTop, toBottom = a.scrollHeight - (b.bottom - box.top + a.scrollTop);
      const fromLeft = b.left - box.left + a.scrollLeft, toRight = a.scrollWidth - (b.right - box.left + a.scrollLeft);
      const cut = [];
      if (ring.top > 0 && (alongY ? fromTop < ring.top : b.top - ring.top < box.top)) cut.push('top');
      if (ring.bottom > 0 && (alongY ? toBottom < ring.bottom : b.bottom + ring.bottom > box.bottom)) cut.push('bottom');
      if (ring.left > 0 && (alongX ? fromLeft < ring.left : b.left - ring.left < box.left)) cut.push('left');
      if (ring.right > 0 && (alongX ? toRight < ring.right : b.right + ring.right > box.right)) cut.push('right');
      if (cut.length) return `${name(c)} draws ${cut.map((k) => `${(ring[k] + 0.5).toFixed(1)} px past its ${k}`).join(' and ')}, cut by the scrolling ${name(a)}`;
      break;   // the nearest scroller is the one that clips it: a cover scrolled out of a row isn't cut by the page
    }
  }
  return null;
};

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

// What a region does wrong, as a list, judged by what it does and never by its shape (ADR-0083): a control nothing can
// click (another element takes every point of it) or off the screen, text something else covers, text on other text,
// text off the screen or cut off without an ellipsis, and, in the chrome, a missing essential (play, previous, next,
// the player's "…", search). Where a part sits, whether it rises out of its bar or sits in another control's area (a
// play key in a disc), and whether a bar is full width or rectangular, are the design's: none of that is a problem.
// The regions are the chrome by default (the header, the player bar, the mini player, the tab bar, the desktop app's
// mini); a test may pass its own, [name, selector, { scrolls }], where `scrolls` says the region scrolls, so what is
// scrolled out of sight is not off the screen. The header and the player bar fit themselves (layout.js, data-fit), so
// no design needs a width band to patch them. chromeNow is the check itself, self-contained, to run in the page where a
// test needs it at a given moment (the fit gate runs it inside the frame that draws a width).
export function chrome(page, regions) { return page.evaluate(chromeNow, regions || null); }
export const chromeNow = (only) => {
  const out = [];
  const vw = document.documentElement.clientWidth, vh = document.documentElement.clientHeight;
  const shown = (e) => {
    if (!e.getClientRects().length) return false;
    const r = e.getBoundingClientRect(); if (r.width < 2 || r.height < 2) return false;
    for (let a = e; a && a !== document.body; a = a.parentElement) {
      const s = getComputedStyle(a); if (s.visibility === 'hidden' || s.display === 'none' || +s.opacity === 0) return false;
    }
    return !e.closest('.vh, [hidden], [aria-hidden="true"]:not(.kbd)');
  };
  // what of a box shows: clipped by every ancestor that clips (a link in an ellipsised line is as wide as its text, but
  // shows only up to the ellipsis), and whether one that scrolls cut it. A text's lines are clipped by the element that
  // holds them too (`self`): a title clamped to two lines lays out a third that never shows
  const seen = (e, r = e.getBoundingClientRect(), self = false) => {
    let { left, right, top, bottom } = r, scrolled = false;
    for (let a = self ? e : e.parentElement; a && a !== document.documentElement; a = a.parentElement) {
      const s = getComputedStyle(a); if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
      const c = a.getBoundingClientRect();
      const cut = left < c.left || right > c.right || top < c.top || bottom > c.bottom;
      if (cut && /auto|scroll/.test(s.overflowX + s.overflowY)) scrolled = true;
      left = Math.max(left, c.left); right = Math.min(right, c.right); top = Math.max(top, c.top); bottom = Math.min(bottom, c.bottom);
    }
    return { left, right, top, bottom, scrolled, empty: right - left < 1 || bottom - top < 1 };
  };
  const label = (e) => String(e.getAttribute('aria-label') || e.getAttribute('data-act') || e.getAttribute('data-nav') || (typeof e.className === 'string' && e.className) || e.tagName).split(' ')[0].slice(0, 24);
  const offScreen = (r) => r.left < -1 || r.right > vw + 1 || r.top < -1 || r.bottom > vh + 1;
  // what a click at a point reaches belongs to e: e itself, a part inside it, or (where e lets clicks through) a box
  // round it; for text, any part of the control it labels too (an icon over a button's word is the button's own make-up)
  const CONTROL = 'button, a[href], input, [role="slider"], .kbd, .tray';
  const mine = (e, hit) => !!hit && (hit === e || e.contains(hit) || hit.contains(e));
  const ownControl = (e, hit) => { const c = e.closest(CONTROL); return !!c && !!hit && c.contains(hit); };
  // what lies over text hides it only where it paints: a background, a picture, a border; an empty box stacked over
  // a line (a title's box stretched across its strip) hides nothing. Text drawn there is text on text, below
  const paints = (hit, el) => {
    for (let a = hit; a && !a.contains(el); a = a.parentElement) {
      if (/^(IMG|SVG|CANVAS|VIDEO|PICTURE|svg|path|use|circle|rect)$/.test(a.tagName)) return true;
      const s = getComputedStyle(a);
      if (s.backgroundImage !== 'none' || !/^(transparent|rgba\([^)]*,\s*0\))$/.test(s.backgroundColor)) return true;
      if (['Top', 'Right', 'Bottom', 'Left'].some((k) => parseFloat(s['border' + k + 'Width']) > 0 && s['border' + k + 'Style'] !== 'none')) return true;
    }
    return false;
  };
  // what scrolls is scrolled clear of what lies over its scroller from outside it: the page under the chrome (a region
  // that scrolls), lyrics under a foot laid over their bottom edge
  const scroller = (e) => {
    for (let a = e.parentElement; a && a !== document.documentElement; a = a.parentElement) {
      const s = getComputedStyle(a);
      if ((/auto|scroll/.test(s.overflowY) && a.scrollHeight > a.clientHeight + 1) || (/auto|scroll/.test(s.overflowX) && a.scrollWidth > a.clientWidth + 1)) return a;
    }
    return null;
  };
  const scrollsClear = (reg, opts, hit, e) => {
    if (!hit) return false;
    if (opts.scrolls && !reg.contains(hit)) return true;
    const sc = scroller(e);
    return !!sc && !sc.contains(hit);
  };
  // the element a click reaches over e's box, if none of it reaches e: its centre first, then a grid of 5 x 5 points
  const blocker = (e, r) => {
    const l = Math.max(r.left, 0), t = Math.max(r.top, 0), w = Math.min(r.right, vw) - l, h = Math.min(r.bottom, vh) - t;
    if (w < 1 || h < 1) return null;
    const first = document.elementFromPoint(l + w / 2, t + h / 2);
    if (mine(e, first)) return null;
    for (let i = 0; i < 5; i++) for (let j = 0; j < 5; j++) if (mine(e, document.elementFromPoint(l + (w * (i + 0.5)) / 5, t + (h * (j + 0.5)) / 5))) return null;
    return first || document.documentElement;
  };
  const CHROME = [['header', '.header'], ['bar', '#pbar'], ['mini', '#mini'], ['tabs', '.tabbar'], ['desktop mini', '[data-dk-player]']];
  const regions = (only || CHROME).map(([n, s, o]) => [n, document.querySelector(s), o || {}]).filter(([, e]) => e && shown(e));
  for (const [name, reg, opts] of regions) {
    // a tooltip is there for a moment, under the pointer: not a part of the region
    const ctrls = [...reg.querySelectorAll(CONTROL)].filter(shown).filter((c) => !c.closest('.tooltip'));
    for (const c of ctrls) {
      const r = seen(c);
      if (r.empty) continue;                                         // scrolled or given way out of sight
      if (offScreen(r)) { if (!(opts.scrolls && r.scrolled)) out.push(`${name}: ${label(c)} off screen`); continue; }
      const b = blocker(c, r);
      if (b && !scrollsClear(reg, opts, b, c)) out.push(`${name}: ${label(c)} can't be clicked, under ${label(b)}`);
    }
    const texts = [];
    const walker = document.createTreeWalker(reg, NodeFilter.SHOW_TEXT);
    for (let t = walker.nextNode(); t; t = walker.nextNode()) {
      const el = t.parentElement;
      if (!t.data.trim() || !el || !shown(el) || el.closest('.tooltip, .seek__tip')) continue;
      const range = document.createRange(); range.selectNodeContents(t);
      const what = `"${t.data.trim().slice(0, 20)}"`;
      const lines = [...range.getClientRects()].filter((x) => x.width >= 1).map((x) => seen(el, x, true)).filter((x) => !x.empty);
      if (!lines.length) continue;
      if (lines.some(offScreen)) { if (!(opts.scrolls && lines.some((x) => x.scrolled))) out.push(`${name}: text ${what} off screen`); continue; }
      const tr = range.getBoundingClientRect();
      let cut = false;
      for (let a = el; a && a !== reg.parentElement; a = a.parentElement) {
        const s = getComputedStyle(a); if (s.overflowX === 'visible') continue;
        if (s.textOverflow === 'ellipsis' || /\d/.test(s.webkitLineClamp || '')) break;   // it ends in an ellipsis here
        const ar = a.getBoundingClientRect();
        if (a.scrollWidth > a.clientWidth + 1 && (tr.right > ar.right + 1 || tr.left < ar.left - 1)) { out.push(`${name}: text ${what} cut off`); cut = true; break; }
      }
      if (cut) continue;
      // text that lets clicks through (a label laid over a band) can't be found by a click: its order isn't knowable here
      const x = lines[0], hit = getComputedStyle(el).pointerEvents === 'none' ? null : document.elementFromPoint((x.left + x.right) / 2, (x.top + x.bottom) / 2);
      if (hit && !mine(el, hit) && !ownControl(el, hit) && !hit.closest('.tooltip') && !scrollsClear(reg, opts, hit, el) && paints(hit, el)) { out.push(`${name}: text ${what} under ${label(hit)}`); continue; }
      // a line's glyphs, not its line box: lines set tighter than the font's own height (a heading at line-height 1.1)
      // have boxes that overlap where no letters do
      texts.push({ el, what, lines: lines.map((x) => { const h = x.bottom - x.top; return { left: x.left, right: x.right, top: x.top + h * 0.25, bottom: x.bottom - h * 0.25 }; }) });
    }
    const meet = (p, q) => Math.min(p.right, q.right) - Math.max(p.left, q.left) > 2 && Math.min(p.bottom, q.bottom) - Math.max(p.top, q.top) > 2;
    for (let i = 0; i < texts.length; i++) for (let j = i + 1; j < texts.length; j++) {
      const a = texts[i], b = texts[j];
      if (a.el === b.el || a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const sa = scroller(a.el), sb = scroller(b.el);
      if ((sa && !sa.contains(b.el)) || (sb && !sb.contains(a.el))) continue;   // one scrolls clear of the other
      if (a.lines.some((p) => b.lines.some((q) => meet(p, q)))) out.push(`${name}: text ${a.what} on text ${b.what}`);
    }
  }
  if (only) return out;
  const bar = regions.find(([n]) => n === 'bar');
  if (bar) for (const act of ['play', 'prev', 'next', 'player-more']) {
    const e = bar[1].querySelector(`[data-act="${act}"]`); if (!e || !shown(e)) out.push(`bar: ${act} missing`);
  }
  const head = regions.find(([n]) => n === 'header');
  const search = head && head[1].querySelector('#header-search');
  if (head && vw >= 640 && !(search && shown(search))) out.push('header: search missing');
  return out;
};

// A design's own choice where a check can't tell a deliberate shape from a bug (ADR-0083): the check is named in the
// custom property --gate-free on the element (or a box round it, as custom properties inherit) with the reason, in the
// design's CSS: `--gate-free: "even-tags: the chips lean like cards on a desk"`, several separated by ";". Returns
// 'free' when the check is named with a reason, 'no reason' when it is named without one (the check still runs, and
// says so), else ''. Self-contained, to run in the page (pass its source where a check runs there).
export const gateFree = (el, check) => {
  const v = getComputedStyle(el).getPropertyValue('--gate-free').trim().replace(/^(["'])([\s\S]*)\1$/, '$2');
  for (const part of v.split(';')) {
    const m = /^\s*([\w-]+)\s*(?::\s*([\s\S]*?))?\s*$/.exec(part);
    if (m && m[1] === check) return m[2] && m[2].split(/\s+/).length >= 3 ? 'free' : 'no reason';
  }
  return '';
};
