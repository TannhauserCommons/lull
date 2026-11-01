/* Layout (ADR-0007): layouts reveal more as the window grows, and the listener can adjust them.
   - Size classes come from the main area's own width: .main is the `main` container for CSS (@container main).
     A ResizeObserver mirrors the class onto .app[data-size] (compact | regular | wide | xwide | ultra) and the window's
     height onto .app[data-height] (short below 760 px), for scripts and designs. No layout is read in a resize loop.
   - App.layout holds the listener's choices, per device: localStorage 'lull:layout' = { panelWidth, panelCollapsed,
     nowColumn, nowWidth, searchSplit, panelSplit, homeOrder, homeHidden }. get(), set(patch) (emits 'layout'), reset(),
     size(). Widths ride on custom properties of .app (--ctx, --now-w), so the columns resize with CSS alone.
   - Resize handles [data-resize="panel|now"] (role separator): a pointer drag throttled to animation frames, arrow
     keys (Shift for bigger steps), Home / End for the limits, double-click for the default. Saved when the drag ends.
   - The Now Playing column (.nowcol, from `wide`): beside the main area in the same grid cell, so no design's shell
     grid changes. It is offered when the main area would stay `wide` with it open, so opening it never takes itself
     away; App.nowColumn (now.js) fills it. Closed on /now, which shows all of it already. */
