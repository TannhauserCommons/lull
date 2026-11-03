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
  var ro = new ResizeObserver(function (entries) {
    entries.forEach(function (e) {
      if (e.target === main) mainW = e.contentRect.width;
      else short = e.contentRect.height < SHORT;
    });
    var next = classOf(mainW);
    toggleAttr('data-height', short ? 'short' : 'tall');
    apply();
    if (next !== size) { size = next; toggleAttr('data-size', size); apply(); App.emit('layout:size', size); }
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
     out with display: contents hands its parts to the row around it. Rows fit again when they or their parts resize
     (ResizeObserver: a web font arriving with wider letters than the fallback's, a label changing), and on
     App.layout.refit() after their content changes (a new song, the status tray: parts drawn since are watched from
     then on). */
  function fitRowOf(part) {
    for (var a = part.parentElement; a; a = a.parentElement) if (a.hasAttribute('data-fit-row') && getComputedStyle(a).display !== 'contents') return a;
    return null;
  }
  /* the row's content is wider than the row: its scroll width, or, for a row that packs toward its end (whose overflow
     the scroll width doesn't count), its children's widths and gaps */
  function overflows(row) {
    if (row.scrollWidth > row.clientWidth + 1) return true;
    var cs = getComputedStyle(row); if (cs.display.indexOf('flex') < 0) return false;
    var need = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight), gap = parseFloat(cs.columnGap) || 0, n = 0;
    Array.prototype.forEach.call(row.children, function (c) {
      var s = getComputedStyle(c); if (s.display === 'none' || s.position === 'absolute' || s.position === 'fixed') return;
      need += c.getBoundingClientRect().width + parseFloat(s.marginLeft) + parseFloat(s.marginRight); n++;
    });
    return need + gap * Math.max(0, n - 1) > row.clientWidth + 1;
  }
  function fit(row) {
    if (!row.getClientRects().length || getComputedStyle(row).display === 'contents') return;
    /* a part a design placed outside the row's flow (glasshouse floats the tray into the rail) takes no room in it */
    var parts = $$('[data-fit]', row).filter(function (p) {
      var pos = getComputedStyle(p).position; return fitRowOf(p) === row && pos !== 'fixed' && pos !== 'absolute';
    });
    parts.forEach(function (p) { p.removeAttribute('data-fit-out'); });
    var orders = parts.map(function (p) { return +p.getAttribute('data-fit'); })
      .filter(function (o, i, all) { return all.indexOf(o) === i; }).sort(function (a, b) { return a - b; });
    for (var i = 0; i < orders.length && overflows(row); i++) {
      parts.forEach(function (p) { if (+p.getAttribute('data-fit') === orders[i]) p.setAttribute('data-fit-out', ''); });
    }
  }
  /* a row fits again when it resizes, and when what is in it does without the row resizing (a web font arriving with
     wider letters than the fallback's, a label that changed): the observer watches each row, its children and its
     parts, and fits the row they belong to. Outer rows first (document order): an inner row's width follows from what
     its outer row gave way */
  function inOrder(a, b) { return a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1; }
  var fitting = new ResizeObserver(function (entries) {
    var rows = [];
    entries.forEach(function (e) {
      var own = e.target.hasAttribute('data-fit-row'), row = own ? e.target : fitRowOf(e.target);
      if (!row || rows.indexOf(row) >= 0) return;
      /* a part or child that resized in a row that still fits needs nothing when it is a part that gave way (fit's own
         doing) or nothing in the row has given way (all shown, nothing to bring back). Read after layout: no cost */
      if (!own && !overflows(row) && (e.target.hasAttribute('data-fit-out') || !$('[data-fit-out]', row))) return;
      rows.push(row);
    });
    rows.sort(inOrder).forEach(fit);
  });
  var watched = new WeakSet();
  function watch(el) { if (!watched.has(el)) { watched.add(el); fitting.observe(el); } }
  /* every row is watched, a box-less one too (display: contents at some widths), since the observer reports it when it
     gets a box; a row or part drawn since the last call (the song's chips) is watched from now on */
  App.layout.refit = function () {
    $$('[data-fit-row]').forEach(function (r) {
      watch(r); Array.prototype.forEach.call(r.children, watch); $$('[data-fit]', r).forEach(watch);
      fit(r);
    });
  };
  App.on('ready', App.layout.refit);
  App.on('design', function () { fitting.disconnect(); watched = new WeakSet(); App.layout.refit(); });
  if (document.fonts) document.fonts.ready.then(App.layout.refit);
  App.layout.refit();

  apply();
})();
