/* Layout (ADR-0007): layouts reveal more as the window grows, and the listener can adjust them.
   - Every layout choice made here is a layout switch (ADR-0101): a custom property on .app that lull's CSS sets to
     lull's own choice (components.css) and a theme may set to anything else, read here instead of decided. App.layout.
     switch(name) gives one to the rest of lull's script. They are read where layout already happens (a window resize,
     a route, a design or stylesheet, a size class that switched), all from one computed style, before anything is
     written, never per frame; `layout:switch` fires when one changed.
   - Size classes come from the main area's own width: .main is the `main` container for CSS (@container main).
     A ResizeObserver mirrors the class onto .app[data-size] (compact | regular | wide | xwide | ultra, from the
     --layout-size-* widths) and the window's height onto .app[data-height] (short below --layout-height-short), for
     scripts and designs. No layout is read in a resize loop.
   - App.layout holds the listener's choices, per device: localStorage 'lull:layout' = { panelWidth, panelHeight,
     panelCollapsed, nowColumn, nowWidth, searchSplit, panelSplit }. get(), set(patch) (emits 'layout'), reset(),
     size(). Sizes ride on custom properties of .app (--ctx, --ctx-h, --now-w), so the columns resize with CSS alone.
   - Resize handles [data-resize="panel|now"] (role separator): a pointer drag throttled to animation frames, arrow
     keys (Shift for bigger steps), Home / End for the limits, double-click for the default. While a drag runs, its
     size (`live`) is the one shown, through --ctx-drag / --ctx-h-drag / --now-drag on the elements that read it
     (components.css says why), so the column follows the pointer each frame; it is saved when the drag ends. The panel
     docked by its height (--layout-ctx-axis: block) drags its height from its top edge, by the same code.
   - The Now Playing column (.nowcol, from `wide`): beside the main area in the same grid cell, so no design's shell
     grid changes. App.nowColumn (now.js) fills it. Closed on /now, which shows all of it already.
     It and the docked side panel follow one rule (docs/context/web.md, Layout):
     - the column is offered when the main area stays `wide` beside it with the panel at its strip: the window's width
       alone decides, so its button never comes and goes as the panel opens or closes;
     - both are open when the main area stays `wide` beside both; otherwise opening one closes the other (the column's
       choice is forgotten, the panel collapses to its strip), and closing it brings nothing back.
     That is --layout-nowcol: room. `kept` offers the column at every width and neither closes the other; `open` keeps
     it open (or at the listener's strip) on every screen but Now Playing, with no button to close it. A panel docked
     by its height takes no width from the main area, so it never closes the column. */
