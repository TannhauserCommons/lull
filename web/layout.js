/* Layout (ADR-0007): layouts reveal more as the window grows, and the listener can adjust them.
   - Size classes come from the main area's own width: .main is the `main` container for CSS (@container main).
     A ResizeObserver mirrors the class onto .app[data-size] (compact | regular | wide | xwide | ultra) and the window's
     height onto .app[data-height] (short below 760 px), for scripts and designs. No layout is read in a resize loop.
   - App.layout holds the listener's choices, per device: localStorage 'lull:layout' = { panelWidth, panelCollapsed,
     nowColumn, nowWidth, searchSplit, panelSplit }. get(), set(patch) (emits 'layout'), reset(),
     size(). Widths ride on custom properties of .app (--ctx, --now-w), so the columns resize with CSS alone.
   - Resize handles [data-resize="panel|now"] (role separator): a pointer drag throttled to animation frames, arrow
     keys (Shift for bigger steps), Home / End for the limits, double-click for the default. While a drag runs, its
     width (`live`) is the one shown, through --ctx-drag / --now-drag on the elements that read it (components.css
     says why), so the column follows the pointer each frame; it is saved when the drag ends.
   - The Now Playing column (.nowcol, from `wide`): beside the main area in the same grid cell, so no design's shell
     grid changes. App.nowColumn (now.js) fills it. Closed on /now, which shows all of it already.
     It and the docked side panel follow one rule (docs/context/web.md, Layout):
     - the column is offered when the main area stays `wide` beside it with the panel at its strip: the window's width
       alone decides, so its button never comes and goes as the panel opens or closes;
     - both are open when the main area stays `wide` beside both; otherwise opening one closes the other (the column's
       choice is forgotten, the panel collapses to its strip), and closing it brings nothing back. */