(function () {
  'use strict';
  var $ = App.$, $$ = App.$$, clamp = App.clamp;
  var SIZES = [['ultra', 2200], ['xwide', 1600], ['wide', 1100], ['regular', 640], ['compact', 0]];
  var SHORT = 760;
  var DEFAULTS = { panelWidth: null, panelCollapsed: false, nowColumn: false, nowWidth: null, searchSplit: false, panelSplit: false,
    homeOrder: null, homeHidden: [] };
  var LIMITS = { panel: [280, 560], now: [300, 560] };
  var NOW_DEFAULT = 360, STRIP = 60, GAP = 16, KEY_STEP = 16;
  var app = $('#app'), main = $('.main');

  /* ---------- the stored choices: anything unknown or out of range falls back to the default ---------- */
  function width(v, kind) { return typeof v === 'number' && isFinite(v) ? clamp(Math.round(v), LIMITS[kind][0], LIMITS[kind][1]) : null; }
  function ids(v) { return Array.isArray(v) ? v.filter(function (x) { return typeof x === 'string'; }) : null; }
  function sanitize(s) {
    s = s && typeof s === 'object' ? s : {};
    return {
      panelWidth: width(s.panelWidth, 'panel'), panelCollapsed: s.panelCollapsed === true,
      nowColumn: s.nowColumn === 'open' || s.nowColumn === 'strip' ? s.nowColumn : false, nowWidth: width(s.nowWidth, 'now'),
      searchSplit: s.searchSplit === true, panelSplit: s.panelSplit === true,
      homeOrder: ids(s.homeOrder), homeHidden: ids(s.homeHidden) || []
    };
  }
  var stored = App.storage('layout');
  if (!stored && App.storage('ctxCollapsed') === true) stored = { panelCollapsed: true };   // the panel's older switch
  try { localStorage.removeItem('lull:ctxCollapsed'); } catch (e) { /* private mode */ }
  var L = sanitize(stored);

  var size = null, mainW = 0, short = false;
  App.layout = {
    get: function () { return Object.assign({}, L, { homeHidden: L.homeHidden.slice(), homeOrder: L.homeOrder && L.homeOrder.slice() }); },
    set: function (patch) {
      L = sanitize(Object.assign({}, L, patch));
      App.storage('layout', L);
      apply(); App.emit('layout', App.layout.get());
    },
    reset: function () {
      L = sanitize(DEFAULTS);
      try { localStorage.removeItem('lull:layout'); } catch (e) { /* private mode */ }
      apply(); App.emit('layout', App.layout.get());
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
  function nowWidth() { return L.nowColumn === 'strip' ? STRIP : L.nowWidth || NOW_DEFAULT; }
  function apply() {
    /* the main area's width without the Now column decides whether the column is offered (no feedback loop) */
    var shownW = nowShown() ? (app.getAttribute('data-nowcol') === 'strip' ? STRIP : L.nowWidth || NOW_DEFAULT) + GAP : 0;
    var offered = rank(classOf(mainW + shownW)) >= rank('wide');
    var show = offered && L.nowColumn && app.getAttribute('data-screen') !== 'now';
    toggleAttr('data-nowcol-ok', offered ? '' : null);
    toggleAttr('data-nowcol', show ? L.nowColumn : null);
    app.style.setProperty('--now-w', nowWidth() + 'px');
    /* the panel: its stored width, or the icon strip while docked and collapsed */
    var ctx = docked() && L.panelCollapsed ? STRIP : L.panelWidth;
    if (ctx) app.style.setProperty('--ctx', ctx + 'px'); else app.style.removeProperty('--ctx');
    toggleAttr('data-panel', docked() && L.panelCollapsed ? 'strip' : null);
    toggleAttr('data-panel-split', L.panelSplit && App.layout.at('xwide') ? '' : null);
    $$('[data-resize]').forEach(function (h) {
      var kind = h.getAttribute('data-resize');
      h.setAttribute('aria-valuemin', LIMITS[kind][0]); h.setAttribute('aria-valuemax', LIMITS[kind][1]);
      h.setAttribute('aria-valuenow', current(kind));
    });
    $$('[data-act="now-column"]').forEach(function (b) { b.setAttribute('aria-pressed', String(!!show)); });
    $$('[data-act="panel-split"]').forEach(function (b) { b.setAttribute('aria-pressed', String(L.panelSplit)); });
  }
  function toggleAttr(name, value) {
    if (value == null) { if (app.hasAttribute(name)) app.removeAttribute(name); }
    else if (app.getAttribute(name) !== value) app.setAttribute(name, value);
  }
  /* a column's width as shown: the stored one, or the design's own (--ctx, read once per design) until one is set */
  var designCtx = null;
  function current(kind) {
    if (kind === 'now') return L.nowWidth || NOW_DEFAULT;
    if (L.panelWidth) return L.panelWidth;
    if (designCtx == null) designCtx = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--ctx'), 10) || LIMITS.panel[0];
    return designCtx;
  }
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
  function setWidth(kind, w) {
    if (kind === 'panel') app.style.setProperty('--ctx', w + 'px'); else app.style.setProperty('--now-w', w + 'px');
  }
  function save(kind, w) { App.layout.set(kind === 'panel' ? { panelWidth: w } : { nowWidth: w }); }
  document.addEventListener('pointerdown', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h || e.button !== 0) return;
    e.preventDefault();
    var kind = h.getAttribute('data-resize'), x0 = e.clientX, w0 = current(kind), w = w0, frame = 0;
    h.setPointerCapture(e.pointerId); app.classList.add('is-resizing');
    /* the handle sits on the column's left edge: dragging left widens it */
    function move(ev) {
      w = clamp(w0 - (ev.clientX - x0), LIMITS[kind][0], LIMITS[kind][1]);
      if (!frame) frame = requestAnimationFrame(function () { frame = 0; setWidth(kind, w); });
    }
    function end() {
      h.removeEventListener('pointermove', move); h.removeEventListener('pointerup', end); h.removeEventListener('pointercancel', end);
      cancelAnimationFrame(frame); app.classList.remove('is-resizing');
      if (w !== w0) save(kind, w);
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
  App.action('now-column', function () { setNow(nowShown() ? false : 'open'); });
  App.action('nowcol-collapse', function () { setNow('strip'); });
  App.action('nowcol-expand', function () { setNow('open'); });
  var lastNow = null;
  new MutationObserver(function () {
    var v = app.getAttribute('data-nowcol');
    if (v === 'open' && lastNow !== 'open' && App.nowColumn) App.nowColumn.render($('#nowcol-body'));
    lastNow = v;
  }).observe(app, { attributes: true, attributeFilter: ['data-nowcol'] });

  App.action('panel-split', function () { App.layout.set({ panelSplit: !L.panelSplit }); App.refreshPanel(); });
  apply();
})();
