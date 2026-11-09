// Page checks shared by the visual matrix (matrix.spec.mjs, the real hub) and the screens gate (gates/, the fixture
// hub): what a page must not do besides looking different from its golden.

import { readFileSync } from 'node:fs';

// lull's features and how each is reached (ADR-0101, section 6): the features check below holds every design and theme
// to them. A finding the owner has yet to decide on is in gates/features-known.json.
export const FEATURES = JSON.parse(readFileSync(new URL('../../web/designs/features.json', import.meta.url), 'utf8'));
const KNOWN_FEATURES = JSON.parse(readFileSync(new URL('./gates/features-known.json', import.meta.url), 'utf8')).known;

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
// text off the screen or cut off without an ellipsis, and, with the chrome, a feature nothing reaches (`features`,
// below). Where a part sits, whether it rises out of its bar or sits in another control's area (a play key in a disc),
// and whether a bar is full width or rectangular, are the design's: none of that is a problem.
// The regions are the chrome by default (the header, the player bar, the mini player, the tab bar, the desktop app's
// mini); a test may pass its own, [name, selector, { scrolls }], where `scrolls` says the region scrolls, so what is
// scrolled out of sight is not off the screen. The header and the player bar fit themselves (layout.js, data-fit), so
// no design needs a width band to patch them. chromeNow is the check itself, self-contained, to run in the page where a
// test needs it at a given moment (the fit gate runs it inside the frame that draws a width): chromeNow(regions, feats),
// where feats ({ list, where, holder }) runs the features check too.
export function chrome(page, regions) { return runChrome(page, regions || null, regions ? null : { list: FEATURES }); }
const runChrome = (page, only, feats) => page.evaluate(([src, o, f]) => (0, eval)('(' + src + ')')(o, f), [chromeNow.toString(), only, feats]);

// Every feature lull has (web/designs/features.json, ADR-0101 section 6) is reached where it is expected: by a control a
// click reaches (shown, at least the 24 x 24 px the accessibility gate asks, on the screen and reached by a click at some
// point of it, as the chrome check's hit test finds, or out of sight only in something that scrolls), or it is held in
// something one such click opens: the player's "…" menu, the phone's More sheet, Following and playlists, and on a phone
// Now Playing, the player's sheet (`holders` in features.json). That a holder holds what features.json says is checked
// by opening it (`holders`, below), not in every picture: a theme may hide, move, group, fold or redraw any of lull's
// controls, and this still knows every feature is there. `where` overrides what is read from the page: { screen, phone
// (under 640 px), player (a song in the player), app (the desktop app) }. One string per feature nothing reaches,
// "feature <id>: ...".
export function features(page, list = FEATURES, where = {}) { return runChrome(page, [], { list, where }); }

// Whether the layers one click opens hold what features.json says they do: each `layer` holder whose opener can be
// clicked now (Playwright's click, which fails when something else would take it) is opened, every feature expected here
// that names it must be in it, reached by a click inside it (one of the feature's controls, or a menu item in its words),
// and Escape closes it. A few hundred ms a holder: the fit gate opens them at a few widths, not at every one.
export async function holders(page, list = FEATURES, where = {}) {
  const out = [];
  for (const [id, h] of Object.entries(list.holders)) {
    if (!h.layer) continue;
    const opener = page.locator(h.opens.join(', ')).filter({ visible: true }).first();
    if (!(await opener.count())) continue;
    const layer = page.locator('body > .menu.is-open, body > .popover.is-open, #overlay-host > .is-open:is(.sheet, .dialog)').last();
    let el;
    try {
      await opener.click({ timeout: 3000 });
      el = await layer.elementHandle({ timeout: 3000 });
    } catch (e) { out.push(`holder ${id}: ${h.name} doesn't open (${e.message.split('\n')[0]})`); continue; }
    await arrived(page);   // judged once it has arrived (a sheet slides up)
    out.push(...await runChrome(page, [], { list, where, holder: id }));
    await page.keyboard.press('Escape');
    // gone, not only closing: a sheet slides away over the page before it is removed
    await page.waitForFunction((e) => !e.isConnected, el, { timeout: 3000 }).catch(() => out.push(`holder ${id}: Escape doesn't close ${h.name}`));
    await el.dispose();
  }
  return out;
}
const arrived = (page) => page.evaluate(() => Promise.race([
  Promise.all(document.getAnimations().filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime)).map((a) => a.finished.catch(() => {}))),
  new Promise((r) => setTimeout(r, 2000)),
]));

