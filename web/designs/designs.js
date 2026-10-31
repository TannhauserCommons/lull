/* Design registry, slots and switching. Classic script, loaded in <head> right after the
   <link id="design-css"> by prototype/index.html, desktop/preview.html and desktop/window.html.
   A design = designs/<name>/design.css (loaded after every other stylesheet) + optional design.js
   (registers slot overrides with Designs.define) + optional fonts/. API: DESIGN.md section 3. */
(function () {
  'use strict';
  var D = window.Designs = {};
  /* the product name: every shipped string (titles, brand link names, copy) reads it from here */
  D.product = 'lull';

  /* Each entry is owned by that design's author (edit only your own entry).
     shell: 'rail' | 'topnav' | 'dock' (desktop and tablet layout; phones always use the tab bar).
     swatches: [page, surface, accent, ink] for the Settings > Appearance preview card.
     preview: true when assets/previews/<name>-day.webp and -night.webp exist (1440x900 screenshots scaled to 320x200; the card
       shows them instead of the swatch shell).
     js: true when designs/<name>/design.js exists (it is only requested then). */
  D.list = {
    lull: { label: 'Lull', shell: 'topnav', js: true, preview: true,
      description: 'Glossy lavender windows, pill navigation and orb title bars.',
      swatches: ['#e4def9', '#f7f4ff', '#5544aa', '#2b2650'] },
    skydeck: { label: 'Skydeck', shell: 'rail', js: true, preview: true,
      description: 'A pocket handheld under a moonlit dusk sky, with cream trays.',
      swatches: ['#414759', '#f2e3d1', '#b6c7f5', '#856e74'] },
    starlight: { label: 'Starlight', shell: 'rail', js: true, preview: true,
      description: 'A starry-night poster: cream trays stacked on lavender and periwinkle, burgundy caps titles.',
      swatches: ['#414759', '#f2e3d1', '#5a75b5', '#4a0f1c'] },
    petal: { label: 'Petal', shell: 'topnav', js: true, preview: true,
      description: 'Pink pixel playlist pages with a glossy pink player window.',
      swatches: ['#fce8f3', '#fffbfd', '#dd9fc9', '#4a3542'] },
    glasshouse: { label: 'Glasshouse', shell: 'dock', js: true, preview: true,
      description: 'Frosted silver media windows on a misty midnight desktop.',
      swatches: ['#3d4152', '#c9ccd8', '#93b8ce', '#8e3f58'] }
  };
  D.order = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
  D.fallback = 'lull';
  D.current = null;

  var root = document.documentElement;
  var link = document.getElementById('design-css');
  /* designs/ folder relative to this page, read from the link the page ships with */
  D.base = link ? link.getAttribute('href').replace(/[^/]+\/design\.css$/, '') : 'designs/';

  D.valid = function (name) { return Object.prototype.hasOwnProperty.call(D.list, name); };
  /* the design to show: design= in the hash (#/route?design=x or #view=x&design=x) or query, else the stored pref, else the fallback */
  D.wanted = function () {
    var m = /(?:^|[#?&])design=([a-z0-9-]+)/.exec(location.hash + '&' + location.search);
    if (m && D.valid(m[1])) return m[1];
    var p = null; try { p = JSON.parse(localStorage.getItem('lull:prefs')); } catch (e) { p = null; }
    return p && D.valid(p.design) ? p.design : D.fallback;
  };

  /* ---------- theme: a stored 'day' | 'night' | 'system' always becomes an explicit data-theme ----------
     'system' is resolved from the OS (prefers-color-scheme) here and again whenever the OS flips, so CSS only
     ever sees :root[data-theme="day"] or [data-theme="night"] and needs no prefers-color-scheme copy.
     :root[data-theme-pref] keeps the choice. Called by the pre-paint scripts and App.setPref('theme'). */
  var themePref = 'system';
  var darkOs = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  D.applyTheme = function (pref) {
    themePref = pref === 'day' || pref === 'night' ? pref : 'system';
    var t = themePref !== 'system' ? themePref : darkOs && darkOs.matches ? 'night' : 'day';
    root.setAttribute('data-theme', t);
    root.setAttribute('data-theme-pref', themePref);
    return t;
  };
  if (darkOs) {
    var onOs = function () { if (themePref === 'system') D.applyTheme('system'); };
    if (darkOs.addEventListener) darkOs.addEventListener('change', onOs); else if (darkOs.addListener) darkOs.addListener(onOs);
  }

  /* default brand markup (the `brand` slot's fallback): .brand holds the visible name, .brand__orn an optional
     ornament (the sparkle). Decorative: the link or element around it carries the accessible name (D.product). */
  D.brandMarkup = function () {
    return '<span class="wordmark brand" aria-hidden="true"><span class="brand__name">' + D.product + '</span>' +
      '<span class="brand__orn wordmark__spark"></span></span>';
  };

  /* ---------- slots: markup a design.js may replace ----------
     Designs.define('lull', { nowHero: function (ctx, fallback) { return '...'; } })
     Designs.slot('nowHero', ctx, fallback) -> the current design's override, else fallback(ctx).
     An override returns markup (string or App.html Safe); it may call fallback(ctx) to wrap the default. */
  var overrides = {};
  D.define = function (name, slots) { overrides[name] = Object.assign(overrides[name] || {}, slots); };
  D.slot = function (name, ctx, fallback) {
    var o = overrides[D.current] && overrides[D.current][name];
    if (o) { try { return o(ctx || {}, fallback); } catch (e) { console.error('[design slot ' + name + ']', e); } }
    return fallback(ctx || {});
  };

  /* ---------- loading ---------- */
  var jsState = {};                           // name -> 'loading' | 'done'
  var jsWaiters = {};
  function loadJs(name, done) {
    if (!D.list[name].js || jsState[name] === 'done') { done(); return; }
    (jsWaiters[name] = jsWaiters[name] || []).push(done);
    if (jsState[name] === 'loading') return;
    jsState[name] = 'loading';
    var s = document.createElement('script');
    s.src = D.base + name + '/design.js';
    s.onload = s.onerror = function () {
      if (jsState[name] !== 'done') { jsState[name] = 'done'; (jsWaiters[name] || []).splice(0).forEach(function (fn) { fn(); }); }
    };
    document.head.appendChild(s);
  }
  function applyAttrs(name) {
    D.current = name;
    root.setAttribute('data-design', name);
    root.setAttribute('data-shell', D.list[name].shell || 'rail');
  }

  /* before first paint (inline script in <head>): attribute + stylesheet, and start design.js */
  D.boot = function () {
    var name = D.wanted();
    applyAttrs(name);
    if (link) link.setAttribute('href', D.base + name + '/design.css');
    loadJs(name, function () {});
  };
  /* run fn once the current design's design.js (if any) has registered its slots */
  D.ready = function (fn) { loadJs(D.current || D.wanted(), fn); };

  /* switch without a reload: the new stylesheet loads next to the old one, then they swap, so
     there is no unstyled frame. onDone runs after the attributes changed (re-render there). If the
     stylesheet fails to load, the current design stays and onFail(name) runs. */
  D.set = function (name, onDone, onFail) {
    if (!D.valid(name)) { console.warn('Unknown design', name); return false; }
    if (name === D.current) { if (onDone) onDone(); return true; }
    var next = document.createElement('link'), finished = false;
    next.rel = 'stylesheet'; next.href = D.base + name + '/design.css';
    function swap() {
      if (finished) return; finished = true;
      loadJs(name, function () {
        var old = document.getElementById('design-css');
        if (old && old !== next) old.remove();
        next.id = 'design-css'; link = next;
        applyAttrs(name);
        if (onDone) onDone();
      });
    }
    next.onload = swap;
    next.onerror = function () {
      if (finished) return; finished = true;
      next.remove();
      console.error('Design stylesheet failed to load: ' + next.href);
      if (onFail) onFail(name);
    };
    (link && link.parentNode ? link.parentNode.insertBefore(next, link.nextSibling) : document.head.appendChild(next));
    return true;
  };
})();