(function () {
  'use strict';
  var $ = App.$, $$ = App.$$, clamp = App.clamp;
  var CLASSES = ['compact', 'regular', 'wide', 'xwide', 'ultra'];   // narrowest first
  var FROM = { regular: 'size-regular', wide: 'size-wide', xwide: 'size-xwide', ultra: 'size-ultra' };   // each from its --layout-size-* width
  var DEFAULTS = { panelWidth: null, panelHeight: null, panelCollapsed: false, nowColumn: false, nowWidth: null, searchSplit: false, panelSplit: false };
  /* the bounds of what the listener drags, by dimension: the panel's width, its height (docked by its height), the
     column's width. The listener's own choice, kept per device whatever the theme, so it stays valid in every theme */
  /* the bounds of each, from the layout switches (lull's: 280 to 560, 200 to 600, 300 to 560 px), read as they are */
  var LIMIT = { panel: ['panel-min', 'panel-max'], 'panel-h': ['panel-h-min', 'panel-h-max'], now: ['now-min', 'now-max'] };
  function lim(dim) { return [px(LIMIT[dim][0]), Math.max(px(LIMIT[dim][0]), px(LIMIT[dim][1]))]; }
  var LIMITS = { get panel() { return lim('panel'); }, get 'panel-h'() { return lim('panel-h'); }, get now() { return lim('now'); } };
  var STORED = { panel: 'panelWidth', 'panel-h': 'panelHeight', now: 'nowWidth' };
  var KEY_STEP = 16;
  /* a length switch in px (each is registered as a <length>, so it reads as one) */
  function px(name) { return parseFloat(S[name]) || 0; }
  var app = $('#app'), main = $('.main');
  var live = { panel: null, 'panel-h': null, now: null };   // a size being dragged: shown at once, saved when the drag ends
  var DRAG = { panel: '--ctx-drag', 'panel-h': '--ctx-h-drag', now: '--now-drag' };
  function readers(dim) { return dim === 'now' ? [main, $('#nowcol')] : [app, $('#ctx')]; }

  /* ---------- layout switches (ADR-0101; docs/themes.md, section 5, Layout switches) ----------
     Their values are registered in components.css, so each read is one of them. Read all at once from one computed
     style, before this pass writes anything */
  /* every --layout-* that components.css registers, one line (tests/web/layout-switches.test.mjs holds it to them) */
  var SWITCHES = ['--layout-ctx', '--layout-ctx-axis', '--layout-ctx-dismiss', '--layout-ctx-split', '--layout-search-split', '--layout-nowcol', '--layout-phone', '--layout-size-regular', '--layout-size-wide', '--layout-size-xwide', '--layout-size-ultra', '--layout-height-short', '--layout-panel-min', '--layout-panel-max', '--layout-panel-h-min', '--layout-panel-h-max', '--layout-now-min', '--layout-now-max', '--layout-now-width', '--layout-strip', '--layout-nowcol-gap', '--layout-popover-margin', '--layout-popover-side', '--layout-tip-margin'];
  var S = {};   // by name without --layout-: S.ctx, S['size-wide']
  function readSwitches() {
    var cs = getComputedStyle(app), changed = false;
    SWITCHES.forEach(function (p) {
      var n = p.slice(9), v = cs.getPropertyValue(p).trim();
      if (S[n] !== v) { S[n] = v; changed = true; }
    });
    return changed;
  }
  /* a switch changed: lay out by it, and tell the rest of lull's script (app.js docks the panel, Search redraws) */
  function switched() { apply(); App.emit('layout:switch'); }
  readSwitches();
  /* the panel's resize handle drags its height when a theme docks it by its height */
  function dimOf(kind) { return kind === 'panel' && S['ctx-axis'] === 'block' ? 'panel-h' : kind; }
  function byWidth() { return S['ctx-axis'] !== 'block'; }

  /* ---------- the stored choices: anything unknown or out of range falls back to the default ---------- */
  function width(v, dim) { return typeof v === 'number' && isFinite(v) ? clamp(Math.round(v), LIMITS[dim][0], LIMITS[dim][1]) : null; }
  function sanitize(s) {
    s = s && typeof s === 'object' ? s : {};
    return {
      panelWidth: width(s.panelWidth, 'panel'), panelHeight: width(s.panelHeight, 'panel-h'), panelCollapsed: s.panelCollapsed === true,
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
    /* the main area's size class; at(name), whether it is at least that one (never for `none`) */
    size: function () { return size || 'compact'; },
    at: function (name) { return rank(name) > 0 && rank(App.layout.size()) >= rank(name); },
    /* a layout switch's value, as the last pass read it: lull's script asks this, never the window */
    switch: function (name) { return S[name]; },
    limits: LIMITS, defaults: { get now() { return px('now-width'); } }
  };
  function rank(name) { return CLASSES.indexOf(name) + 1; }
  /* the widest class whose width the main area reaches */
  function classOf(w) {
    for (var i = CLASSES.length - 1; i > 0; i--) if (w >= parseFloat(S[FROM[CLASSES[i]]])) return CLASSES[i];
    return CLASSES[0];
  }

  /* ---------- applying: attributes and custom properties on .app, nothing measured ---------- */
  function docked() { return app.getAttribute('data-ctx') === 'docked'; }
  function nowShown() { var v = app.getAttribute('data-nowcol'); return v === 'open' || v === 'strip'; }
  function nowOpen() { return live.now || L.nowWidth || px('now-width'); }
  function nowWidth() { return L.nowColumn === 'strip' ? px('strip') : L.nowWidth || px('now-width'); }
  function panelExpanded() { return docked() && !L.panelCollapsed; }
  /* what the expanded panel takes from the main area's width beside its strip: nothing when docked by its height */
  function panelCost() { return byWidth() ? current('panel') - px('strip') : 0; }
  /* the main area's width with the Now column closed and the panel at its strip (no feedback loop: it doesn't change
     as either opens), and what the expanded panel takes from it */
  function freeWidth() {
    var shownW = nowShown() ? (app.getAttribute('data-nowcol') === 'strip' ? px('strip') : nowOpen()) + px('nowcol-gap') : 0;
    return mainW + shownW + (panelExpanded() ? panelCost() : 0);
  }
  function wideAt(w) { return rank(classOf(w)) >= rank('wide'); }
  /* the column and the expanded panel side by side: by the room (--layout-nowcol: room), or always (kept, open) */
  function bothFit() { return S.nowcol !== 'room' || wideAt(freeWidth() - panelCost()); }
  function apply() {
    var always = S.nowcol === 'open', offered = !always && (S.nowcol === 'kept' || wideAt(freeWidth()));
    var show = app.getAttribute('data-screen') !== 'now' && (always || offered && L.nowColumn && (!panelExpanded() || bothFit()));
    toggleAttr('data-nowcol-ok', offered ? '' : null);
    toggleAttr('data-nowcol', show ? L.nowColumn || 'open' : null);
    prop(app, '--now-w', nowWidth() + 'px');
    /* the panel: its stored width (or height), or the icon strip while docked and collapsed */
    var strip = docked() && L.panelCollapsed;
    prop(app, '--ctx', strip && byWidth() ? px('strip') + 'px' : L.panelWidth ? L.panelWidth + 'px' : '');
    prop(app, '--ctx-h', strip && !byWidth() ? px('strip') + 'px' : L.panelHeight ? L.panelHeight + 'px' : '');
    toggleAttr('data-panel', strip ? 'strip' : null);
    toggleAttr('data-panel-split', L.panelSplit && App.layout.at(S['ctx-split']) ? '' : null);
    showLive('panel'); showLive('panel-h'); showLive('now');
    $$('[data-resize]').forEach(function (h) {
      var d = dimOf(h.getAttribute('data-resize'));
      h.setAttribute('aria-orientation', d === 'panel-h' ? 'horizontal' : 'vertical');
      h.setAttribute('aria-valuemin', LIMITS[d][0]); h.setAttribute('aria-valuemax', LIMITS[d][1]);
      h.setAttribute('aria-valuenow', current(d));
    });
    $$('[data-act="now-column"]').forEach(function (b) { b.setAttribute('aria-pressed', String(!!show)); });
    $$('[data-act="panel-split"]').forEach(function (b) { b.setAttribute('aria-pressed', String(L.panelSplit)); });
  }
  function showLive(dim) {
    readers(dim).forEach(function (el) { if (el) prop(el, DRAG[dim], live[dim] == null ? '' : live[dim] + 'px'); });
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
  /* a column's size as shown: the stored one, or the design's own (--ctx, --ctx-h, read once per design) until one is set */
  var designSize = {}, TOKEN = { panel: '--ctx', 'panel-h': '--ctx-h' };
  function current(dim) {
    if (dim === 'now') return nowOpen();
    var own = live[dim] || L[STORED[dim]];
    if (own) return own;
    if (designSize[dim] == null) designSize[dim] = parseInt(getComputedStyle(document.documentElement).getPropertyValue(TOKEN[dim]), 10) || LIMITS[dim][0];
    return designSize[dim];
  }
  /* read again once the design's stylesheet is in (boot), and whenever the design changes: its sizes and its switches */
  function restyled() { designSize = {}; if (readSwitches()) switched(); else apply(); }
  App.on('ready', restyled);
  App.on('design', restyled);
  /* the window resized: the switches lull's CSS sets by its width (the panel docking, the phone's layout) follow it here,
     where lull laid out by the window before, so they switch in the same frame */
  window.addEventListener('resize', function () { if (readSwitches()) switched(); });

  /* ---------- size classes: the main area's width and the window's height, as ResizeObserver reports them ---------- */
  /* the states a resize can switch, which change how the rows are laid out */
  function states() { return ['data-size', 'data-height', 'data-nowcol', 'data-nowcol-ok', 'data-panel', 'data-panel-split'].map(function (n) { return app.getAttribute(n); }).join('|'); }
  var ro = new ResizeObserver(function (entries) {
    var was = states(), classes = app.getAttribute('data-size') + app.getAttribute('data-height');
    entries.forEach(function (e) {
      if (e.target === main) mainW = e.contentRect.width;
      else short = e.contentRect.height < parseFloat(S['height-short']);
    });
    var next = classOf(mainW);
    toggleAttr('data-height', short ? 'short' : 'tall');
    apply();
    if (next !== size) { size = next; toggleAttr('data-size', size); apply(); App.emit('layout:size', size); }
    /* a switch a theme sets by size class (.app[data-size="wide"]) follows the class: read once more when one switched */
    if (app.getAttribute('data-size') + app.getAttribute('data-height') !== classes && readSwitches()) switched();
    /* a state switched lays the rows out anew: fit them now, in this frame. Left to the rows' own observer, the change
       reaches it a round later, which a frame can end before, and the header stuck out for a frame (CI, Skydeck, 900 px) */
    if (states() !== was && App.layout.refit) App.layout.refit();
  });
  ro.observe(main); ro.observe(document.documentElement);
  /* the panel docking (app.js) and the route (/now hides the column) change what applies; a switch may follow the
     screen (.app[data-screen="home"]), so a route reads them again. Never on data-ctx, which follows the switches */
  new MutationObserver(function (records) {
    if (records.some(function (r) { return r.attributeName === 'data-screen'; }) && readSwitches()) switched();
    else apply();
  }).observe(app, { attributes: true, attributeFilter: ['data-ctx', 'data-screen'] });

  /* ---------- resize handles ---------- */
  function save(dim, w) { var patch = {}; patch[STORED[dim]] = w; App.layout.set(patch); }
  document.addEventListener('pointerdown', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h || e.button !== 0) return;
    e.preventDefault();
    var d = dimOf(h.getAttribute('data-resize')), along = d === 'panel-h' ? 'clientY' : 'clientX';
    var p0 = e[along], w0 = current(d), w = w0, frame = 0;
    h.setPointerCapture(e.pointerId); app.classList.add('is-resizing');
    /* the handle sits on the column's left edge (the panel's top edge, docked by its height): dragging away from the
       column, left (or up), makes it bigger */
    function move(ev) {
      w = clamp(w0 - (ev[along] - p0), LIMITS[d][0], LIMITS[d][1]);
      if (!frame) frame = requestAnimationFrame(function () { frame = 0; live[d] = w; apply(); });
    }
    function end() {
      h.removeEventListener('pointermove', move); h.removeEventListener('pointerup', end); h.removeEventListener('pointercancel', end);
      cancelAnimationFrame(frame); app.classList.remove('is-resizing');
      live[d] = null;
      if (w !== w0) save(d, w); else apply();
    }
    h.addEventListener('pointermove', move); h.addEventListener('pointerup', end); h.addEventListener('pointercancel', end);
  });
  document.addEventListener('dblclick', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h) return;
    save(dimOf(h.getAttribute('data-resize')), null);
  });
  document.addEventListener('keydown', function (e) {
    var h = e.target.closest && e.target.closest('[data-resize]'); if (!h) return;
    var kind = h.getAttribute('data-resize'), d = dimOf(kind), w = current(d), step = KEY_STEP * (e.shiftKey ? 4 : 1), lim = LIMITS[d];
    var keys = d === 'panel-h' ? { ArrowUp: w + step, ArrowDown: w - step } : { ArrowLeft: w + step, ArrowRight: w - step };
    keys.Home = lim[0]; keys.End = lim[1];
    var next = keys[e.key];
    if (next == null) return;
    e.preventDefault();
    save(d, clamp(next, lim[0], lim[1]));
    App.announce((kind === 'panel' ? 'Panel' : 'Now Playing column') + ' ' + current(d) + ' pixels ' + (d === 'panel-h' ? 'tall' : 'wide'));
  });

  /* ---------- the Now Playing column ---------- */
  function setNow(v) { App.layout.set({ nowColumn: v }); }
  /* opening one of the two closes the other when the main area wouldn't stay wide beside both (the rule above; never
     while --layout-nowcol is kept or open, bothFit); app.js calls this as the panel opens docked */
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
    /* a part a design placed outside the row's flow (glasshouse floats the tray into the rail) takes no room in it. Its
       order is its data-fit, or a theme's --fit-order (a layout switch), read from the style read here already */
    var order = new Map();
    var parts = $$('[data-fit]', row).filter(function (p) {
      var s = getComputedStyle(p), own = s.getPropertyValue('--fit-order').trim();
      order.set(p, own && own !== 'auto' ? +own : +p.getAttribute('data-fit'));
      return fitRowOf(p) === row && s.position !== 'fixed' && s.position !== 'absolute';
    });
    function orderOf(p) { return order.get(p); }
    var orders = parts.map(orderOf)
      .filter(function (o, i, all) { return all.indexOf(o) === i; }).sort(function (a, b) { return a - b; });
    /* the row says what gave way in it (the orders, and "all" once every one did), at each step, for a style that
       follows a part out (lull's cache meter, the search's padding) and so takes its room with it as the row is measured:
       a :has([data-fit-out]) would restyle the whole page at every step in a design with a :has() on the shell */
    function give(i, out) {
      parts.forEach(function (p) { if (orderOf(p) === orders[i]) p.toggleAttribute('data-fit-out', out); });
      var off = orders.filter(function (o) { return parts.some(function (p) { return orderOf(p) === o && p.hasAttribute('data-fit-out'); }); }),
        gave = off.join(' ') + (off.length && off.length === orders.length ? ' all' : '');
      if (row.getAttribute('data-fit-gave') !== gave) row.setAttribute('data-fit-gave', gave);
    }
    var k = 0, was;
    while (k < orders.length && parts.every(function (p) { return orderOf(p) !== orders[k] || p.hasAttribute('data-fit-out'); })) k++;
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
    /* in whole pixels, as every width the sides are tried at: the search below halves between whole widths. A theme
       that lays the row out its own way ends the mirroring with --fit-sides: none (a layout switch), and lull's
       --fit-side goes */
    var mirror = root.hasAttribute('data-fit-sides') && getComputedStyle(root).getPropertyValue('--fit-sides').trim() !== 'none',
      side = mirror && $('[data-fit-side]', root), most = side ? Math.floor(sideMost(root)) : 0;
    if (side) root.style.setProperty('--fit-side', most + 'px');
    else if (root.style.getPropertyValue('--fit-side')) root.style.removeProperty('--fit-side');
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
     text, on load for what it imports. Only <head> holds the sheets, so nothing else on the page pays. The sheet may set
     the layout switches too (a theme edited live), so they are read again first */
  function sheet(n) { return n && (n.nodeName === 'STYLE' || n.nodeName === 'LINK'); }
  function sheetIn() { if (readSwitches()) switched(); App.layout.refit(); }
  if (window.MutationObserver && document.head) new MutationObserver(function (records) {
    if (records.some(function (r) { return sheet(r.target) || sheet(r.target.parentNode) || [].some.call(r.addedNodes, sheet) || [].some.call(r.removedNodes, sheet); })) sheetIn();
  }).observe(document.head, { childList: true, subtree: true, characterData: true });
  document.addEventListener('load', function (e) { if (sheet(e.target)) sheetIn(); }, true);
  App.layout.refit();

  apply();
})();