(function () {
  'use strict';
  var $ = App.$, $$ = App.$$, clamp = App.clamp;
  var SIZES = [['ultra', 2200], ['xwide', 1600], ['wide', 1100], ['regular', 640], ['compact', 0]];
  var SHORT = 760;
  var DEFAULTS = { panelWidth: null, panelCollapsed: false, nowColumn: false, nowWidth: null, searchSplit: false, panelSplit: false };
  var LIMITS = { panel: [280, 560], now: [300, 560] };
  var NOW_DEFAULT = 360, STRIP = 60, GAP = 16, KEY_STEP = 16;
  var app = $('#app'), main = $('.main');
  var live = { panel: null, now: null };          // a width being dragged: shown at once, saved when the drag ends
  var DRAG = { panel: '--ctx-drag', now: '--now-drag' };
  function readers(kind) { return kind === 'panel' ? [app, $('#ctx')] : [main, $('#nowcol')]; }

  /* ---------- the stored choices: anything unknown or out of range falls back to the default ---------- */
  function width(v, kind) { return typeof v === 'number' && isFinite(v) ? clamp(Math.round(v), LIMITS[kind][0], LIMITS[kind][1]) : null; }
  function sanitize(s) {
    s = s && typeof s === 'object' ? s : {};
    return {
      panelWidth: width(s.panelWidth, 'panel'), panelCollapsed: s.panelCollapsed === true,
      nowColumn: s.nowColumn === 'open' || s.nowColumn === 'strip' ? s.nowColumn : false, nowWidth: width(s.nowWidth, 'now'),
      searchSplit: s.searchSplit === true, panelSplit: s.panelSplit === true
    };
  }
  var stored = App.storage('layout');
  if (!stored && App.storage('ctxCollapsed') === true) stored = { panelCollapsed: true };   // the panel's older switch
  try { localStorage.removeItem('lull:ctxCollapsed'); } catch (e) { /* private mode */ }
  var L = sanitize(stored);

  var size = null, mainW = 0, short = false;
  App.layout = {
    get: function () { return Object.assign({}, L); },
    set: function (patch) {
      L = sanitize(Object.assign({}, L, patch));
      App.storage('layout', L);
      apply(); App.emit('layout', App.layout.get());
    },
    /* Settings > Appearance "Reset layout": defaults at once, no reload; the screen re-renders (Home order, search) */
    reset: function () {
      L = sanitize(DEFAULTS);
      try { localStorage.removeItem('lull:layout'); } catch (e) { /* private mode */ }
      apply(); App.emit('layout', App.layout.get()); App.refresh();
    },
    /* the main area's size class; at least(name) compares against it */
    size: function () { return size || 'compact'; },
    at: function (name) { return rank(App.layout.size()) >= rank(name); },
    limits: LIMITS, defaults: { now: NOW_DEFAULT }
  };
  function rank(name) { for (var i = 0; i < SIZES.length; i++) if (SIZES[i][0] === name) return SIZES.length - i; return 0; }
  function classOf(w) { for (var i = 0; i < SIZES.length; i++) if (w >= SIZES[i][1]) return SIZES[i][0]; return 'compact'; }

  /* ---------- applying: attributes and custom properties on .app, nothing measured ---------- */
  function docked() { return app.getAttribute('data-ctx') === 'docked'; }
  function nowShown() { var v = app.getAttribute('data-nowcol'); return v === 'open' || v === 'strip'; }
  function nowOpen() { return live.now || L.nowWidth || NOW_DEFAULT; }
  function nowWidth() { return L.nowColumn === 'strip' ? STRIP : L.nowWidth || NOW_DEFAULT; }
  function panelExpanded() { return docked() && !L.panelCollapsed; }
  /* the main area's width with the Now column closed and the panel at its strip (no feedback loop: it doesn't change
     as either opens), and what the expanded panel takes from it */
  function freeWidth() {
    var shownW = nowShown() ? (app.getAttribute('data-nowcol') === 'strip' ? STRIP : nowOpen()) + GAP : 0;
    return mainW + shownW + (panelExpanded() ? current('panel') - STRIP : 0);
  }
  function wideAt(w) { return rank(classOf(w)) >= rank('wide'); }
  function bothFit() { return wideAt(freeWidth() - (current('panel') - STRIP)); }
  function apply() {
    var offered = wideAt(freeWidth());
    var show = offered && L.nowColumn && app.getAttribute('data-screen') !== 'now' && (!panelExpanded() || bothFit());
    toggleAttr('data-nowcol-ok', offered ? '' : null);
    toggleAttr('data-nowcol', show ? L.nowColumn : null);
    prop(app, '--now-w', nowWidth() + 'px');
    /* the panel: its stored width, or the icon strip while docked and collapsed */
    var ctx = docked() && L.panelCollapsed ? STRIP : L.panelWidth;
    prop(app, '--ctx', ctx ? ctx + 'px' : '');
    toggleAttr('data-panel', docked() && L.panelCollapsed ? 'strip' : null);
    toggleAttr('data-panel-split', L.panelSplit && App.layout.at('xwide') ? '' : null);
    showLive('panel'); showLive('now');
    $$('[data-resize]').forEach(function (h) {
      var kind = h.getAttribute('data-resize');
      h.setAttribute('aria-valuemin', LIMITS[kind][0]); h.setAttribute('aria-valuemax', LIMITS[kind][1]);
      h.setAttribute('aria-valuenow', current(kind));
    });
    $$('[data-act="now-column"]').forEach(function (b) { b.setAttribute('aria-pressed', String(!!show)); });
    $$('[data-act="panel-split"]').forEach(function (b) { b.setAttribute('aria-pressed', String(L.panelSplit)); });
  }
  function showLive(kind) {
    readers(kind).forEach(function (el) { if (el) prop(el, DRAG[kind], live[kind] == null ? '' : live[kind] + 'px'); });
  }
  /* custom properties written only when they change: apply() runs on every frame of a drag (the main area resizes),
     and rewriting --ctx or --now-w, even unchanged, could restyle the whole page */
  function prop(el, name, v) {
    if (el.style.getPropertyValue(name) === v) return;
    if (v) el.style.setProperty(name, v); else el.style.removeProperty(name);
  }
  function toggleAttr(name, value) {
    if (value == null) { if (app.hasAttribute(name)) app.removeAttribute(name); }
    else if (app.getAttribute(name) !== value) app.setAttribute(name, value);
  }
  /* a column's width as shown: the stored one, or the design's own (--ctx, read once per design) until one is set */
  var designCtx = null;
  function current(kind) {
    if (kind === 'now') return nowOpen();
    if (live.panel || L.panelWidth) return live.panel || L.panelWidth;
    if (designCtx == null) designCtx = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--ctx'), 10) || LIMITS.panel[0];
    return designCtx;
  }
  /* read again once the design's stylesheet is in (boot), and whenever the design changes */
  App.on('ready', function () { designCtx = null; apply(); });
  App.on('design', function () { designCtx = null; apply(); });

  /* ---------- size classes: the main area's width and the window's height, as ResizeObserver reports them ---------- */
  /* the states a resize can switch, which change how the rows are laid out */
  function states() { return ['data-size', 'data-height', 'data-nowcol', 'data-nowcol-ok', 'data-panel', 'data-panel-split'].map(function (n) { return app.getAttribute(n); }).join('|'); }
  var ro = new ResizeObserver(function (entries) {
    var was = states();
    entries.forEach(function (e) {
      if (e.target === main) mainW = e.contentRect.width;
      else short = e.contentRect.height < SHORT;
    });
    var next = classOf(mainW);
    toggleAttr('data-height', short ? 'short' : 'tall');
    apply();
    if (next !== size) { size = next; toggleAttr('data-size', size); apply(); App.emit('layout:size', size); }
    /* a state switched lays the rows out anew: fit them now, in this frame. Left to the rows' own observer, the change
       reaches it a round later, which a frame can end before, and the header stuck out for a frame (CI, Skydeck, 900 px) */
    if (states() !== was && App.layout.refit) App.layout.refit();
  });
  ro.observe(main); ro.observe(document.documentElement);
  /* the panel docking (app.js) and the route (/now hides the column) change what applies */
  new MutationObserver(apply).observe(app, { attributes: true, attributeFilter: ['data-ctx', 'data-screen'] });

  /* ---------- resize handles ---------- */
  function save(kind, w) { App.layout.set(kind === 'panel' ? { panelWidth: w } : { nowWidth: w }); }
  document.addEventListener('pointerdown', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h || e.button !== 0) return;
    e.preventDefault();
    var kind = h.getAttribute('data-resize'), x0 = e.clientX, w0 = current(kind), w = w0, frame = 0;
    h.setPointerCapture(e.pointerId); app.classList.add('is-resizing');
    /* the handle sits on the column's left edge: dragging left widens it */
    function move(ev) {
      w = clamp(w0 - (ev.clientX - x0), LIMITS[kind][0], LIMITS[kind][1]);
      if (!frame) frame = requestAnimationFrame(function () { frame = 0; live[kind] = w; apply(); });
    }
    function end() {
      h.removeEventListener('pointermove', move); h.removeEventListener('pointerup', end); h.removeEventListener('pointercancel', end);
      cancelAnimationFrame(frame); app.classList.remove('is-resizing');
      live[kind] = null;
      if (w !== w0) save(kind, w); else apply();
    }
    h.addEventListener('pointermove', move); h.addEventListener('pointerup', end); h.addEventListener('pointercancel', end);
  });
  document.addEventListener('dblclick', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h) return;
    App.layout.set(h.getAttribute('data-resize') === 'panel' ? { panelWidth: null } : { nowWidth: null });
  });
  document.addEventListener('keydown', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h) return;
    var kind = h.getAttribute('data-resize'), w = current(kind), step = KEY_STEP * (e.shiftKey ? 4 : 1), lim = LIMITS[kind];
    var next = { ArrowLeft: w + step, ArrowRight: w - step, Home: lim[0], End: lim[1] }[e.key];
    if (next == null) return;
    e.preventDefault();
    save(kind, clamp(next, lim[0], lim[1]));
    App.announce((kind === 'panel' ? 'Panel' : 'Now Playing column') + ' ' + current(kind) + ' pixels wide');
  });

  /* ---------- the Now Playing column ---------- */
  function setNow(v) { App.layout.set({ nowColumn: v }); }
  /* opening one of the two closes the other when the main area wouldn't stay wide beside both (the rule above);
     app.js calls this as the panel opens docked */
  App.layout.makeRoom = function (opening) {
    if (opening === 'panel' && L.nowColumn && !bothFit()) setNow(false);
    if (opening === 'now' && panelExpanded() && !bothFit()) App.closePanel();
  };
  App.action('now-column', function () {
    if (nowShown()) { setNow(false); return; }
    App.layout.makeRoom('now');
    setNow('open');
  });
  App.action('nowcol-collapse', function () { setNow('strip'); });
  App.action('nowcol-expand', function () { App.layout.makeRoom('now'); setNow('open'); });
  var lastNow = null;
  new MutationObserver(function () {
    var v = app.getAttribute('data-nowcol');
    if (v === 'open' && lastNow !== 'open' && App.nowColumn) App.nowColumn.render($('#nowcol-body'));
    if ((v === 'open') !== (lastNow === 'open')) App.emit('nowcol', v === 'open');   // the panel's tabs follow (app.js)
    lastNow = v;
  }).observe(app, { attributes: true, attributeFilter: ['data-nowcol'] });

  App.action('panel-split', function () { App.layout.set({ panelSplit: !L.panelSplit }); App.refreshPanel(); });

  /* ---------- rows that fit themselves ----------
     The header, the player bar and the rows inside them lay themselves out from the room they have, never from the
     window's width, so a docked panel, the Now column, the text size or a longer title count as much as the window. A
     row is [data-fit-row]; a part that can give way is [data-fit="<order>"] (1 gives way first; parts of one order go
     together). When a row's content needs more than its width, parts give way in order (they get [data-fit-out]; the
     component's CSS says what that means: hidden and in the "…" menu, a pill without its label, the compact tray) until
     it fits; with room again they come back. A part belongs to its nearest row with a box, so a design that lays a row
     out with display: contents hands its parts to the row around it. A row and the rows inside it fit as one, so they
     always end as a fresh fit of them would. Rows fit again when they or their parts resize (ResizeObserver: a label
     that grew), when a font loads (a web font arriving with other widths than the fallback's), and on
     App.layout.refit() after their content changes (a new song, the status tray: parts drawn since are watched from
     then on). */
  function fitRowOf(part) {
    for (var a = part.parentElement; a; a = a.parentElement) if (a.hasAttribute('data-fit-row') && getComputedStyle(a).display !== 'contents') return a;
    return null;
  }
  /* the row's content is wider than the row: its scroll width, or any box laid out in it (through display: contents)
     reaching past its content box on either side, into its padding or beyond (a row that packs toward its end
     overflows at its start, which the scroll width doesn't count; content in the end padding counts in the scroll
     width only from the padding's edge), or, in a flex row, its children's widths and gaps. Content clipped counts as
     content that doesn't fit: a box laid out in the row past the edge of a box around it that clips (overflow hidden or
     clip, up to the row it fits with: the song's chips in the bar's song side), and a box in the row that clips content
     of its own (the song side holding the Private pill while nothing plays). A box that cuts its text short on purpose
     (text-overflow: ellipsis: a long title) or scrolls is not clipping anything that should give way */
  function clips(s) { return (s.overflowX === 'hidden' || s.overflowX === 'clip') && s.textOverflow !== 'ellipsis'; }
  /* words that keep a readable width ([data-fit-keep], its --fit-keep: the song's title and artist): narrower than that,
     or than all of them when they are shorter, they crowd their row, and its parts give way to make room */
  function crowded(k) {
    if (!k.getClientRects().length) return false;
    var full = [k].concat($$('*', k)).reduce(function (w, e) { return Math.max(w, e.scrollWidth); }, 0);
    return k.clientWidth + 1 < Math.min(full, parseFloat(getComputedStyle(k).getPropertyValue('--fit-keep')) || 0);
  }
  /* a part is shown whole or not at all: one whose words are cut short (text-overflow: ellipsis, in it or on it: the
     place "Playing from" names) doesn't fit, so what gives way before it does, and then it does. A field's own text
     (the search as typed) is the listener's, not the part's */
  function cut(p) {
    if (p.hasAttribute('data-fit-out')) return false;
    return [p].concat($$('*', p)).some(function (e) {
      if (e.nodeName === 'INPUT' || e.nodeName === 'TEXTAREA' || e.scrollWidth < e.clientWidth) return false;
      var s = getComputedStyle(e); if (s.textOverflow !== 'ellipsis') return false;
      if (e.scrollWidth > e.clientWidth) return true;
      /* the browser draws the ellipsis for any overflow, a fraction of a pixel too, which whole-pixel widths round
         away: then the words' own width, to the fraction, read with them let out for the measure (an ellipsis cuts
         the boxes a range reports) */
      var r = e.getBoundingClientRect(), inner = r.width - parseFloat(s.paddingLeft) - parseFloat(s.paddingRight) - parseFloat(s.borderLeftWidth) - parseFloat(s.borderRightWidth);
      var was = e.style.getPropertyValue('overflow'), prio = e.style.getPropertyPriority('overflow');
      e.style.setProperty('overflow', 'visible', 'important');
      var range = document.createRange(); range.selectNodeContents(e);
      var rects = range.getClientRects(), right = r.left;
      for (var i = 0; i < rects.length; i++) right = Math.max(right, rects[i].right);
      if (was) e.style.setProperty('overflow', was, prio); else e.style.removeProperty('overflow');
      return right - r.left - parseFloat(s.paddingLeft) - parseFloat(s.borderLeftWidth) > inner + .01;
    });
  }
  /* a row's content against its room: `over`, how far it runs past (0 when it fits; overflows() allows a pixel), and
     `need`, the width the row takes with it (its boxes through display: contents, their margins, its gaps, its padding
     and border: Skydeck's dock is outlined) */
  function measure(row) {
    var f = parseFloat, cs = getComputedStyle(row), r = row.getBoundingClientRect(),
      padL = f(cs.paddingLeft) + f(cs.borderLeftWidth), padR = f(cs.paddingRight) + f(cs.borderRightWidth),
      left = r.left + padL, right = r.right - padR, over = Math.max(0, row.scrollWidth - row.clientWidth), sum = 0, n = 0;
    for (var a = row.parentElement, top = rootOf(row); a && row !== top; a = a.parentElement) {
      var as = getComputedStyle(a);
      if (clips(as)) { var ar = a.getBoundingClientRect(); left = Math.max(left, ar.left + f(as.borderLeftWidth)); right = Math.min(right, ar.right - f(as.borderRightWidth)); }
      if (a === top) break;
    }
    (function walk(box) {
      Array.prototype.forEach.call(box.children, function (c) {
        var s = getComputedStyle(c);
        if (s.display === 'contents') { walk(c); return; }
        if (s.display === 'none' || s.position === 'absolute' || s.position === 'fixed') return;
        var b = c.getBoundingClientRect();
        if (b.width) over = Math.max(over, b.right - right, left - b.left);
        if (clips(s)) over = Math.max(over, c.scrollWidth - c.clientWidth);
        sum += b.width + f(s.marginLeft) + f(s.marginRight); n++;
      });
    })(row);
    sum += (f(cs.columnGap) || 0) * Math.max(0, n - 1);
    if (cs.display.indexOf('flex') >= 0) over = Math.max(over, sum - (right - left));
    return { over: over, need: Math.ceil(sum + padL + padR) };
  }
  function overflows(row) { return measure(row).over > 1; }
  /* the parts that give way first are out (orders 1..k, together); fit moves k from where the row stands: further while
     the row overflows, back while it fits with one order more shown. Returns whether k moved */
  function fit(row) {
    if (!row.getClientRects().length || getComputedStyle(row).display === 'contents') return false;
    /* a part a design placed outside the row's flow (glasshouse floats the tray into the rail) takes no room in it */
    var parts = $$('[data-fit]', row).filter(function (p) {
      var pos = getComputedStyle(p).position; return fitRowOf(p) === row && pos !== 'fixed' && pos !== 'absolute';
    });
    var orders = parts.map(function (p) { return +p.getAttribute('data-fit'); })
      .filter(function (o, i, all) { return all.indexOf(o) === i; }).sort(function (a, b) { return a - b; });
    /* the row says what gave way in it (the orders, and "all" once every one did), at each step, for a style that
       follows a part out (lull's cache meter, the search's padding) and so takes its room with it as the row is measured:
       a :has([data-fit-out]) would restyle the whole page at every step in a design with a :has() on the shell */
    function give(i, out) {
      parts.forEach(function (p) { if (+p.getAttribute('data-fit') === orders[i]) p.toggleAttribute('data-fit-out', out); });
      var off = orders.filter(function (o) { return parts.some(function (p) { return +p.getAttribute('data-fit') === o && p.hasAttribute('data-fit-out'); }); }),
        gave = off.join(' ') + (off.length && off.length === orders.length ? ' all' : '');
      if (row.getAttribute('data-fit-gave') !== gave) row.setAttribute('data-fit-gave', gave);
    }
    var k = 0, was;
    while (k < orders.length && parts.every(function (p) { return +p.getAttribute('data-fit') !== orders[k] || p.hasAttribute('data-fit-out'); })) k++;
    for (var i = 0; i < orders.length; i++) give(i, i < k);
    was = k;
    var keep = $$('[data-fit-keep]', row).filter(function (k) { return fitRowOf(k) === row; });
    function over() { return overflows(row) || parts.some(cut) || keep.some(crowded); }
    if (over()) { while (k < orders.length && over()) give(k++, true); }
    else while (k > 0) { give(k - 1, false); if (over()) { give(k - 1, true); break; } k--; }
    return k !== was;
  }
  /* a row and the rows inside it fit together: from all shown, outer rows first (document order: an inner row's width
     follows from what its outer row gave way), again until none moves (what an inner row gave way can give its outer row
     room back). The same start every time, so the same room always ends the same way, however it was reached */
  function rootOf(row) { for (var a = row.parentElement; a; a = a.parentElement) if (a.hasAttribute('data-fit-row')) row = a; return row; }
  function settle(root) {
    var rows = [root].concat($$('[data-fit-row]', root));
    function passes() {
      for (var pass = 0, moved = true; moved && pass < 4; pass++) {
        moved = false;
        rows.forEach(function (r) { if (fit(r)) moved = true; });
      }
    }
    /* in whole pixels, as every width the sides are tried at: the search below halves between whole widths */
    var side = root.hasAttribute('data-fit-sides') && $('[data-fit-side]', root), most = side ? Math.floor(sideMost(root)) : 0;
    if (side) root.style.setProperty('--fit-side', most + 'px');
    rows.forEach(function (r) { $$('[data-fit-out]', r).forEach(function (p) { p.removeAttribute('data-fit-out'); }); });
    passes();
    if (side) {
      /* what gave way with the most room; at the tools' width the song's side may have to give way more (its badges),
         so the sides widen by as much as those parts' rows then run over with them shown, to the narrowest width at
         which nothing gives way that the most room kept (by halves from there, where a row took less than all of it) */
      var kept = $$('[data-fit-out]', root),
        min = Math.ceil(parseFloat(getComputedStyle(root).getPropertyValue('--fit-side-min')) || 0),
        lo = Math.max(0, Math.min(most, Math.max(measure(side).need, min)));
      function more() { return $$('[data-fit-out]', root).filter(function (p) { return kept.indexOf(p) < 0; }); }
      function at(w) { root.style.setProperty('--fit-side', w + 'px'); passes(); return more().length > 0; }
      if (at(lo)) {
        var hit = [];
        more().forEach(function (p) { var r = fitRowOf(p); if (hit.indexOf(r) < 0) hit.push(r); p.removeAttribute('data-fit-out'); });
        var hi = Math.min(most, Math.ceil(lo + Math.max.apply(Math, hit.map(function (r) { return measure(r).over; }))));
        if (at(hi)) { lo = hi; hi = most; while (hi - lo > 1) { var mid = Math.floor((lo + hi) / 2); if (at(mid)) lo = mid; else hi = mid; } at(hi); }
      }
    }
  }
  /* a row in three columns with mirrored sides ([data-fit-sides]: the player bar) has sides exactly as wide as its side
     row needs ([data-fit-side]: the tools), once that has given way to fit the most room the middle can spare
     (--fit-centre-min kept for it). Its CSS takes the width from --fit-side for both sides, so the middle gets every
     pixel left and stays centred: no gap beside the tools at any width. A design that lays the row out its own way
     doesn't use --fit-side, and nothing changes for it */
  function sideMost(root) {
    var cs = getComputedStyle(root), inner = root.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight),
      tracks = cs.gridTemplateColumns.split(' ').map(parseFloat),
      /* the gap as laid out: a design may give it as min() or a percentage, which getComputedStyle hands back unresolved;
         the tracks fill the row (the middle one is 1fr), so what they leave is the gaps */
      gap = tracks.length === 3 && tracks.every(isFinite) ? (inner - tracks[0] - tracks[1] - tracks[2]) / 2 : parseFloat(cs.columnGap) || 0;
    return Math.max(0, (inner - (parseFloat(cs.getPropertyValue('--fit-centre-min')) || 0) - 2 * gap) / 2);
  }
  /* rows fit again when they resize, and when what is in them does without the row resizing (a web font arriving with
     wider letters than the fallback's, a label that changed): the observer watches each row, its children and its
     parts, and settles the rows they belong to */
  var fitting = new ResizeObserver(function (entries) {
    var roots = [];
    entries.forEach(function (e) {
      var own = e.target.hasAttribute('data-fit-row'), row = own ? e.target : fitRowOf(e.target);
      if (!row) return;
      /* a part or child that resized in a row that still fits needs nothing when it is a part that gave way (fit's own
         doing) or nothing in the row has given way (all shown, nothing to bring back). Read after layout: no cost */
      if (!own && !overflows(row) && !(e.target.hasAttribute('data-fit') && cut(e.target)) && (e.target.hasAttribute('data-fit-out') || !$('[data-fit-out]', row))) return;
      var root = rootOf(row); if (roots.indexOf(root) < 0) roots.push(root);
    });
    roots.forEach(settle);
  });
  var watched = new WeakSet();
  function watch(el) { if (!watched.has(el)) { watched.add(el); fitting.observe(el); } }
  /* every row is watched, a box-less one too (display: contents at some widths), since the observer reports it when it
     gets a box; a row or part drawn since the last call (the song's chips) is watched from now on */
  App.layout.refit = function () {
    $$('[data-fit-row]').forEach(function (r) { watch(r); Array.prototype.forEach.call(r.children, watch); $$('[data-fit]', r).forEach(watch); });
    $$('[data-fit-row]').filter(function (r) { return rootOf(r) === r; }).forEach(settle);
  };
  App.on('ready', App.layout.refit);
  App.on('design', function () { fitting.disconnect(); watched = new WeakSet(); App.layout.refit(); });
  /* a font that arrives changes the words' widths in place, and a part that gave way shows no box to tell the observer it
     would fit now: every font that loads (a design's own, on first use) fits the rows again */
  if (document.fonts) { document.fonts.ready.then(App.layout.refit); document.fonts.addEventListener('loadingdone', App.layout.refit); }
  /* a stylesheet that changes (a theme reloaded or edited live, Theme Studio's draft, a screen's sheet arriving) can
     widen the words inside boxes that keep their size: a cut part's or a kept line's, beside sides the fitter sized
     itself. No box the observer watches resizes, so every row fits again once the sheet is in: at once for a sheet's
     text, on load for what it imports. Only <head> holds the sheets, so nothing else on the page pays */
  function sheet(n) { return n && (n.nodeName === 'STYLE' || n.nodeName === 'LINK'); }
  if (window.MutationObserver && document.head) new MutationObserver(function (records) {
    if (records.some(function (r) { return sheet(r.target) || sheet(r.target.parentNode) || [].some.call(r.addedNodes, sheet) || [].some.call(r.removedNodes, sheet); })) App.layout.refit();
  }).observe(document.head, { childList: true, subtree: true, characterData: true });
  document.addEventListener('load', function (e) { if (sheet(e.target)) App.layout.refit(); }, true);
  App.layout.refit();

  apply();
})();