// What the features check found, less what gates/features-known.json lists for a built-in design at this width and
// screen (findings the owner has yet to decide on; the list only shrinks).
export function unknownFeatures(found, { design, width, screen }) {
  return found.filter((p) => {
    const id = /^feature (\S+):/.exec(p)?.[1];
    return !(id && KNOWN_FEATURES.some((k) => k.feature === id && k.designs.includes(design) &&
      (!k.widths || (width >= k.widths[0] && width <= k.widths[1])) && (!k.screens || k.screens.includes(screen))));
  });
}

export const chromeNow = (only, feats) => {
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
  // a line (a title's box stretched across its strip) hides nothing, and neither does a box that doesn't show (faded
  // out, its own opacity or an ancestor's 0, or visibility hidden: a note gone from a screen still takes the point).
  // Text drawn there is text on text, below
  const invisible = (a) => {
    if (getComputedStyle(a).visibility === 'hidden') return true;
    for (let p = a; p; p = p.parentElement) if (+getComputedStyle(p).opacity === 0) return true;
    return false;
  };
  const paints = (hit, el) => {
    for (let a = hit; a && !a.contains(el); a = a.parentElement) {
      if (invisible(a)) continue;
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
  // the features (`features` above): lull's page only, not the desktop app's mini player windows
  const app = document.getElementById('app');
  if (!feats || !app) return out;
  const { list, where = {}, holder } = feats;
  const cur = window.App && App.player && App.player.current();
  const at = {
    screen: where.screen || app.getAttribute('data-screen'), phone: where.phone ?? vw < 640,
    player: where.player ?? !!cur, app: where.app ?? !!(window.LullDesktop && LullDesktop.inWrapper),
  };
  const song = at.player && cur ? String(cur.id) : '';
  const HIT = 24;   // the smallest target the accessibility gate takes (axe's target-size, WCAG 2.2 AA)
  // why a click can't use a control, or '' when it can
  const why = (c) => {
    if (!shown(c) || c.closest('.tooltip')) return 'hidden';
    const box = c.getBoundingClientRect();
    if (box.width < HIT - 0.5 || box.height < HIT - 0.5) return `${Math.round(box.width)} x ${Math.round(box.height)} px, under ${HIT}`;
    const r = seen(c);
    if (r.empty || offScreen(r)) return r.scrolled ? '' : 'off screen';   // out of sight in a scroller: scrolling brings it in
    const b = blocker(c, r);
    return b ? `under ${label(b)}` : '';
  };
  const found = (s, root) => (s.includes('{song}') && !song ? [] : [...root.querySelectorAll(s.split('{song}').join(CSS.escape(song)))]);
  const reached = (sels, root = document) => sels.some((s) => found(s, root).some((c) => !why(c)));
  const expected = (f) => {
    const rule = f.where[at.phone ? 'phone' : 'wide'];
    if (!rule || (f.player && !at.player) || (f.app && !at.app)) return false;
    return rule === 'all' || (rule === 'not-now' ? at.screen !== 'now' : rule.includes(at.screen));
  };
  if (holder) {
    // a holder open now (the newest layer): what features.json says it holds is in it, reached by a click there
    const h = list.holders[holder];
    const layer = [...document.querySelectorAll('body > .menu.is-open, body > .popover.is-open, #overlay-host > .is-open:is(.sheet, .dialog)')].pop();
    if (!layer) return [`holder ${holder}: ${h.name} isn't open`];
    const item = (f) => !!f.label && [...layer.querySelectorAll('[role="menuitem"], .menu__item')].some((m) => new RegExp(f.label).test(m.textContent.trim()) && !why(m));
    for (const f of list.features) {
      if ((f.in || []).includes(holder) && expected(f) && !reached(f.controls, layer) && !item(f)) out.push(`feature ${f.id}: ${h.name} doesn't hold ${f.name}`);
    }
    return out;
  }
  const opens = (id) => { const h = list.holders[id]; return !!h && (!h.phone || at.phone) && reached(h.opens); };
  for (const f of list.features) {
    if (!expected(f) || reached(f.controls) || (f.in || []).some(opens)) continue;
    // what came closest: a control of the feature's that is there but can't be used
    const near = f.controls.flatMap((s) => found(s, document)).map((c) => [c, why(c)]).find(([, w]) => w !== 'hidden');
    out.push(`feature ${f.id}: nothing a click reaches gives ${f.name}${near ? ` (${label(near[0])}: ${near[1]})` : ''}`);
  }
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

// Null when every line of text on the screen reads against what is actually drawn behind it; else the worst spot, by how
// far it falls short. Each visible text's colour (with its opacity) is set against the pixels under its glyphs in a
// picture of the page with every text made transparent, so a picture, a gradient or glass behind a line counts as what
// it is, not as the token it was meant to sit on. A text passes when 95% of the pixels behind it give 4.5:1, or 3:1 for
// large text (24px, or 18.66px bold): WCAG's minimums; the 5% left are the edges of whatever frames it.
// - An outline (-webkit-text-stroke) is ink too: a line passes where its fill or its outline reads against what is
//   behind, or, for an outline 2px or wider, where the fill reads against the outline round it.
// - A line that something laid over it hides (a fade painted over a list's foot) is not judged: one that barely shows
//   when every text is drawn in a probe colour, magenta.
// - Text a mask fades itself (on it or round it: a list dissolving at a scroller's edge) is judged only where no mask
//   draws: a third picture with the masks taken off finds those pixels. Text that sits over a masked shape (fog, a
//   feathered patch, an icon cut from a fill) is judged against that shape as drawn.
// - With { seeThrough: true } (a window the desktop shows through) a see-through pixel is judged over black and over
//   white, the worse of the two.
// Not judged: text in form fields, in SVG and in generated content (::before, ::after), and text whose fill is a picture
// (background-clip: text). A text's shadows stay in every picture: they paint whatever colour the text is, so what
// they draw behind the letters is judged as what a reader sees there (a dense shadow close to the letters backs them;
// a faint one doesn't).
// Left out as decoration: text drawn at under half opacity (its own times its ancestors') and text a filter blurs (on it
// or round it): a blurred, faded copy of a title behind it. Text a screen reader skips (aria-hidden) is still judged when
// it shows: a sighted listener reads a visible label (Now Playing's "Devices") whatever the button's accessible name.
export async function contrast(page, { seeThrough = false } = {}) {
  const texts = await page.evaluate(contrastTexts);
  if (!texts.length) return null;
  const shoot = async (css) => {
    const style = css && await page.addStyleTag({ content: css });
    try {
      await page.evaluate(() => new Promise((ok) => requestAnimationFrame(() => requestAnimationFrame(ok))));
      return (await page.screenshot({ type: 'png', omitBackground: seeThrough })).toString('base64');
    } finally { if (style) await style.evaluate((s) => s.remove()); }
  };
  const inked = (c) => `*, *::before, *::after { color: ${c} !important; -webkit-text-fill-color: ${c} !important; ` +
    `-webkit-text-stroke-color: ${c} !important; transition: none !important; }`;
  const probe = await shoot(inked('#f0f')), clear = await shoot(inked('transparent'));
  const unmasked = await shoot(inked('transparent') + ' *, *::before, *::after { -webkit-mask: none !important; mask: none !important; }');
  return page.evaluate(contrastWorst, { texts, probe, clear, unmasked, seeThrough });
}

// In the page: each visible text node's fill and outline as drawn, its size class, and the boxes of its glyphs as far as
// they show
const contrastTexts = () => {
  const canvas = document.createElement('canvas'); canvas.width = canvas.height = 1;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  const rgba = (css) => { ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = '#000'; ctx.fillStyle = css; ctx.fillRect(0, 0, 1, 1); return [...ctx.getImageData(0, 0, 1, 1).data]; };
  const vw = document.documentElement.clientWidth, vh = document.documentElement.clientHeight;
  // a box cut to what its clipping ancestors and the window let show
  const shows = (el, r) => {
    let { left, right, top, bottom } = r;
    for (let a = el; a && a !== document.documentElement; a = a.parentElement) {
      const s = getComputedStyle(a); if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
      const c = a.getBoundingClientRect();
      left = Math.max(left, c.left); right = Math.min(right, c.right); top = Math.max(top, c.top); bottom = Math.min(bottom, c.bottom);
    }
    left = Math.max(left, 0); top = Math.max(top, 0); right = Math.min(right, vw); bottom = Math.min(bottom, vh);
    return right - left >= 1 && bottom - top >= 1 ? { left, right, top, bottom } : null;
  };
  const out = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let t = walker.nextNode(); t; t = walker.nextNode()) {
    const el = t.parentElement;
    if (!t.data.trim() || !el || el.closest('svg, script, style, template, .vh, .tooltip')) continue;
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
    const s = getComputedStyle(el);
    let opacity = 1, through = false, picture = false, blurred = false, masked = false;
    for (let p = el; p; p = p.parentElement) {
      const ps = getComputedStyle(p);
      opacity *= +ps.opacity;
      masked = masked || [ps.maskImage, ps.webkitMaskImage].some((m) => m && m !== 'none');
      through = through || ps.pointerEvents === 'none';
      picture = picture || /text/.test(ps.backgroundClip + ps.webkitBackgroundClip);
      blurred = blurred || [...ps.filter.matchAll(/blur\(\s*([\d.]+)/g)].some((m) => +m[1] > 0);
    }
    if (picture) continue;   // its fill is a picture: no one colour to judge
    if (opacity < 0.5 || blurred) continue;   // decoration: faded or blurred, not meant to be read
    const fill = rgba(s.webkitTextFillColor || s.color), alpha = (fill[3] / 255) * opacity;
    const strokeWidth = parseFloat(s.webkitTextStrokeWidth) || 0, ink = rgba(s.webkitTextStrokeColor);
    const stroke = strokeWidth >= 0.5 && ink[3] > 5 ? { rgb: ink.slice(0, 3), alpha: (ink[3] / 255) * opacity, width: strokeWidth } : null;
    if (alpha < 0.02 && !stroke) continue;   // nothing drawn: transparent text
    const range = document.createRange(); range.selectNodeContents(t);
    // each line's whole box: its letters are found in it by their ink (a comma below the middle of the line too)
    const rects = [...range.getClientRects()].filter((x) => x.width >= 1 && x.height >= 1)
      .map((x) => shows(el, { left: x.left, right: x.right, top: x.top, bottom: x.bottom })).filter(Boolean);
    if (!rects.length) continue;
    // something that takes clicks laid over it hides it: the chrome check's business, not this one's (text that lets
    // clicks through can't be found by a click: the picture says whether it shows)
    if (!through) {
      const x = rects[0], hit = document.elementFromPoint((x.left + x.right) / 2, (x.top + x.bottom) / 2);
      if (!hit || !(hit === el || el.contains(hit) || hit.contains(el) || (el.closest('button, a[href], label') || el).contains(hit))) continue;
    }
    const size = parseFloat(s.fontSize), weight = parseInt(s.fontWeight, 10) || 400;
    const cls = typeof el.className === 'string' && el.className.trim() ? '.' + el.className.trim().split(/\s+/).join('.') : '';
    out.push({ name: `${el.tagName.toLowerCase()}${cls} "${t.data.trim().replace(/\s+/g, ' ').slice(0, 24)}"`, fg: fill.slice(0, 3), alpha, stroke, masked,
      large: size >= 24 || (size >= 18.66 && weight >= 700), rects });
  }
  return out;
};

// In the page: the texts' ratios against the picture with the text taken out, and the worst that falls short
const contrastWorst = async ({ texts, probe, clear, unmasked, seeThrough }) => {
  const pixels = async (png) => {
    const img = new Image(); img.src = 'data:image/png;base64,' + png; await img.decode();
    const canvas = document.createElement('canvas'); canvas.width = img.naturalWidth; canvas.height = img.naturalHeight;
    const ctx = canvas.getContext('2d', { willReadFrequently: true }); ctx.drawImage(img, 0, 0);
    return ctx.getImageData(0, 0, img.naturalWidth, img.naturalHeight);
  };
  const [probed, behind, bare] = await Promise.all([probe, clear, unmasked].map(pixels));
  const w = behind.width, h = behind.height, k = w / window.innerWidth;
  const at = (x, y) => (Math.min(h - 1, Math.floor(y)) * w + Math.min(w - 1, Math.floor(x))) * 4;
  const px = (img, i) => [img.data[i], img.data[i + 1], img.data[i + 2]];
  const masked = (i) => [0, 1, 2, 3].some((c) => Math.abs(behind.data[i + c] - bare.data[i + c]) > 6);
  const lum = (c) => { const f = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2]); };
  const ratio = (p, q) => { const a = lum(p), b = lum(q); return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05); };
  const over = (top, a, under) => top.map((v, i) => a * v + (1 - a) * under[i]);
  const dist = (p, q) => Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
  const hex = (c) => '#' + c.map((v) => Math.round(v).toString(16).padStart(2, '0')).join('');
  // a text's inks over one pixel: its fill, and its outline
  const inks = (t, bg) => {
    const edge = t.stroke && over(t.stroke.rgb, t.stroke.alpha, bg);
    return { fill: over(t.fg, t.alpha, edge && t.stroke.width >= 2 ? edge : bg), edge };
  };
  // the best they do against it: the fill or the outline against what is behind, or the fill against a wide outline
  const reads = (t, bg) => {
    const { fill, edge } = inks(t, bg);
    return Math.max(ratio(over(t.fg, t.alpha, bg), bg), edge ? ratio(edge, bg) : 0, edge && t.stroke.width >= 2 ? ratio(fill, edge) : 0);
  };
  // whether a line shows: the most its probe-coloured glyphs change a pixel, beside the most they would at the text's
  // opacity. Behind a probe-coloured spot (a magenta picture) the probe can't tell, and the line is judged
  const PROBE = [255, 0, 255];
  const hidden = (t, r) => {
    const alpha = Math.max(t.alpha, t.stroke ? t.stroke.alpha : 0);
    let seen = 0, meant = 0;
    for (let y = Math.floor(r.top * k); y < r.bottom * k; y++) for (let x = Math.floor(r.left * k); x < r.right * k; x++) {
      const i = at(x, y); if (t.masked && masked(i)) continue;
      const bg = px(behind, i);
      meant = Math.max(meant, dist(over(PROBE, alpha, bg), bg));
      seen = Math.max(seen, dist(px(probed, i), bg));
    }
    return meant >= 48 && seen < 0.4 * meant;
  };
  const short = [];
  for (const t of texts) {
    const lines = t.rects.filter((r) => !hidden(t, r));
    const area = lines.reduce((s, r) => s + (r.right - r.left) * (r.bottom - r.top), 0);
    // what is behind the letters themselves: the pixels the probe-coloured text inks, anywhere in the line (the gaps
    // between letters, where a shadow close to them doesn't reach, aren't read against; a comma's few pixels are its
    // own); the middle of the line, where letters sit, only when none is inked at all
    const step = Math.max(1, Math.sqrt(area / 2400));   // about 2,400 samples a text, at most
    const inked = (i) => dist(px(probed, i), px(behind, i)) >= 48, spots = [], band = [];
    for (const r of lines) for (let y = r.top + step / 2; y < r.bottom; y += step) for (let x = r.left + step / 2; x < r.right; x += step) {
      const i = at(x * k, y * k); if (t.masked && masked(i)) continue;
      const mid = Math.abs(y - (r.top + r.bottom) / 2) <= (r.bottom - r.top) / 4;
      const into = inked(i) ? spots : mid ? band : null; if (!into) continue;
      const pixel = px(behind, i), a = behind.data[i + 3] / 255;
      for (const bg of a < 1 && seeThrough ? [over(pixel, a, [0, 0, 0]), over(pixel, a, [255, 255, 255])] : [pixel]) into.push({ ratio: reads(t, bg), bg });
    }
    if (!spots.length) spots.push(...band);
    if (!spots.length) continue;
    spots.sort((p, q) => p.ratio - q.ratio);
    const worst = spots[Math.floor(spots.length * 0.05)], need = t.large ? 3 : 4.5;
    if (worst.ratio < need) short.push({ ...t, ...worst, need });
  }
  if (!short.length) return null;
  short.sort((p, q) => p.ratio / p.need - q.ratio / q.need);
  const s = short[0];
  const colour = hex(s.fg) + (s.alpha < 0.995 ? ` at ${Math.round(s.alpha * 100)}%` : '') + (s.stroke ? `, outlined ${hex(s.stroke.rgb)}` : '');
  return `${s.ratio.toFixed(2)}:1 where ${s.need}:1 is needed: ${s.name}, ${colour} on ${hex(s.bg)}` +
    (short.length > 1 ? ` (and ${short.length - 1} more under their minimum)` : '');
};

