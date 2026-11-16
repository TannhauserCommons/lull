/* lull desktop wrapper UI (Tauri v2). Spec: design/desktop/DESKTOP.md; wiring: docs/context/desktop.md.
   Styles: desktop.css. Classic script; needs only ../icons.js. Loaded by the web UI (index.html) and by
   the wrapper's own page (wrapper.html). Outside the wrapper it does nothing. Exposes window.LullDesktop:
     inWrapper                                       true inside the desktop app
     attachTitlebar(os, layout) / detachTitlebar()   Windows caption buttons or GNOME header-bar buttons
     welcomePage() / connectPage(opts)               bundled first-run pages: which hub, pairing
     offlinePage(opts) / hubErrorPage(opts)          bundled pages: hub unreachable, hub didn't start
     crashedPage()                                   bundled page: the window kept crashing (app/src/desktop/recovery.rs)
     miniDevice() / miniStrip()                      mini-player markup (the miniPlayer design slot)
     askClose()                                      one-time "keep playing in the tray?" dialog
     windowAction(action, detail)                    window operations (DESKTOP.md 4, window_action)
     settings() / setSettings(patch)                 Settings > App inside the wrapper (Promises)
     glass                                           the see-through window: state ({ material, held }, null until the
                                                     app answered) and on(fn), which hears every change
     tailscale() / tailscaleServe()                  Settings > App when this computer is the hub (Promises)
     updates                                         this app's own updates: status(), check(), install() (Promises of
                                                     { current, state, ... }) and on(fn), which hears every change
     openMini()                                      the player bar's Device view button
     openClassic(handoff) / onClassic(fn)            Classic mode's own window (screens/classic.js): open it with the
                                                     queue; fn(message) hears what Webamp's window reports
     classicBack()                                   the queue the native Classic window hands this page when it closes
                                                     (null if none): a Promise
     front() / onFront(fn)                           main's web UI: one of lull's windows has the focus (a Promise of a
                                                     boolean); fn(boolean) hears each change once the focus settles
     quiet() / onQuiet(fn)                           main's web UI: the system asks for quiet (Windows' No Sounds scheme,
                                                     GNOME's event sounds off): a Promise of a boolean; fn hears changes
                                                     (front and quiet exist only in the app: ADR-0081's UI sounds)
     graphics                                        what draws the page (app/src/desktop/graphics.rs): state ({ mode,
                                                     software, light, fullLook, renderer, fellBack, ask }, null until the
                                                     app answered) and on(fn), which hears every change
     mountPage()                                     wrapper.html: render the view named in the hash
   window.__LULL_DESKTOP__ ({ os, titlebar, layout }) is set by the wrapper before any page script runs. */
(function () {
  'use strict';
  var D = window.LullDesktop = {};
  var doc = document, root = doc.documentElement;
  /* Only a window's own page is the app's. lull's page in a frame of it (a theme's picture, Theme Studio's previews)
     is a view: on Windows it gets the app's scripts too (WebView2 runs them in every frame), and as the app's page it
     would tell the app its design (the mini player and the app's pages would switch to the theme it draws), ask for the
     window's glass, report what it plays, and answer the tray and the mini player's keys with a player of its own. */
  var own = window.top === window;
  var T = own && window.__TAURI__ || null;
  var W = own && window.__LULL_DESKTOP__ || null;      // only in the wrapper's main window
  // The Android app also has Tauri, but none of the desktop's windows; it brings window.LullNative instead.
  D.inWrapper = !!T && !window.LullNative;

  function $(sel, el) { return (el || doc).querySelector(sel); }
  function $$(sel, el) { return Array.prototype.slice.call((el || doc).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; }); }
  function ic(name, cls) { return window.icon(name, null, cls); }
  function announce(text) {
    var el = $('#dk-announcer');
    if (!el) { el = doc.createElement('div'); el.id = 'dk-announcer'; el.className = 'vh'; el.setAttribute('aria-live', 'polite'); doc.body.appendChild(el); }
    el.textContent = ''; setTimeout(function () { el.textContent = text; }, 30);
  }
  function logFailure(what) { return function (err) { console.error('lull desktop: ' + what + ' failed', err); }; }

  /* ---------- window API ---------- */
  function currentWindow() { return T ? T.window.getCurrentWindow() : null; }
  function invoke(cmd, args) { return T ? T.core.invoke(cmd, args) : Promise.reject(new Error('not in the desktop app')); }
  D.windowAction = function (action, detail) {
    var w = currentWindow();
    if (!w) return Promise.reject(new Error('not in the desktop app'));
    if (action === 'minimize') return w.minimize();
    if (action === 'maximize') return w.toggleMaximize();
    if (action === 'close') return w.close();       // the wrapper decides: hide to tray, ask, or quit
    return invoke('window_action', { request: Object.assign({ action: action }, detail || {}) });
  };
  D.settings = function () { return invoke('desktop_settings'); };
  D.setSettings = function (patch) {
    return invoke('set_desktop_settings', { patch: patch }).then(function (r) {
      // Graphics take effect at the next start (app/src/desktop/graphics.rs)
      if (patch.graphics && window.App) App.toast('lull draws this way from its next start', { icon: 'refresh', action: 'Restart now', onAction: function () { D.windowAction('restart'); } });
      // and so do the helpers' copies (app/src/desktop/local_hub.rs)
      if (patch.ownHelpers != null && window.App) App.toast('lull uses these copies from its next start', { icon: 'refresh', action: 'Restart now', onAction: function () { D.windowAction('restart'); } });
      return r;
    });
  };
  /* a see-through window (app/src/desktop/glass.rs): what `main` shows behind the page, and why not when its theme
     asked for glass and got none (held: off, system, unsupported, restart) */
  var glassFns = [];
  D.glass = { state: null, on: function (fn) { glassFns.push(fn); } };
  function showGlass(grant) {
    D.glass.state = grant;
    Designs.setWindow(grant && grant.material);
    glassFns.forEach(function (fn) { fn(grant); });
  }
  /* graphics (app/src/desktop/graphics.rs): where the processor draws the page, it takes lull's lighter look,
     :root[data-graphics="light"] (base.css: no blur behind glass, no movement), and decorative loops hold still. The app
     finds out itself on Linux (WebKitGTK's page scripts can't tell); on Windows it asks the page for WebGL's renderer
     (state.ask), once the page has drawn. The listener's first click or key here tells the app this start's graphics
     work: a start in a chosen mode that never gets one falls back to Automatic. The last look is kept on this device,
     so a page opens in it. */
  var graphicsFns = [];
  D.graphics = { state: null, on: function (fn) { graphicsFns.push(fn); } };
  function lightLook(on) {
    if (on) root.setAttribute('data-graphics', 'light'); else root.removeAttribute('data-graphics');
    if (window.App && App.still) App.still('graphics', on);
  }
  function webglRenderer() {
    var gl = doc.createElement('canvas').getContext('webgl');
    if (!gl) return null;
    var info = gl.getExtension('WEBGL_debug_renderer_info'), lose = gl.getExtension('WEBGL_lose_context');
    var name = String(gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER));
    if (lose) lose.loseContext();
    return name;
  }
  /* Settings › App's line under its Graphics row (web/screens/system.js draws the row, #sys-graphics): what draws lull
     and the look that follows, with the other look a click away, or that a chosen mode didn't come up */
  D.graphics.help = 'How lull draws its window. Takes effect after a restart.';
  D.graphics.modes = [['auto', 'Automatic'], ['gpu', 'Use the graphics card'], ['cpu', 'Processor only']];
  var MODES = { gpu: D.graphics.modes[1][1], cpu: D.graphics.modes[2][1] };
  D.graphics.line = function () {
    var g = D.graphics.state;
    if (g && g.fellBack) return '<p class="sys-note sys-note--warn">' + ic('warning', 'ic-16') + '<span>The last start with “' +
      MODES[g.fellBack] + '” didn’t come up, so lull went back to Automatic.</span></p>';
    if (!g || !g.software) return '';
    return '<p class="sys-note">' + ic('info', 'ic-16') + '<span>' + (g.fullLook ? 'Your computer draws lull without its graphics card. The full look may feel slow.'
      : 'Your computer draws lull without its graphics card, so lull uses a lighter look: no blur, fewer animations.') +
      '</span><button class="btn btn--sm" data-dk-look>' + (g.fullLook ? 'Use the lighter look' : 'Use the full look') + '</button></p>';
  };
  /* Settings › App's line on the helper programs this computer's hub runs (ADR-0064, app/src/desktop/local_hub.rs):
     each one's version and whose copy it is, any note, what Soulseek says when this computer's untested slskd won't
     work, and the way to lull's own copies (or back) where lull has them */
  var HELPER = { ytdlp: 'yt-dlp', deno: 'Deno', slskd: 'slskd' };
  var WHOSE = { override: 'set by its LULL_ setting', system: 'this computer’s', own: 'lull’s own' };
  var helpersNow = null;
  D.helpersLine = function (h, soulseek) {
    helpersNow = h;
    var notes = h.chosen.filter(function (c) { return c.note; }).map(function (c) { return esc(c.note); });
    var slskd = h.chosen.filter(function (c) { return c.helper === 'slskd'; })[0];
    if (slskd && slskd.source === 'system' && slskd.note && soulseek && soulseek.state === 'down') {
      notes.push('lull can’t use this computer’s slskd ' + esc(slskd.version || '') + ': ' + esc(soulseek.detail) + '.' +
        (h.own ? ' lull’s own copy should work.' : ' lull has no copy of its own here: install slskd ' + esc(slskd.tested) + ', or set LULL_SLSKD.'));
    }
    var mine = h.own && (h.preferOwn || h.chosen.some(function (c) { return c.source === 'system'; }));
    return '<p class="sys-note">' + ic('info', 'ic-16') + '<span>Helpers: ' + h.chosen.map(function (c) {
      return HELPER[c.helper] + ' ' + esc(c.version || '') + ' (' + WHOSE[c.source] + ')';
    }).join(', ') + '.' + (notes.length ? ' ' + notes.join(' ') : '') + '</span>' +
      (mine ? '<button class="btn btn--sm" data-dk-own-helpers>' + (h.preferOwn ? 'Use this computer’s copies' : 'Use lull’s own copies') + '</button>' : '') + '</p>';
  };
  doc.addEventListener('click', function (e) {
    if (!(e.target.closest && e.target.closest('[data-dk-own-helpers]')) || !helpersNow) return;
    D.setSettings({ ownHelpers: !helpersNow.preferOwn }).catch(logFailure('the helpers’ copies'));
  });
  function showGraphics(state) {
    if (!state) return;
    D.graphics.state = state;
    lightLook(state.light);
    var line = doc.getElementById('sys-graphics');
    if (line) line.innerHTML = D.graphics.line();
    try { localStorage.setItem('lull:graphicsLight', state.light ? '1' : ''); } catch (e) { /* no storage: the answer brings the look */ }
    graphicsFns.forEach(function (fn) { fn(state); });
    if (state.ask) requestAnimationFrame(function () { requestAnimationFrame(function () {
      invoke('graphics_report', { drawn: { renderer: webglRenderer() } }).then(showGraphics, logFailure('graphics report'));
    }); });
  }
  if (D.inWrapper) {
    try { lightLook(localStorage.getItem('lull:graphicsLight') === '1'); } catch (e) { /* no storage: the answer brings the look */ }
    invoke('graphics_state').then(showGraphics, logFailure('graphics'));
    T.event.listen('graphics', function (e) { showGraphics(e.payload); }).catch(logFailure('graphics events'));
    var used = function (e) {
      if (!e.isTrusted) return;
      doc.removeEventListener('pointerdown', used, true); doc.removeEventListener('keydown', used, true);
      invoke('graphics_used').catch(logFailure('graphics used'));
    };
    doc.addEventListener('pointerdown', used, true); doc.addEventListener('keydown', used, true);
    doc.addEventListener('click', function (e) {
      if (e.target.closest && e.target.closest('[data-dk-look]') && D.graphics.state) {
        D.setSettings({ fullLook: !D.graphics.state.fullLook }).catch(logFailure('the look'));
      }
    });
    /* in the lighter look, the web UI's own motion stops too (its screens ask App.reducedMotion) */
    if (window.App && App.reducedMotion) {
      var reduced = App.reducedMotion;
      App.reducedMotion = function () { return root.getAttribute('data-graphics') === 'light' || reduced(); };
    }
  }
  D.openMini = function () { return D.windowAction('mini').catch(logFailure('mini player')); };
  /* Classic mode: Webamp alone in its own window (desktop/winamp.js); the app's window hides meanwhile */
  var classicHears = [];
  D.openClassic = function (handoff) { return invoke('classic_open', { handoff: handoff }); };
  D.onClassic = function (fn) { classicHears.push(fn); };
  D.classicBack = function () { return invoke('classic_back'); };
  /* this computer is the hub: Tailscale's state, and serving the hub on the tailnet (only on the listener's click) */
  D.tailscale = function () { return invoke('tailscale_status'); };
  D.tailscaleServe = function () { return invoke('tailscale_serve'); };
  /* this app's own updates (app/src/desktop/updater.rs): the check runs in Rust, a download goes on in the background */
  D.updates = D.inWrapper ? {
    status: function () { return invoke('app_update'); },
    check: function () { return invoke('app_update_check'); },
    install: function () { return invoke('app_update_install'); },
    on: function (fn) { T.event.listen('app-update', function (e) { fn(e.payload); }).catch(logFailure('update events')); }
  } : null;
  /* the UI sounds (ADR-0081, app/src/desktop/sounds.rs): main's page plays them all, the mini player's and Classic
     mode's included, so it asks the app whether any of lull's windows is in front, and whether the system asks for
     quiet. Only main hears the changes. */
  if (D.inWrapper) {
    D.front = function () { return invoke('window_front'); };
    D.onFront = function (fn) {
      T.event.listen('window-front', function (e) { fn(!!e.payload.front); }).catch(logFailure('front events'));
    };
    D.quiet = function () { return invoke('sound_quiet'); };
    D.onQuiet = function (fn) {
      T.event.listen('sound-quiet', function (e) { fn(!!e.payload.quiet); }).catch(logFailure('quiet events'));
    };
  }

  /* caption glyphs: OS-convention shapes, 10x10, 1 px (Windows draws these at 10 px) */
  var GLYPH = {
    min: '<path d="M1 5.5h8"/>',
    max: '<rect x="1" y="1" width="8" height="8" rx="1.5"/>',
    restore: '<rect x="1" y="3" width="6" height="6" rx="1.2"/><path d="M3 3v-.8A1.2 1.2 0 0 1 4.2 1h3.6A1.2 1.2 0 0 1 9 2.2v3.6A1.2 1.2 0 0 1 7.8 7H7"/>',
    close: '<path d="M1.5 1.5l7 7M8.5 1.5l-7 7"/>'
  };
  function glyph(n) { return '<svg class="dk-glyph" viewBox="0 0 10 10" aria-hidden="true" focusable="false">' + GLYPH[n] + '</svg>'; }

  /* =====================================================================
     1. Title bars
     ===================================================================== */
  var maximized = false;
  function winButton(act, label, g, cls) {
    return '<button class="' + cls + '" data-dk="' + act + '" aria-label="' + label + '" data-tip="' + label + '">' + glyph(g) + '</button>';
  }
  D.captionBar = function () {
    return '<div class="dk-caption" role="group" aria-label="Window">' +
      winButton('minimize', 'Minimise', 'min', 'dk-caption__btn') +
      winButton('maximize', maximized ? 'Restore' : 'Maximise', maximized ? 'restore' : 'max', 'dk-caption__btn') +
      winButton('close', 'Close', 'close', 'dk-caption__btn dk-caption__btn--close') + '</div>';
  };
  /* GNOME button-layout, e.g. "appmenu:minimize,maximize,close" (left of ':' = start of the header bar) */
  function headerBarGroup(names, start) {
    var map = { minimize: ['minimize', 'Minimise', 'min'], maximize: ['maximize', 'Maximise', 'max'], close: ['close', 'Close', 'close'] };
    var html = names.filter(function (n) { return map[n]; }).map(function (n) {
      var m = map[n]; return winButton(m[0], m[1], m[2], 'dk-hb__btn');
    }).join('');
    return html ? '<div class="dk-hb' + (start ? ' dk-hb--start' : '') + '" role="group" aria-label="Window">' + html + '</div>' : '';
  }
  var DRAG = ['.header', '.header .spacer', '.header__brand', '.rail__head'];
  function markDragRegions() {
    if (!root.hasAttribute('data-titlebar')) return;
    DRAG.forEach(function (sel) { $$(sel).forEach(function (el) { el.setAttribute('data-tauri-drag-region', ''); }); });
  }
  D.attachTitlebar = function (os, layout) {
    if (root.getAttribute('data-titlebar') === os) return;
    D.detachTitlebar();
    root.setAttribute('data-titlebar', os);
    markDragRegions();
    var header = $('.header');
    var host = doc.createElement('div');
    host.className = 'dk-titlebar' + (header ? '' : ' dk-titlebar--bare');   // bare = a bundled page without the app header
    if (os === 'win') {
      host.innerHTML = '<div class="dk-dragstrip" data-tauri-drag-region></div>' + D.captionBar();
      doc.body.appendChild(host);
    } else if (os === 'gnome') {
      var parts = String(layout || ':minimize,maximize,close').split(':');
      var start = headerBarGroup(parts[0].split(','), true), end = headerBarGroup((parts[1] || '').split(','), false);
      if (header) {
        header.insertAdjacentHTML('afterbegin', start);
        header.insertAdjacentHTML('beforeend', end);
      } else {
        host.setAttribute('data-tauri-drag-region', '');
        host.innerHTML = start + '<span class="spacer" data-tauri-drag-region></span>' + end;
        doc.body.appendChild(host);
      }
    }
  };
  D.detachTitlebar = function () {
    root.removeAttribute('data-titlebar');
    $$('.dk-titlebar, .header > .dk-hb').forEach(function (el) { el.remove(); });
    DRAG.forEach(function (sel) { $$(sel).forEach(function (el) { el.removeAttribute('data-tauri-drag-region'); }); });
  };
  D.setMaximized = function (on) {
    maximized = !!on;
    $$('[data-dk="maximize"]').forEach(function (b) {
      var label = on ? 'Restore' : 'Maximise';
      b.setAttribute('aria-label', label); b.setAttribute('data-tip', label);
      b.innerHTML = glyph(on ? 'restore' : 'max');
    });
  };
  /* the wrapper's main window: attach what it asked for; keep it attached when the web UI re-renders its shell */
  function attachConfigured() { if (W && W.titlebar) D.attachTitlebar(W.titlebar, W.layout); }
  function reattach() { if (W && W.titlebar) { D.detachTitlebar(); attachConfigured(); } }
  /* Windows dims caption glyphs in an inactive window; start active, follow real blur/focus */
  window.addEventListener('blur', function () { root.setAttribute('data-window-inactive', ''); });
  window.addEventListener('focus', function () { root.removeAttribute('data-window-inactive'); });
  (function watchMaximize() {            // keep the glyph honest when the user snaps or double-clicks the bar
    var w = currentWindow(); if (!w || !W) return;
    w.onResized(function () { w.isMaximized().then(D.setMaximized); });
  })();

  /* =====================================================================
     2. Bundled pages: first run (welcome, connect), hub unreachable, hub didn't start
     ===================================================================== */
  function hostOf(url) { try { return new URL(url).host; } catch (e) { return url; } }
  /* the design's brand slot (designs/designs.js) when the page loads it, else the plain wordmark */
  function wordmark() {
    var plain = function () {
      return '<span class="wordmark brand" aria-hidden="true"><span class="brand__name">lull</span><span class="brand__orn wordmark__spark"></span></span>';
    };
    var name = window.Designs ? Designs.product : 'lull';
    return '<span class="brand-mark" role="img" aria-label="' + name + '">' +
      (window.Designs ? String(Designs.slot('brand', { where: 'desktop' }, Designs.brandMarkup)) : plain()) + '</span>';
  }
  function page(inner) { return '<div class="dk-page"><div class="dk-page__inner">' + wordmark() + inner + '</div></div>'; }
  function status(kind, text) {
    var glyph = kind === 'busy' ? '<span class="spin-ring" aria-hidden="true"></span>'
      : kind === 'ok' ? window.icon('status-ok', 'OK', 'sg sg--ok')
      : kind === 'error' ? window.icon('status-down', 'Error', 'sg sg--down') : ic('info');
    return glyph + '<span class="dk-status__text">' + text + '</span>';
  }
  function setStatus(card, kind, text) {
    var st = $('.dk-status', card); st.setAttribute('data-state', kind === 'busy' ? 'testing' : kind === 'info' ? 'idle' : kind);
    st.innerHTML = status(kind, text);
  }

  /* Welcome: this computer is the hub, or a hub elsewhere (ADR-0006). */
  D.welcomePage = function () {
    return page('<section class="card dk-card" aria-labelledby="dk-w-title" data-dk-welcome>' +
      '<h1 class="dk-card__title" id="dk-w-title">where does your music live?</h1>' +
      '<p class="dk-card__lede">One computer is your hub: it keeps your library, finds music on Soulseek and remembers what you play. Your other devices pair with it.</p>' +
      '<div class="dk-choices">' +
      '<button type="button" class="dk-choice" data-dk="this-computer"><span class="dk-choice__title">This computer is my hub</span>' +
      '<span class="dk-choice__text">lull runs here. Next, choose a music folder or your Jellyfin server, then Soulseek and Last.fm.</span></button>' +
      '<button type="button" class="dk-choice" data-dk="connect"><span class="dk-choice__title">Connect to a hub</span>' +
      '<span class="dk-choice__text">Your hub runs on another computer or a server. You&rsquo;ll need a pair code from it.</span></button>' +
      '</div><div class="dk-status" role="status" data-state="idle" hidden></div></section>');
  };
  function useThisComputer(card) {
    $$('.dk-choice', card).forEach(function (b) { b.disabled = true; });
    var st = $('.dk-status', card); st.hidden = false;
    setStatus(card, 'busy', 'Starting your hub&hellip;');
    // on success the app loads the hub's own setup in this window; a failure shows the hub-error page
    D.windowAction('this-computer').catch(function (err) {
      $$('.dk-choice', card).forEach(function (b) { b.disabled = false; });
      setStatus(card, 'error', esc(String(err)));
    });
  }

  /* Connect: hubs found on this network or an address, then a pair code from a paired device. */
  D.connectPage = function (o) {
    o = o || {};
    return page('<section class="card dk-card" aria-labelledby="dk-c-title" data-dk-connect>' +
      '<h1 class="dk-card__title" id="dk-c-title">connect to your hub</h1>' +
      '<p class="dk-card__lede">' + (o.unpaired
        ? 'This hub doesn&rsquo;t know this computer any more. Pair it again with a new code.'
        : 'On a device that already uses your hub, open Settings &gt; Devices &gt; Pair a device for a code.') + '</p>' +
      '<form class="dk-form" novalidate>' +
      '<div class="field"><span class="field__label" id="dk-found-label">On this network</span>' +
      '<div class="dk-hubs" role="group" aria-labelledby="dk-found-label" data-dk-found>' + status('busy', 'Looking for hubs&hellip;') + '</div></div>' +
      '<div class="field"><label class="field__label" for="dk-url">Hub address</label>' +
      '<input class="input" id="dk-url" name="url" type="text" inputmode="url" autocomplete="url" spellcheck="false" placeholder="192.168.1.20:8484" value="' + esc(o.url || '') + '" aria-describedby="dk-url-help">' +
      '<span class="field__help" id="dk-url-help">Where lull runs, not Jellyfin: the hub reaches Jellyfin for you. Or its Tailscale address, like https://lull.example.ts.net</span></div>' +
      '<div class="field"><label class="field__label" for="dk-code">Pair code</label>' +
      '<input class="input dk-code" id="dk-code" name="code" type="text" autocomplete="one-time-code" autocapitalize="characters" spellcheck="false" maxlength="6" placeholder="ABC123"></div>' +
      '<div class="dk-status" role="status" data-state="idle">' + status('info', 'lull checks the hub before pairing.') + '</div>' +
      '<div class="dk-actions">' + (o.phone
        ? '<button type="button" class="btn" data-dk="scan">Scan the QR code</button>'     // phones can't be hubs: no way back to the welcome page
        : '<button type="button" class="btn" data-dk="change-server">Back</button>') +
      '<button type="submit" class="btn btn--primary">Pair</button></div>' +
      '</form></section>');
  };
  function discover(card) {
    var box = $('[data-dk-found]', card);
    box.innerHTML = status('busy', 'Looking for hubs&hellip;');
    invoke('discover_hubs').then(function (hubs) {
      box.innerHTML = (hubs.length ? hubs.map(function (h) {
        return '<button type="button" class="chip" data-dk="pick-hub" data-url="' + esc(h.url) + '" aria-pressed="false">' + esc(h.name) + ' <span class="dk-hubs__host">' + esc(hostOf(h.url)) + '</span></button>';
      }).join('') : '<span class="dk-hubs__none">None found on this network. Only a computer running lull as its hub shows up here; a server you reach over Tailscale doesn\'t. ' +
        ($('[data-dk="scan"]', card) ? 'Scan the QR code from Settings &rsaquo; Devices on a paired device, or type the address below.' : 'Type its address below, with a code from Settings &rsaquo; Devices on a paired device.') + '</span>') +
        '<button type="button" class="btn btn--quiet btn--sm" data-dk="discover">Search again</button>';
    }, function (err) {
      box.innerHTML = '<span class="dk-hubs__none">' + esc(String(err)) + '</span>' +
        '<button type="button" class="btn btn--quiet btn--sm" data-dk="discover">Search again</button>';
    });
  }
  /* phone: the hub's pairing QR code is <hub url>/#/pair/<code>; no content means the listener backed out */
  function scan(card) {
    invoke('plugin:lull-player|scan').then(function (r) {
      if (r.content == null) return;
      var m = /^(https?:\/\/[^/#?]+)[^#]*#\/pair\/([A-Za-z0-9]{6})$/.exec(String(r.content || '').trim());
      if (!m) return setStatus(card, 'error', 'That QR code isn&rsquo;t a lull pair code.');
      $('[name="url"]', card).value = m[1];
      $('[name="code"]', card).value = m[2].toUpperCase();
      pair(card);
    }, function (err) { setStatus(card, 'error', esc(String(err))); });
  }
  function pair(card) {
    var urlIn = $('[name="url"]', card), codeIn = $('[name="code"]', card);
    var url = urlIn.value.trim(), code = codeIn.value.trim().toUpperCase();
    if (!url) { setStatus(card, 'error', 'Pick a hub or type its address.'); return urlIn.focus(); }
    if (code.length !== 6) { setStatus(card, 'error', 'The pair code has 6 characters.'); return codeIn.focus(); }
    var run = card._dkRun = (card._dkRun || 0) + 1;
    setStatus(card, 'busy', 'Checking ' + esc(hostOf(url) || url) + '&hellip;');
    invoke('find_hub', { url: url }).then(function (r) {
      if (run !== card._dkRun) return;
      if (r.state !== 'ok') { setStatus(card, 'error', 'Can&rsquo;t reach that hub: ' + esc(r.error)); return urlIn.focus(); }
      urlIn.value = r.url;
      setStatus(card, 'busy', 'Pairing with lull ' + esc(r.health.version) + '&hellip;');
      // on success the app loads the hub's web UI in this window
      return D.windowAction('pair', { url: r.url, code: code }).catch(function (err) {
        if (run === card._dkRun) { setStatus(card, 'error', esc(String(err))); codeIn.focus(); }
      });
    }, function (err) {
      if (run === card._dkRun) { setStatus(card, 'error', esc(String(err))); urlIn.focus(); }
    });
  }

  /* Hub didn't start (this computer is the hub): the reason, try again, or use another hub. */
  D.hubErrorPage = function (o) {
    o = o || {};
    return page('<section class="card dk-card dk-offline" aria-labelledby="dk-e-title">' +
      '<div class="empty__art" aria-hidden="true">' + STAR_ART + '<span class="sparkle"></span></div>' +
      '<h1 class="empty__title" id="dk-e-title">Your hub didn&rsquo;t start</h1>' +
      '<p class="dk-offline__text">' + esc(o.error || 'Something stopped it.') + '</p>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="change-server">Use another hub</button>' +
      '<button type="button" class="btn btn--primary" data-dk="restart-hub">Try again</button></div></section>');
  };

  /* The window's page kept crashing (recovery.rs): lull's own page instead of the engine's error page. */
  D.crashedPage = function () {
    return page('<section class="card dk-card dk-offline" aria-labelledby="dk-x-title">' +
      '<div class="empty__art" aria-hidden="true">' + STAR_ART + '<span class="sparkle"></span></div>' +
      '<h1 class="empty__title" id="dk-x-title">lull stopped working a few times in a row</h1>' +
      '<p class="dk-offline__text">The window\u2019s page crashed again each time it came back. Reload to try once more. If it keeps happening, the app\u2019s logs folder has a file called webview.log that says why.</p>' +
      '<div class="dk-actions"><button type="button" class="btn btn--primary" data-dk="restart-hub">Reload</button></div></section>');
  };

  var STAR_ART = '<svg viewBox="0 0 112 88"><path d="M56 10l9 20 22 3-16 15 4 22-19-10-19 10 4-22-16-15 22-3z"/><path d="M56 30l-4 12 6 4-5 12"/></svg>';
  function when(unix) {
    if (!unix) return 'Never';
    return new Date(unix * 1000).toLocaleString('en', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', hour12: false });
  }
  /* o = { url, error, last }: the launch probe was attempt 1 */
  D.offlinePage = function (o) {
    o = o || {};
    return '<div class="dk-page"><div class="dk-page__inner">' + wordmark() +
      '<section class="card dk-card dk-offline" aria-labelledby="dk-o-title" data-dk-offline data-attempt="1">' +
      '<div class="empty__art" aria-hidden="true">' + STAR_ART + '<span class="sparkle"></span></div>' +
      '<h1 class="empty__title" id="dk-o-title">Can&rsquo;t reach your hub</h1>' +
      '<p class="dk-offline__text">' + esc(hostOf(o.url || '')) + ' didn&rsquo;t answer. Check that your hub is running and that this computer is on its network or your Tailscale.</p>' +
      '<div class="dk-retry"><span class="dk-retry__label" aria-hidden="true"></span>' +
      '<div class="progress" aria-hidden="true"><i style="--p:0"></i></div><span class="vh" role="status" data-dk-retry-status></span></div>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="change-server">Change hub</button>' +
      '<button type="button" class="btn btn--primary" data-dk="retry">Retry now</button></div>' +
      '<details class="dk-details"><summary>' + ic('chevron-down') + 'Details</summary><dl class="dk-kv">' +
      '<dt>Address</dt><dd>' + esc(o.url || '') + '</dd><dt>Error</dt><dd data-dk-error>' + esc(o.error || '') + '</dd>' +
      '<dt>Last connected</dt><dd>' + esc(when(+o.last)) + '</dd><dt>Attempt</dt><dd class="px" data-dk-attempt>1</dd></dl></details>' +
      '</section></div></div>';
  };
  /* the unreachable page shows how reconnecting goes; the wrapper does it (app/src/reconnect.rs): it retries by itself,
     on a backoff, at once when the network changes, and whether this window shows or not, and says how each attempt went
     as `reconnect` { attempt, trying, error, nextIn }. Retry now asks it to try at once. The countdown here is only
     shown: nothing waits on this page's timer */
  function startRetry(card) {
    var label = $('.dk-retry__label', card), bar = $('.progress > i', card), status = $('[data-dk-retry-status]', card);
    var now = null, due = 0;
    function paint() {
      if (!now) return;
      if (now.trying) { label.textContent = 'trying now'; bar.style.setProperty('--p', '1'); return; }
      var left = Math.max(0, Math.ceil((due - Date.now()) / 1000));
      label.textContent = left ? 'retrying in ' + left + 's' : 'trying now';
      bar.style.setProperty('--p', String(now.nextIn ? 1 - left / now.nextIn : 1));
    }
    function show(s) {
      if (!s || !card.isConnected) return;
      var was = now;
      now = s; due = Date.now() + s.nextIn * 1000;
      card.setAttribute('data-attempt', s.attempt);
      $('[data-dk-attempt]', card).textContent = s.attempt;
      if (s.error) $('[data-dk-error]', card).textContent = s.error;
      if (!s.trying && s.attempt > 1 && (!was || was.attempt !== s.attempt)) status.textContent = 'Still can\'t reach your hub. Attempt ' + s.attempt + '.';
      paint();
    }
    card._dkRetry = function () {
      label.textContent = 'trying now'; bar.style.setProperty('--p', '1');
      D.windowAction('retry').catch(logFailure('retry'));
    };
    card._dkTimer = setInterval(paint, 1000);
    if (!T) return;
    T.event.listen('reconnect', function (e) { show(e.payload); });
    invoke('reconnect_status').then(show, logFailure('reconnect status'));
  }

  /* =====================================================================
     3. Close to tray: asked once, when the wrapper says so (DESKTOP.md 1). The answer is the wrapper's to keep.
     ===================================================================== */
  D.askClose = function () {
    if ($('.dk-modal')) return;
    var opener = doc.activeElement;
    var host = doc.createElement('div');
    host.className = 'dk-modal';
    host.innerHTML = '<div class="dk-modal__scrim"></div>' +
      '<section class="card dk-card" role="dialog" aria-modal="true" aria-labelledby="dk-m-title" aria-describedby="dk-m-text">' +
      '<h2 class="dk-card__title" id="dk-m-title">keep playing in the tray?</h2>' +
      '<p id="dk-m-text">lull keeps playing with the window closed. Open it again or quit from the tray icon.</p>' +
      '<label class="switch-row"><span class="switch-row__label">Don&rsquo;t ask again</span><button type="button" class="switch" role="switch" aria-checked="true" data-dk="switch"></button></label>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk-close="quit">Quit lull</button>' +
      '<button type="button" class="btn btn--primary" data-dk-close="keep">Keep playing</button></div></section>';
    doc.body.appendChild(host);
    var focusables = $$('button', host);
    focusables[focusables.length - 1].focus();
    function close(choice) {
      var remember = $('[role="switch"]', host).getAttribute('aria-checked') === 'true';
      host.remove(); doc.removeEventListener('keydown', onKey, true);
      if (opener && opener.focus) opener.focus();
      if (choice) D.windowAction('close-choice', { keep: choice === 'keep', remember: remember }).catch(logFailure('close'));
    }
    function onKey(e) {
      if (e.key === 'Escape') { e.preventDefault(); close(null); }
      else if (e.key === 'Tab') {                      // focus trap
        var first = focusables[0], last = focusables[focusables.length - 1];
        if (e.shiftKey && doc.activeElement === first) { e.preventDefault(); last.focus(); }
        else if (!e.shiftKey && doc.activeElement === last) { e.preventDefault(); first.focus(); }
      }
    }
    doc.addEventListener('keydown', onKey, true);
    host.addEventListener('click', function (e) {
      var b = e.target.closest('[data-dk-close]');
      if (b) close(b.getAttribute('data-dk-close'));
      else if (e.target.classList.contains('dk-modal__scrim')) close(null);
    });
  };

  /* =====================================================================
     4. Mini player: shows `main`'s player view (web/player-view.js, ADR-0041). `main` sends the view itself (section 8) as
        the 'player' event and the cover on its own ('player-art'); the song is placed with PlayerView.posAt, so this
        window's clock runs between views. PlayerView.paint writes every hook that changes during a song, as it does
        in the web UI: the markup below draws the song alone. The buttons go back to `main` as 'mini' events and it runs
        them as its own actions.
        The link has a version and two ends that each say hello: `hello` carries PlayerView.VERSION, and main answers
        every hello with its view; main says `main-hello` when its page starts (this window then says hello again) and
        `main-bye` when it goes (its page hides, or the app sees its renderer die). data-link on [data-dk-player]:
        ok, away (after main-bye, until a view comes), update-app (main's view is newer than this app) or
        update-hub (older, or from before the version): the clock stops and the window says which side to update.
     ===================================================================== */
  var V = window.PlayerView;
  var FALLBACK_ART = '../assets/fallback-art-256.png';
  var BARS = 40;
  var coverArt = null;                // the cover comes on its own ('player-art'): once per song, never inside the view
  var latest = null;               // the last view main sent that this window can draw (the placeholder until the first)
  var link = 'ok', liveValues;   // data-link, and the live model's values main sent with the view
  var LINK_WORDS = { away: 'lull’s window is away', 'update-app': 'Update the lull app', 'update-hub': 'Update your hub' };
  var LINK_ERROR = {
    'update-app': 'lull mini player: main’s view is newer than this app understands. Update the lull app.',
    'update-hub': 'lull mini player: main’s view is older than this app, or from before the view had a version. Update your hub.'
  };
  /* what the window shows before main's first view: nothing playing, built the way main builds a view */
  function idleView() {
    return V.derive({ track: null, status: 'idle', intent: false, sound: null, words: '', copy: null, pos: 0, duration: 0, durationKnown: true,
      received: 1, front: 1, parts: [], buffered: [], seekWait: null, liked: false, saved: false, shuffle: false, repeat: 'off',
      volume: 1, muted: false, crossfading: false, now: Date.now() });
  }
  /* what the window draws: the view, and while the link isn't ok, why in the status line's place. Nothing it knows says the
     song plays then, so it is a pause: the key, the wave and data-playing show none of it playing */
  function shown() {
    return link === 'ok' ? latest : Object.assign({}, latest, { words: LINK_WORDS[link], activity: 'paused', intent: false });
  }
  function barHeight(i) { return (0.32 + 0.6 * Math.abs(Math.sin(i * 1.7) * Math.cos(i * .43))).toFixed(2); }
  function chip(copy) {
    return copy && copy.format ? '<span class="q-chip ' + (copy.lossless ? 'q-chip--ll' : 'q-chip--ly') + ' q-chip--solid" title="' + (copy.lossless ? 'Lossless, ' : '') + esc(copy.format) + '">' + esc(copy.format) + '</span>' : '';
  }
  function cover(size) {
    return '<span class="cover dk-art' + (size <= 64 ? ' cover--thumb' : '') + '" style="--cover-size-base:' + size + 'px" aria-hidden="true"><img alt="" src="' + esc(coverArt || FALLBACK_ART) + '"></span>';
  }
  function waveMarkup() {
    var b = ''; for (var i = 0; i < BARS; i++) b += '<i class="wave__bar" style="--h:' + barHeight(i) + '"></i>';
    return '<div class="wave" role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax="100" data-dk="seek">' + b +
      '<div class="wave__rail"><div class="wave__head"></div></div></div>';
  }
  /* the default markup's status line (data-dk-status): shown only when there is news, in the place of the line it replaces
     (the facts here, the artist in the strip); a design's own slot places its line as it likes */
  var STATUS = '<div class="device__state" data-dk-status hidden></div>';
  /* the song: its cover, title, artist, and the copy that plays (the bar's format; the peer's share while it arrives).
     `withStatus`: the default device's, which has its status line beside the facts */
  function nowMarkup(withStatus) {
    var t = latest.track, status = withStatus ? STATUS : '';
    /* with no song there are no facts: the status takes the hint's line, at its height */
    if (!t) return '<div class="dk-mini__now">' + cover(84) + '<div><div class="device__title">Nothing playing</div><div class="device__artist"><span class="dk-mini__hint">Pick something in lull</span>' +
      (withStatus ? '<span class="dk-mini__hint-state" data-dk-status hidden></span>' : '') + '</div></div></div>';
    return '<div class="dk-mini__now">' + cover(84) + '<div><div class="device__title">' + esc(t.title) + '</div>' +
      '<div class="device__artist">' + esc(t.artist) + '</div><div class="device__facts">' + chip(latest.copy) +
      (latest.transfer.received < 1 ? '<span class="device__fact">peer <span data-bind="received"></span></span>' : '') + '</div>' + status + '</div></div>';
  }

  /* miniPlayer slot: a design.js may replace the window content. ctx = { style: 'device'|'strip', opaque,
     track { title, artist, format, lossless } | null, playing, liked, parts { device(), strip(), cover(size), wave() } }.
     Keep data-dk-player and the data-dk="..." buttons / data-dk-* fields in an override: section 5 drives those. */
  var miniOpts = {};
  function miniPlayer(style) {
    var t = latest.track, copy = latest.copy;
    var ctx = { style: style, opaque: !!miniOpts.opaque, playing: V.pressed(latest), liked: latest.liked,
      track: t ? { title: t.title, artist: t.artist, format: copy && copy.format || '', lossless: !!(copy && copy.lossless) } : null,
      parts: { device: deviceMarkup, strip: stripMarkup, cover: cover, wave: waveMarkup } };
    var dflt = function (c) { return c.style === 'strip' ? stripMarkup() : deviceMarkup(); };
    return window.Designs ? String(Designs.slot('miniPlayer', ctx, dflt)) : dflt(ctx);
  }
  D.miniDevice = function () { return miniPlayer('device'); };
  D.miniStrip = function () { return miniPlayer('strip'); };
  function deviceMarkup() {
    var grill = ''; for (var g = 0; g < 9; g++) grill += '<i></i>';
    return '<div class="dk-mini' + (miniOpts.opaque ? ' dk-mini--opaque' : '') + '" data-dk-player="device" data-link="ok">' +
      '<section class="device" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="dk-mini__head" data-tauri-drag-region><span class="dk-mini__led" aria-hidden="true"><i></i>on</span><span class="spacer" data-tauri-drag-region></span>' +
      '<button class="dk-mini__win" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="dk-mini__win" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button></div>' +
      '<div class="device__bezel" data-tauri-drag-region><div class="device__screen s-screen">' +
      '<div class="device__page"><div class="device__page-head"><span>now playing</span><span data-dk-vol></span></div>' +
      '<div class="dk-mini__now-host" data-dk-now>' + nowMarkup(true) + '</div>' + waveMarkup() +
      '<div class="wave-times"><b data-dk-pos></b><span data-dk-dur></span></div></div>' +
      '<div class="dk-mini__note" aria-hidden="true"></div></div>' +
      '<div class="device__brand">' + wordmark() + '</div></div>' +
      '<div class="device__controls"><div class="dpad">' +
      '<button class="dpad__btn dpad__btn--up" data-dk="vol-up" aria-label="Volume up" data-tip="Volume up">' + ic('plus') + '</button>' +
      '<button class="dpad__btn dpad__btn--left" data-dk="previous" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<span class="dpad__hub"></span>' +
      '<button class="dpad__btn dpad__btn--right" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="dpad__btn dpad__btn--down" data-dk="vol-down" aria-label="Volume down" data-tip="Volume down">' + ic('minus') + '</button></div>' +
      '<div class="device__ab"><span class="device__key device__key--b"><button class="btn-gloss btn-gloss--48" data-dk="like" aria-label="Like" data-tip="Like">' + ic('heart') + '</button>like</span>' +
      '<span class="device__key device__key--a"><button class="btn-gloss" data-dk="play-pause" aria-label="Play" data-tip="Play">' + ic('play', 'ic-play') + '</button><span data-dk-play-label></span></span></div></div>' +
      '<div class="device__pills"><button class="device__pill device__pill--shuffle" data-dk="shuffle" aria-pressed="false"><i></i>shuffle</button>' +
      '<button class="device__pill device__pill--repeat" data-dk="repeat" aria-pressed="false"><i></i>repeat</button></div>' +
      '<div class="device__grill" aria-hidden="true">' + grill + '</div></section></div>';
  }
  function stripMarkup() {
    var t = latest.track;
    return '<div class="dk-strip-wrap" data-dk-player="strip" data-link="ok"><section class="dk-strip s-panel" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="seek seek--line" aria-hidden="true"><div class="seek__track"><div class="seek__xfer"></div><div class="seek__bufs"><div class="seek__buf" data-dk-buf></div></div><div class="seek__played"></div></div></div>' +
      '<button class="bezel-thumb" data-dk="full" aria-label="Open full window" data-tip="Open full window" data-dk-thumb>' + cover(40) + '</button>' +
      '<div class="dk-strip__text" data-tauri-drag-region><div class="dk-strip__title" data-dk-title>' + esc(t ? t.title : 'Nothing playing') + '</div><div class="dk-strip__artist" data-dk-artist>' + esc(t ? t.artist : '') + '</div><div class="dk-strip__state" data-dk-status hidden></div></div>' +
      '<div class="dk-strip__keys">' +
      '<button class="btn-icon btn-icon--bare" data-dk="previous" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<button class="btn-gloss btn-gloss--40" data-dk="play-pause" aria-label="Play" data-tip="Play">' + ic('play', 'ic-play') + '</button>' +
      '<button class="btn-icon btn-icon--bare" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="like" aria-label="Like" data-tip="Like">' + ic('heart') + '</button>' +
      '<span class="dk-sep" aria-hidden="true"></span>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button>' +
      '</div></section></div>';
  }

  /* the window's hooks, from the view at `now`; the link's state beside them. The default markup's status lines show
     only when there is news: the player is waiting, has to ask, failed, or the link isn't ok */
  var NEWS = { loading: 1, choose: 1, error: 1 };
  function paint(scope) {
    var view = shown(), news = link !== 'ok' || !!NEWS[view.activity];
    $$('[data-dk-player]', scope).forEach(function (el) { if (el.getAttribute('data-link') !== link) el.setAttribute('data-link', link); });
    $$('.device__state, .dk-strip__state, .dk-mini__hint-state', scope).forEach(function (el) { if (el.hidden === news) el.hidden = !news; });
    V.paint(scope, view, Date.now());
  }
  /* the song's own parts: drawn again when the song or its copy changes, or the share starts or stops arriving */
  var songKey = function (v) { return JSON.stringify([v.track, v.copy && [v.copy.format, v.copy.lossless], v.transfer.received < 1]); };
  function paintSong(scope) {
    var t = latest.track;
    $$('[data-dk-now]', scope).forEach(function (n) { n.innerHTML = nowMarkup(n.classList.contains('dk-mini__now-host')); });
    $$('[data-dk-thumb]', scope).forEach(function (n) { n.innerHTML = cover(40); });
    $$('[data-dk-title]', scope).forEach(function (n) { n.textContent = t ? t.title : 'Nothing playing'; });
    $$('[data-dk-artist]', scope).forEach(function (n) { n.textContent = t ? t.artist : ''; });
  }
  /* `says`: one view's changes in words, announced together (once) by the caller instead of by each note */
  function note(scope, text, ms, says) {
    $$('.dk-mini__note', scope).forEach(function (n) {
      n.textContent = text; n.classList.add('is-on');
      clearTimeout(n._t); n._t = setTimeout(function () { n.classList.remove('is-on'); }, ms || 1600);
    });
    if (says) says.push(text); else announce(text);
  }

  /* the live model's values the theme names: main's, with the song's place worked out here from the view's clock (the
     place moves with the song, so main sends none of it) */
  var hundredths = function (f) { return Math.floor(Math.max(0, Math.min(1, f || 0)) * 100) / 100; };
  function feedLive() {
    if (!window.Live) return;
    var c = latest.clock, pos = V.posAt(latest, Date.now());
    Live.feed(Object.assign({}, liveValues, {
      'song.pos': latest.track && c.duration ? hundredths(pos / c.duration) : 0, 'song.elapsed': latest.track ? V.time(pos) : '',
      'song.buffered': latest.track ? hundredths(latest.transfer.buffered) : 0, 'song.received': latest.track ? hundredths(latest.transfer.received) : 0
    }));
  }
  /* the clock stops where it is: the window is away, or main's view isn't one it can draw */
  function stopClock() {
    var now = Date.now();
    latest = Object.assign({}, latest, { clock: Object.assign({}, latest.clock, { pos: V.posAt(latest, now), at: now, rate: 0 }) });
  }
  /* which side a view's version says to update; null while it is this window's own */
  function staleSide(v) {
    if (!v || typeof v.v !== 'number' || v.v < V.VERSION) return 'update-hub';
    return v.v > V.VERSION ? 'update-app' : null;
  }
  var NOTICED = { error: 1, choose: 1, ended: 1 };
  var REPEAT_TEXT = { off: 'repeat off', all: 'repeat all', one: 'repeat one' };
  var heardMain = false;
  /* a view from main: draw it, and say what the last button press changed. Until main's first view the window shows the
     placeholder: that first one is the player as it already was, not a change to say. */
  function receive(scope, next) {
    var side = staleSide(next);
    if (side) {
      if (link !== side) { console.error(LINK_ERROR[side]); note(scope, LINK_WORDS[side], 8000); }
      stopClock(); link = side; paint(scope); feedLive(); return;
    }
    var prev = latest;
    latest = next; liveValues = next.live;
    if (songKey(prev) !== songKey(next)) paintSong(scope);
    link = 'ok'; paint(scope); feedLive();
    if (!heardMain) { heardMain = true; return; }
    var says = [];
    /* the song, unless the slot's own live region on it says it (asked now: the slot may gain or lose it as it likes) */
    if (next.track && (!prev.track || prev.track.id !== next.track.id) && !$('[data-dk-now][aria-live]:not([aria-live="off"])')) says.push(next.track.title + ', ' + next.track.artist);
    /* news the listener didn't cause and can't see coming, said once on the change: a failure, a copy to pick, the queue's end.
       The routine moves (loading, playing, paused) are heard and seen already: no live region reads out each step of them */
    if (prev.activity !== next.activity && NOTICED[next.activity]) says.push(next.words || next.activity);
    var was = prev.muted ? 0 : prev.volume, now = next.muted ? 0 : next.volume;
    if (was !== now) note(scope, 'volume ' + Math.round(now * 100) + '%', 0, says);
    if (prev.shuffle !== next.shuffle) note(scope, next.shuffle ? 'shuffle on' : 'shuffle off', 0, says);
    if (prev.repeat !== next.repeat) note(scope, REPEAT_TEXT[next.repeat] || 'repeat', 0, says);
    if (says.length) announce(says.join('. '));
  }
  /* main's page is gone (hidden, reloading, its renderer dead) until it says hello again with a view */
  function mainAway(scope) { var said = link === 'away'; stopClock(); link = 'away'; paint(scope); feedLive(); if (!said) announce(LINK_WORDS.away); }
  /* the song moves on between views: this window's clock, while main's view says the song runs */
  function tickClock() {
    if (link !== 'ok' || latest.clock.rate !== 1) return;
    paint(doc); feedLive();
  }
  function toMain(action, detail) {
    T.event.emitTo('main', 'mini', Object.assign({ action: action }, detail || {})).catch(logFailure('mini ' + action));
  }

  /* =====================================================================
     5. One delegated handler for everything above
     ===================================================================== */
  /* The mini player's keys: the data-dk="..." names a button in [data-dk-player] may carry, and who runs each: "main"
     (sent as a 'mini' event; main's run() below), "window" (this window's own, windowAction) or "seek" (the wave, by
     pointer and arrow keys). The one list is mini-keys.json beside this file, which lull-themes reads too, so a slot
     naming any other key is refused (crates/themes/src/package.rs). The line below is written from it: edit the JSON,
     then LULL_WRITE_PACKAGES=1 cargo test -p lull-themes */
  var MINI_KEYS = {"full":"window","like":"main","mini-close":"window","next":"main","play-pause":"main","previous":"main","repeat":"main","save":"main","seek":"seek","shuffle":"main","vol-down":"main","vol-up":"main"};
  function seekTo(fraction) { toMain('seek', { fraction: Math.min(1, Math.max(0, fraction)) }); }

  doc.addEventListener('click', function (e) {
    /* any of the page's actions, on a key of a theme's own in the mini player (data-act, with its data-* attributes):
       main runs it as the page would (App.run) */
    var page = e.target.closest('[data-dk-player] [data-act]');
    if (page && !page.closest('[data-dk]')) {
      var attrs = {};
      Array.prototype.forEach.call(page.attributes, function (a) { if (a.name.indexOf('data-') === 0) attrs[a.name] = a.value; });
      return toMain('act', { name: page.getAttribute('data-act'), attrs: attrs });
    }
    var el = e.target.closest('[data-dk]'); if (!el) return;
    var act = el.getAttribute('data-dk'), scope = el.closest('[data-dk-player]');
    if (scope && MINI_KEYS[act] === 'main') return toMain(act);
    var card = el.closest('[data-dk-connect]');
    switch (act) {
      case 'minimize': case 'maximize': case 'close': case 'full': case 'mini-close': case 'change-server': case 'connect':
        return D.windowAction(act).catch(logFailure(act));
      case 'restart-hub': return D.windowAction('retry').catch(logFailure('restart hub'));
      case 'this-computer': return useThisComputer(el.closest('[data-dk-welcome]'));
      case 'discover': return discover(card);
      case 'scan': return scan(card);
      case 'pick-hub':
        $$('[data-dk="pick-hub"]', card).forEach(function (b) { b.setAttribute('aria-pressed', String(b === el)); });
        $('[name="url"]', card).value = el.getAttribute('data-url');
        return $('[name="code"]', card).focus();
      case 'switch': el.setAttribute('aria-checked', String(el.getAttribute('aria-checked') !== 'true')); return;
      case 'retry': var off = el.closest('[data-dk-offline]'); if (off && off._dkRetry) off._dkRetry(); return;
    }
  });
  doc.addEventListener('submit', function (e) {
    var card = e.target.closest('[data-dk-connect]'); if (!card) return;
    e.preventDefault();
    pair(card);
  });
  /* the wave scrubs like the main window's: the playhead follows the pointer while the button is down (painted here
     at once, main seeks once a frame), not only where the button comes up. Main's next view has the truth (a
     Soulseek song can't seek past what has arrived) */
  doc.addEventListener('pointerdown', function (e) {
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w || e.button !== 0 || !latest.clock.duration) return;
    e.preventDefault(); w.focus({ preventScroll: true });
    try { w.setPointerCapture(e.pointerId); } catch (err) { /* synthetic events */ }
    var f = 0, frame = 0;
    function at(ev) { var r = w.getBoundingClientRect(); return Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width)); }
    function show() {
      frame = 0;
      latest = Object.assign({}, latest, { clock: Object.assign({}, latest.clock, { pos: f * latest.clock.duration, at: Date.now() }) });
      paint(doc); seekTo(f);
    }
    function move(ev) { f = at(ev); if (!frame) frame = requestAnimationFrame(show); }
    function end(ev) {
      w.removeEventListener('pointermove', move); w.removeEventListener('pointerup', end); w.removeEventListener('pointercancel', end);
      cancelAnimationFrame(frame);
      if (ev.type === 'pointerup') { f = at(ev); show(); }
    }
    move(e);
    w.addEventListener('pointermove', move); w.addEventListener('pointerup', end); w.addEventListener('pointercancel', end);
  });
  doc.addEventListener('keydown', function (e) {
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w || !latest.clock.duration) return;
    var step = e.key === 'ArrowRight' ? 5 : e.key === 'ArrowLeft' ? -5 : e.key === 'Home' ? -1e9 : e.key === 'End' ? 1e9 : 0;
    if (!step) return;
    e.preventDefault(); seekTo((V.posAt(latest, Date.now()) + step) / latest.clock.duration);
  });
  /* typing an address unpicks the hub chip that filled it */
  doc.addEventListener('input', function (e) {
    var card = e.target.closest && e.target.closest('[data-dk-connect]');
    if (card && e.target.name === 'url') $$('[data-dk="pick-hub"]', card).forEach(function (b) { b.setAttribute('aria-pressed', 'false'); });
  });

  /* =====================================================================
     6. Tooltips when the web UI's App isn't on the page (the bundled pages)
     ===================================================================== */
  if (!window.App) {
    var tip = null;
    var showTip = function (e) {
      var el = e.target.closest && e.target.closest('[data-tip]'); if (!el) return;
      if (!tip) { tip = doc.createElement('div'); tip.className = 'tooltip dk-tip'; tip.setAttribute('aria-hidden', 'true'); doc.body.appendChild(tip); }
      tip.textContent = el.getAttribute('data-tip');
      var r = el.getBoundingClientRect(), tw = tip.offsetWidth, th = tip.offsetHeight;
      var top = r.bottom + 6 + th > innerHeight ? r.top - th - 6 : r.bottom + 6;
      tip.style.left = Math.max(4, Math.min(innerWidth - tw - 4, r.left + r.width / 2 - tw / 2)) + 'px';
      tip.style.top = top + 'px'; tip.classList.add('is-on');
    };
    var hideTip = function () { if (tip) tip.classList.remove('is-on'); };
    doc.addEventListener('mouseover', showTip); doc.addEventListener('focusin', showTip);
    doc.addEventListener('mouseout', hideTip); doc.addEventListener('focusout', hideTip);
    doc.addEventListener('click', hideTip);
  }

  /* =====================================================================
     7. wrapper.html: render the view the wrapper named in the hash (desktop/src/pages.rs)
     ===================================================================== */
  D.mountPage = function () {
    var q = new URLSearchParams(location.hash.slice(1)), view = q.get('view'), host = $('#dk-root');
    var pages = {
      welcome: ['Welcome to lull', function () { return D.welcomePage(); }],
      connect: ['Connect to your hub', function () { return D.connectPage({ url: q.get('url'), unpaired: !!q.get('unpaired'), phone: !!q.get('phone') }); }],
      offline: ['Can’t reach your hub', function () { return D.offlinePage({ url: q.get('url'), error: q.get('error'), last: q.get('last') }); }],
      'hub-error': ['Your hub didn’t start', function () { return D.hubErrorPage({ error: q.get('error') }); }],
      crashed: ['lull stopped working', function () { return D.crashedPage(); }]
    };
    if (pages[view]) {
      $$('[data-dk-offline]').forEach(function (c) { clearInterval(c._dkTimer); });   // leaving the unreachable page
      doc.title = pages[view][0];
      host.innerHTML = pages[view][1]();
      attachConfigured();
      $$('[data-dk-offline]').forEach(startRetry);
      $$('[data-dk-connect]').forEach(discover);
      var first = $('[data-dk-connect] [name="code"]') || $('.dk-choice'); if (first) first.focus();
      /* main's own pages: the window its theme asks for, as the web UI's page would (the theme came from main) */
      if (T) Designs.ready(function () {
        var request = Designs.windowRequest();
        if (request) invoke('window_glass', { request: request }).then(showGlass, logFailure('glass'));
      });
    } else if (view === 'mini' || view === 'strip') {
      doc.title = 'lull mini player';
      miniOpts = { opaque: !!q.get('opaque') };
      latest = idleView();
      var render = function () {
        host.innerHTML = view === 'strip' ? D.miniStrip() : D.miniDevice();
        paintSong(doc); paint(doc); feedLive();
        fitWindow($('[data-dk-player]', host));
      };
      Designs.ready(render);
      T.event.listen('player', function (e) { receive(doc, e.payload); });
      T.event.listen('player-art', function (e) { if (coverArt !== e.payload.art) { coverArt = e.payload.art; paintSong(doc); paint(doc); } });
      /* DESKTOP.md 6.1: follow the web UI live. A listener's theme comes from main (theme-package, once main has
         loaded it, and again whenever it changes on the hub); until then the design it extends shows. */
      var want = { design: q.get('design'), base: q.get('base') };
      var show = function () {
        if (want.design === Designs.current) return;
        var stopgap = function () { if (want.base && want.base !== Designs.current) Designs.set(want.base, render); };
        if (Designs.set(want.design, render, stopgap) === false) stopgap();
      };
      T.event.listen('appearance', function (e) {
        Designs.applyTheme(e.payload.theme);
        want = { design: e.payload.design, base: e.payload.base || null };
        show();
      });
      T.event.listen('theme-package', function (e) {          // a copy kept from before is out of date now
        var id = e.payload.id;
        Designs.reload(id).then(function (inUse) {
          if (inUse) render(); else if (id === want.design) show();
        }, logFailure('theme ' + id));
      });
      setInterval(tickClock, 250);
      T.event.listen('main-hello', function () { toMain('hello', { v: V.VERSION }); }).catch(logFailure('main hello'));   // main reloaded: it learns this window is open
      T.event.listen('main-bye', function () { mainAway(doc); }).catch(logFailure('main bye'));
      toMain('hello', { v: V.VERSION });                   // main answers with its view
      window.addEventListener('pagehide', function () { toMain('bye'); });   // main stops sending views
      if (q.get('wayland')) setTimeout(function () {
        note(doc, 'Your desktop decides what stays on top: use the window menu (Alt+Space) > Always on Top.', 8000);
      }, 400);
    }
  };

  /* the mini window is as tall as its design's player, which may differ from the window's opening size (mini.rs);
     its width is the design's own. Follows the player as it changes (a title on two lines, a design switch). */
  var fitting = null;
  function fitWindow(el) {
    if (fitting) fitting.disconnect();
    var w = currentWindow(), LogicalSize = (T.dpi || T.window).LogicalSize;
    /* the width its theme's CSS asks for (--layout-mini-width on the player, ADR-0101 amended, E), within the screen */
    var want = parseFloat(getComputedStyle(el).getPropertyValue('--layout-mini-width')) || 0;
    var width = want ? Math.round(Math.min(Math.max(want, 200), screen.availWidth || want)) : 0;
    fitting = new ResizeObserver(function () {
      var h = Math.ceil(el.getBoundingClientRect().height), wd = width || innerWidth;
      if (h && (h !== innerHeight || wd !== innerWidth)) w.setSize(new LogicalSize(wd, h)).catch(logFailure('mini player size'));
    });
    fitting.observe(el);
  }

  /* the app moves between its pages by changing the hash: same document, so render again */
  if ($('#dk-root')) window.addEventListener('hashchange', D.mountPage);

  /* =====================================================================
     8. Bridge in the web UI (wrapper only; DESKTOP.md 4 and 6.1). Tray items and mini-player buttons run
        the web UI's own actions; set_now_playing keeps the tray, the taskbar buttons and Discord current;
        'player' views feed the mini player; 'appearance' tells the wrapper which design and theme its own
        pages should use. While Classic mode's window plays, what it reports is what's playing.
     ===================================================================== */
  if (window.App && App.player && D.inWrapper) {
    var P = App.player;
    var classicNow = null;                                 // { id, playing, pos, duration } from Webamp's window
    var playingId = function () { return classicNow ? classicNow.id : P.currentId(); };
    var run = function (a, detail) {
      var id = playingId(), ps = P.state;
      if (App.sounds) App.sounds.pressed();              // a press in the tray or a mini player: the listener's (sounds.js)
      switch (a) {
        case 'play-pause': return P.toggle();
        case 'next': return P.next();
        case 'previous': return P.prev();
        case 'stop': return P.pause();
        case 'like': return id && App.toggleLike(id);
        case 'save': return id && !App.isSaved(id) && App.saves.start([id]);
        case 'classic': return App.go('/classic');
        case 'shuffle': return P.setShuffle();
        case 'repeat': return P.cycleRepeat();
        case 'vol-up': return P.setVolume(ps.volume + .1);
        case 'vol-down': return P.setVolume(ps.volume - .1);
        case 'seek': return typeof detail.fraction === 'number' && P.seekFraction(detail.fraction);
        case 'act': return typeof detail.name === 'string' && (App.run(detail.name, detail.attrs) || console.warn('lull desktop: unknown action', detail.name));
        case 'hello': miniOpen = true; sendArt(true); return sendView(P.view());
        case 'bye': miniOpen = false; return;            // also sent by the app when it destroys the mini player
        default: console.warn('lull desktop: unknown action', a);
      }
    };
    T.event.listen('tray', function (e) { run(e.payload.action, e.payload); });
    /* the mini player's hello carries its version, and main answers every one with its view: the mini player judges it.
       Main says hello itself as its page starts, once it listens, so a mini player left open over a reload (a crash,
       the unreachable page, a reconnect) says hello again; and bye as it goes (the app says it too when main's
       renderer dies, app/src/desktop/recovery.rs) */
    T.event.listen('mini', function (e) { run(e.payload.action, e.payload); })
      .then(function () { return T.event.emit('main-hello', {}); }).catch(logFailure('main hello'));
    window.addEventListener('pagehide', function () { T.event.emit('main-bye', {}).catch(logFailure('main bye')); });
    T.event.listen('ask-close', function () { D.askClose(); });
    T.event.listen('classic', function (e) {
      var m = e.payload || {};
      if (m.type === 'now') classicNow = m;
      else if (m.type === 'back') classicNow = null;
      else if (m.type === 'action') run(m.action);
      if (m.type === 'now' || m.type === 'back') sendNow();
      classicHears.forEach(function (fn) { fn(m); });
    });

    /* what plays: the app engine, or Webamp's window while Classic mode is open */
    var playingNow = function () {
      if (classicNow) {
        return { track: classicNow.id ? App.data.track(classicNow.id) : null, playing: !!classicNow.playing,
          pos: classicNow.pos || 0, duration: classicNow.duration || 0 };
      }
      return { track: P.current(), playing: !!P.state.playing, pos: P.state.pos || 0, duration: P.duration() || 0 };
    };
    var lastNow = '';
    var sendNow = function () {
      var p = playingNow(), t = p.track, id = t ? t.id : null;
      var now = { title: t ? t.title : '', artist: t ? t.artist : '', playing: p.playing,
        liked: !!(id && App.isLiked(id)), saved: !!(id && App.isSaved(id)),
        connected: !App.banner.list().some(function (b) { return b.id === 'sys-unreachable'; }),
        id: id || '', artistId: t && t.artistId || '', albumId: t && t.albumId || '', album: t && t.album || '',
        position: p.pos, duration: p.duration };
      /* the position moves every second: a new report only when it jumped (a seek), not as it plays */
      var started = p.playing ? Math.round((Date.now() / 1000 - p.pos) / 3) : 0;
      var key = JSON.stringify([now.title, now.artist, now.playing, now.liked, now.saved, now.connected, id, now.artistId,
        now.albumId, now.album, Math.round(now.duration), started]);
      if (key === lastNow) return; lastNow = key;
      invoke('set_now_playing', { now: now }).catch(logFailure('set_now_playing'));
    };

    /* cover art: the largest Media Session artwork, as a data URL (the mini player's origin can't load the server's covers) */
    var art = { src: null, data: null };
    var artFor = function () {
      var m = navigator.mediaSession && navigator.mediaSession.metadata, list = m && m.artwork ? m.artwork : [];
      var best = list.reduce(function (a, b) { return !a || parseInt(b.sizes, 10) > parseInt(a.sizes, 10) ? b : a; }, null);
      var src = best ? new URL(best.src, location.href).href : null;
      if (src === art.src) return art.data;
      art = { src: src, data: null };
      if (src) fetch(src, { credentials: 'same-origin' }).then(function (r) { return r.ok ? r.blob() : null; }).then(function (blob) {
        if (!blob || art.src !== src) return;
        var reader = new FileReader();
        reader.onload = function () { if (art.src === src) { art.data = reader.result; sendArt(false); } };
        reader.readAsDataURL(blob);
      }).catch(logFailure('cover art'));
      return null;
    };
    /* what the mini player is sent: the player's view as it is (web/player-view.js, ADR-0041), with the live model's values
       the theme names (designs/live.js, ADR-0101), as this page read them. The song's place is no part of them: the mini
       player works it out from the view's clock, as every surface does. None for a design that names none */
    var PLACE = ['song.pos', 'song.elapsed', 'song.buffered', 'song.received'];
    var liveNow = function () {
      if (!window.Live) return undefined;
      var o = Object.assign({}, Live.values());
      PLACE.forEach(function (k) { delete o[k]; });
      return o;
    };
    /* the cover goes on its own, once per song: inside the view, a 100 KB data URL crossed to the mini player with
       every change, and the page's memory grew by tens of MB a minute */
    var sentArt;
    var sendArt = function (force) {
      if (!miniOpen) return;
      var data = P.current() ? artFor() : null;
      if (data === null && art.src && P.current()) return;   // still fetching this song's: keep showing the last one
      if (!force && data === sentArt) return; sentArt = data;
      T.event.emit('player-art', { art: data }).catch(logFailure('cover art'));
    };
    /* views feed only the mini player: none while it's closed ('hello' when it opens, 'bye' when it goes). The page's player
       publishes a view when its key changes or its clock strays, hidden or not (the `view` event, app.js), and each goes
       on as it is: while the song plays none is needed for the position. A clock of this page's checks every second for
       what changes without one (the live model's values, the cover). */
    var liveSent = '', miniOpen = false;
    var sendView = function (view) {
      if (!miniOpen) return;
      var live = liveNow(); liveSent = JSON.stringify(live || null);
      T.event.emit('player', live ? Object.assign({}, view, { live: live }) : view).catch(logFailure('player view'));
    };
    setInterval(function () {
      if (!miniOpen) return;
      sendArt(false);
      if (JSON.stringify(liveNow() || null) !== liveSent) sendView(P.view());
    }, 1000);
    var sendAppearance = function () {
      var design = App.design(), base = Designs.designOf(design);
      T.event.emit('appearance', { design: design, base: base !== design ? base || undefined : undefined, theme: App.prefs().theme })
        .catch(logFailure('appearance'));
    };
    /* the listener's theme in use, for the app's own pages (mini player, unreachable...), which can't fetch it from
       the hub (app/src/desktop/theme.rs): once loaded, on every switch, and when it changes on the hub */
    var sentPackage;
    var sendPackage = function () {
      var design = App.design();
      Designs.packageOf(design).then(function (p) {
        if (p === sentPackage || design !== App.design()) return; sentPackage = p;   // a later switch sends its own
        return invoke('set_theme_package', { package: p });
      }).catch(logFailure('theme package'));
    };

    /* the theme's window: asked again whenever the theme or its mode changes. The answer, and every later change (the
       listener's switch, Windows' transparency effects), sets :root[data-window]. A theme that wants a solid window
       gets its wallpaper back first, so a solid window never shows a frame of see-through page. */
    var askedGlass;
    var askGlass = function () {
      var request = Designs.windowRequest(), key = JSON.stringify(request);
      if (key === askedGlass) return; askedGlass = key;
      if (!request) Designs.setWindow(null);
      invoke('window_glass', { request: request }).then(showGlass, logFailure('glass'));
    };
    T.event.listen('glass', function (e) { showGlass(e.payload); });
    Designs.onLook(askGlass);
    askGlass();

    ['player', 'like', 'save', 'ready'].forEach(function (ev) { App.on(ev, sendNow); });
    App.on('view', function (view) { sendArt(false); sendView(view); });
    ['design', 'prefs'].forEach(function (ev) { App.on(ev, sendAppearance); });
    App.on('design', sendPackage);
    App.on('ready', reattach); App.on('design', reattach); App.on('route', markDragRegions);
    doc.addEventListener('keydown', function (e) {        // Ctrl+Shift+M: mini player
      if (e.ctrlKey && e.shiftKey && !e.altKey && (e.key === 'M' || e.key === 'm')) { e.preventDefault(); D.openMini(); }
    });
    if (!('mediaSession' in navigator)) D.windowAction('media-keys').catch(logFailure('media keys'));
    attachConfigured();
    sendNow(); sendAppearance(); Designs.ready(sendPackage);
    /* the page came back after its process crashed (app/src/desktop/recovery.rs): the queue is restored from this
       device, and the app says which song played, where, and whether it was playing */
    invoke('resume_after_crash').then(function (r) {
      var at = r ? P.state.queue.findIndex(function (q) { return q.trackId === r.id; }) : -1;
      if (at >= 0) P.jumpTo(at, { at: r.position, autoplay: r.playing });
    }).catch(logFailure('resume after a crash'));
  }
})();