// A scrollbar on the screen that the look never drew: lull's own (base.css's, a thin thumb in the surface's quiet colour)
// where the theme or design drew the rest of the window. Information for the author, as contrast is (ADR-0101): lull's
// scrollbar works, so this never judges a theme; it says where one shows that the author may not have seen, since a
// browser in a test hides scrollbars and the app never does. Drawn by the look: a ::-webkit-scrollbar rule of its own
// (or one of its parts) that reaches the element, scrollbar-color or scrollbar-width on it (which lull sets for Firefox
// only), or --scroll-thumb set by the look on it or round it. The look's rules are the design's stylesheet
// (#design-css) and a listener's theme (#user-css), with what they import. Only a bar that takes room is seen: an
// overlay bar (a phone's) shows while scrolling only.
export function plainScrollbars(page) { return page.evaluate(plainScrollbarsNow); }
export const plainScrollbarsNow = () => {
  // a selector list split at its own commas, never those inside :is(...) and the like
  const parts = (list) => {
    const out = []; let depth = 0, from = 0;
    for (let i = 0; i < list.length; i++) {
      const c = list[i];
      if (c === '(' || c === '[') depth++; else if (c === ')' || c === ']') depth--;
      else if (c === ',' && !depth) { out.push(list.slice(from, i).trim()); from = i + 1; }
    }
    out.push(list.slice(from).trim());
    return out;
  };
  const bars = [], tokens = [];
  const walk = (rules) => {
    for (const r of rules) {
      if (r.styleSheet) { try { walk(r.styleSheet.cssRules); } catch { /* another origin's: not the look's */ } continue; }
      if (r instanceof CSSMediaRule && !matchMedia(r.media.mediaText).matches) continue;
      if (r instanceof CSSSupportsRule && !CSS.supports(r.conditionText)) continue;
      if (r.selectorText) {
        for (const p of parts(r.selectorText)) {
          const at = p.indexOf('::-webkit-scrollbar');
          // what the bar belongs to: `x ::-webkit-scrollbar` (a space, or > + ~, before it) is any element in x's place
          if (at >= 0) { const on = p.slice(0, at); bars.push((/(^|[\s>+~])$/.test(on) ? on + '*' : on).trim()); }
          else if (r.style.getPropertyValue('--scroll-thumb')) tokens.push(p);
        }
      }
      if (r.cssRules && !r.selectorText) walk(r.cssRules);
    }
  };
  for (const id of ['design-css', 'user-css']) {
    const sheet = document.getElementById(id)?.sheet;
    if (sheet) try { walk(sheet.cssRules); } catch { /* not readable */ }
  }
  const matches = (el, sel, up) => { try { return up ? !!el.closest(sel) : el.matches(sel); } catch { return false; } };
  const drawn = (el) => {
    const s = getComputedStyle(el);
    if (s.scrollbarWidth === 'none') return true;
    if (s.scrollbarColor !== 'auto' || s.scrollbarWidth !== 'auto') return true;
    return bars.some((sel) => matches(el, sel)) || tokens.some((sel) => matches(el, sel, true));
  };
  const name = (el) => (el.id ? '#' + el.id : el.tagName.toLowerCase() + [...el.classList].map((c) => '.' + c).join(''));
  const found = [];
  const root = document.documentElement;
  const sides = (down, across) => [down && 'down', across && 'across'].filter(Boolean).join(' and ');
  {
    const down = innerWidth - root.clientWidth > 2, across = innerHeight - root.clientHeight > 2;
    if ((down || across) && !drawn(root)) found.push(`the page (${sides(down, across)})`);
  }
  for (const el of document.querySelectorAll('body *')) {
    const s = getComputedStyle(el);
    if (!/auto|scroll|overlay/.test(s.overflowX + s.overflowY)) continue;
    const down = el.offsetWidth - el.clientWidth - parseFloat(s.borderLeftWidth) - parseFloat(s.borderRightWidth) > 2;
    const across = el.offsetHeight - el.clientHeight - parseFloat(s.borderTopWidth) - parseFloat(s.borderBottomWidth) > 2;
    if (!down && !across) continue;
    if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) continue;
    const r = el.getBoundingClientRect();
    if (r.right <= 0 || r.bottom <= 0 || r.left >= innerWidth || r.top >= innerHeight) continue;
    if (!drawn(el)) found.push(`${name(el)} (${sides(down, across)})`);
  }
  if (!found.length) return null;
  const shown = found.slice(0, 4).join(', ');
  return `lull's own scrollbar, not the look's: ${shown}${found.length > 4 ? ` and ${found.length - 4} more` : ''}`;
};
