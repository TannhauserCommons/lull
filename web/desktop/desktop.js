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
     mountPage()                                     wrapper.html: render the view named in the hash
   window.__LULL_DESKTOP__ ({ os, titlebar, layout }) is set by the wrapper before any page script runs. */
(function () {
  'use strict';
  var D = window.LullDesktop = {};
  var doc = document, root = doc.documentElement;
  var T = window.__TAURI__ || null;
  var W = window.__LULL_DESKTOP__ || null;      // only in the wrapper's main window
  // The Android app also has Tauri, but none of the desktop's windows; it brings window.LullNative instead.
  D.inWrapper = !!T && !window.LullNative;

  function $(sel, el) { return (el || doc).querySelector(sel); }
  function $$(sel, el) { return Array.prototype.slice.call((el || doc).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; }); }
  function ic(name, cls) { return window.icon(name, null, cls); }
  function fmt(sec) { sec = Math.max(0, Math.round(sec)); return Math.floor(sec / 60) + ':' + ('0' + sec % 60).slice(-2); }
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
  D.setSettings = function (patch) { return invoke('set_desktop_settings', { patch: patch }); };
  /* a see-through window (app/src/desktop/glass.rs): what `main` shows behind the page, and why not when its theme
     asked for glass and got none (held: off, system, unsupported, restart) */
  var glassFns = [];
  D.glass = { state: null, on: function (fn) { glassFns.push(fn); } };
  function showGlass(grant) {
    D.glass.state = grant;
    Designs.setWindow(grant && grant.material);
    glassFns.forEach(function (fn) { fn(grant); });
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
     4. Mini player: mirrors the main window's player. `main` sends a snapshot (section 8, snapshot())
        as the 'player' event; the buttons go back to it as 'mini' events and it runs them as its own actions.
     ===================================================================== */
  var S = { track: null, art: null, playing: false, liked: false, pos: 0, duration: 0, durationKnown: true,
    buffered: [], received: 1, shuffle: false, repeat: 'off', volume: 1 };
  var FALLBACK_ART = '../assets/fallback-art-256.png';
  var BARS = 40;
  function barHeight(i) { return (0.32 + 0.6 * Math.abs(Math.sin(i * 1.7) * Math.cos(i * .43))).toFixed(2); }
  function chip(t) { return t.format ? '<span class="q-chip ' + (t.lossless ? 'q-chip--ll' : 'q-chip--ly') + ' q-chip--solid" title="' + (t.lossless ? 'Lossless, ' : '') + esc(t.format) + '">' + esc(t.format) + '</span>' : ''; }
  function cover(size) {
    return '<span class="cover dk-art' + (size <= 64 ? ' cover--thumb' : '') + '" style="--cover-size-base:' + size + 'px" aria-hidden="true"><img alt="" src="' + esc(S.art || FALLBACK_ART) + '"></span>';
  }
  function frac() { return S.duration ? Math.min(1, S.pos / S.duration) : 0; }
  function bufferedAt(x) { return S.buffered.some(function (r) { return x >= r[0] && x <= r[1]; }); }
  function bufferEnd() {           // end of the buffered range the playhead is in
    var f = frac(), r = S.buffered.filter(function (r) { return f >= r[0] && f <= r[1]; })[0];
    return r ? r[1] : 0;
  }
  function waveText() { return fmt(S.pos) + ' of ' + (S.durationKnown ? '' : 'about ') + fmt(S.duration) + ', downloaded to ' + fmt(bufferEnd() * S.duration); }
  function waveMarkup() {
    var b = ''; for (var i = 0; i < BARS; i++) b += '<i class="wave__bar" style="--h:' + barHeight(i) + '"></i>';
    return '<div class="wave" role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax="100" data-dk="seek">' + b +
      '<div class="wave__rail"><div class="wave__head"></div></div></div>';
  }
  function nowMarkup() {
    var t = S.track;
    if (!t) return '<div class="dk-mini__now">' + cover(84) + '<div><div class="device__title">Nothing playing</div><div class="device__artist">Pick something in lull</div></div></div>';
    return '<div class="dk-mini__now">' + cover(84) + '<div><div class="device__title">' + esc(t.title) + '</div>' +
      '<div class="device__artist">' + esc(t.artist) + '</div><div class="device__facts">' + chip(t) +
      (S.received < 1 ? '<span class="device__fact">peer ' + Math.round(S.received * 100) + '%</span>' : '') + '</div></div></div>';
  }
  function playIcon() { return S.playing ? ic('pause') : ic('play', 'ic-play'); }
  function likeIcon() { return ic(S.liked ? 'heart-fill' : 'heart'); }
  function volText() { return 'vol ' + Math.round(S.volume * 100) + '%'; }

  /* miniPlayer slot: a design.js may replace the window content. ctx = { style: 'device'|'strip', opaque,
     track { title, artist, format, lossless } | null, playing, liked, parts { device(), strip(), cover(size), wave() } }.
     Keep data-dk-player and the data-dk="..." buttons / data-dk-* fields in an override: section 5 drives those. */
  var miniOpts = {};
  function miniPlayer(style) {
    var ctx = { style: style, opaque: !!miniOpts.opaque, playing: S.playing, liked: S.liked, track: S.track,
      parts: { device: deviceMarkup, strip: stripMarkup, cover: cover, wave: waveMarkup } };
    var dflt = function (c) { return c.style === 'strip' ? stripMarkup() : deviceMarkup(); };
    return window.Designs ? String(Designs.slot('miniPlayer', ctx, dflt)) : dflt(ctx);
  }
  D.miniDevice = function () { return miniPlayer('device'); };
  D.miniStrip = function () { return miniPlayer('strip'); };
  function deviceMarkup() {
    var grill = ''; for (var g = 0; g < 9; g++) grill += '<i></i>';
    return '<div class="dk-mini' + (miniOpts.opaque ? ' dk-mini--opaque' : '') + '" data-dk-player="device">' +
      '<section class="device" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="dk-mini__head" data-tauri-drag-region><span class="dk-mini__led" aria-hidden="true"><i></i>on</span><span class="spacer" data-tauri-drag-region></span>' +
      '<button class="dk-mini__win" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="dk-mini__win" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button></div>' +
      '<div class="device__bezel" data-tauri-drag-region><div class="device__screen s-screen">' +
      '<div class="device__page"><div class="device__page-head"><span>now playing</span><span data-dk-vol>' + volText() + '</span></div>' +
      '<div data-dk-now aria-live="polite">' + nowMarkup() + '</div>' + waveMarkup() +
      '<div class="wave-times"><b data-dk-pos></b><span data-dk-dur></span></div></div>' +
      '<div class="dk-mini__note" aria-hidden="true"></div></div>' +
      '<div class="device__brand">' + wordmark() + '</div></div>' +
      '<div class="device__controls"><div class="dpad">' +
      '<button class="dpad__btn dpad__btn--up" data-dk="vol-up" aria-label="Volume up" data-tip="Volume up">' + ic('plus') + '</button>' +
      '<button class="dpad__btn dpad__btn--left" data-dk="previous" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<span class="dpad__hub"></span>' +
      '<button class="dpad__btn dpad__btn--right" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="dpad__btn dpad__btn--down" data-dk="vol-down" aria-label="Volume down" data-tip="Volume down">' + ic('minus') + '</button></div>' +
      '<div class="device__ab"><span class="device__key device__key--b"><button class="btn-gloss btn-gloss--48" data-dk="like" aria-label="Like" data-tip="Like">' + likeIcon() + '</button>like</span>' +
      '<span class="device__key device__key--a"><button class="btn-gloss" data-dk="play-pause" aria-label="Play" data-tip="Play">' + playIcon() + '</button><span data-dk-play-label></span></span></div></div>' +
      '<div class="device__pills"><button class="device__pill device__pill--shuffle" data-dk="shuffle" aria-pressed="false"><i></i>shuffle</button>' +
      '<button class="device__pill device__pill--repeat" data-dk="repeat" aria-pressed="false"><i></i>repeat</button></div>' +
      '<div class="device__grill" aria-hidden="true">' + grill + '</div></section></div>';
  }
  function stripMarkup() {
    var t = S.track;
    return '<div class="dk-strip-wrap" data-dk-player="strip"><section class="dk-strip s-panel" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="seek seek--line" aria-hidden="true"><div class="seek__track"><div class="seek__xfer"></div><div class="seek__bufs"><div class="seek__buf" data-dk-buf></div></div><div class="seek__played"></div></div></div>' +
      '<button class="bezel-thumb" data-dk="full" aria-label="Open full window" data-tip="Open full window" data-dk-thumb>' + cover(40) + '</button>' +
      '<div class="dk-strip__text" data-tauri-drag-region aria-live="polite"><div class="dk-strip__title" data-dk-title>' + esc(t ? t.title : 'Nothing playing') + '</div><div class="dk-strip__artist" data-dk-artist>' + esc(t ? t.artist : '') + '</div></div>' +
      '<div class="dk-strip__keys">' +
      '<button class="btn-icon btn-icon--bare" data-dk="previous" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<button class="btn-gloss btn-gloss--40" data-dk="play-pause" aria-label="Play" data-tip="Play">' + playIcon() + '</button>' +
      '<button class="btn-icon btn-icon--bare" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="like" aria-label="Like" data-tip="Like">' + likeIcon() + '</button>' +
      '<span class="dk-sep" aria-hidden="true"></span>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button>' +
      '</div></section></div>';
  }

  function paintPlayer(scope) {
    var f = frac();
    $$('[data-dk-player]', scope).forEach(function (el) {
      $$('.wave', el).forEach(function (w) {
        w.style.setProperty('--pos', f.toFixed(4));
        w.classList.toggle('is-playing', S.playing);
        $$('.wave__bar', w).forEach(function (b, i) {
          var x = (i + .5) / BARS;
          b.className = 'wave__bar' + (x <= f ? ' is-played' : bufferedAt(x) ? ' is-buf' : x <= S.received ? ' is-xfer' : '');
        });
        w.setAttribute('aria-valuenow', Math.round(f * 100)); w.setAttribute('aria-valuetext', waveText());
      });
      $$('[data-dk-pos]', el).forEach(function (n) { n.textContent = fmt(S.pos); });
      $$('[data-dk-dur]', el).forEach(function (n) { n.textContent = (S.durationKnown ? '' : '~') + fmt(S.duration); });
      $$('[data-dk-vol]', el).forEach(function (n) { n.textContent = volText(); });
      $$('.seek--line', el).forEach(function (s) {
        s.style.setProperty('--pos', f.toFixed(4)); s.style.setProperty('--xfer', S.received);
        var buf = $('[data-dk-buf]', s); buf.style.left = '0'; buf.style.width = (bufferEnd() * 100) + '%';
      });
      $$('[data-dk="play-pause"]', el).forEach(function (b) {
        var label = S.playing ? 'Pause' : 'Play';
        b.innerHTML = playIcon(); b.setAttribute('aria-label', label); b.setAttribute('data-tip', label);
      });
      $$('[data-dk-play-label]', el).forEach(function (n) { n.textContent = S.playing ? 'pause' : 'play'; });
      $$('[data-dk="like"]', el).forEach(function (b) {
        b.innerHTML = likeIcon(); b.setAttribute('aria-pressed', String(S.liked)); b.classList.toggle('is-liked', S.liked);
        b.setAttribute('data-tip', S.liked ? 'Liked' : 'Like');
      });
      $$('[data-dk="shuffle"]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(S.shuffle)); });
      $$('[data-dk="repeat"]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(S.repeat !== 'off')); });
    });
  }
  function paintTrack(scope) {
    var t = S.track;
    $$('[data-dk-now]', scope).forEach(function (n) { n.innerHTML = nowMarkup(); });
    $$('[data-dk-thumb]', scope).forEach(function (n) { n.innerHTML = cover(40); });
    $$('[data-dk-title]', scope).forEach(function (n) { n.textContent = t ? t.title : 'Nothing playing'; });
    $$('[data-dk-artist]', scope).forEach(function (n) { n.textContent = t ? t.artist : ''; });
  }
  function note(scope, text, ms) {
    $$('.dk-mini__note', scope).forEach(function (n) {
      n.textContent = text; n.classList.add('is-on');
      clearTimeout(n._t); n._t = setTimeout(function () { n.classList.remove('is-on'); }, ms || 1600);
    });
    announce(text);
  }
  /* a new snapshot from main: repaint, and say what the last button press changed */
  var REPEAT_TEXT = { off: 'repeat off', all: 'repeat all', one: 'repeat one' };
  function receive(scope, next, first) {
    var prev = S; S = Object.assign({}, next, { art: prev.art });   // the cover comes on its own ('player-art')
    var key = function (s) { return JSON.stringify([s.track, s.received < 1]); };
    if (first || key(prev) !== key(next)) paintTrack(scope);
    paintPlayer(scope);
    if (first) return;
    if (prev.volume !== next.volume) note(scope, 'volume ' + Math.round(next.volume * 100) + '%');
    if (prev.shuffle !== next.shuffle) note(scope, next.shuffle ? 'shuffle on' : 'shuffle off');
    if (prev.repeat !== next.repeat) note(scope, REPEAT_TEXT[next.repeat] || 'repeat');
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
  var MINI_KEYS = {"full":"window","like":"main","mini-close":"window","next":"main","play-pause":"main","previous":"main","repeat":"main","seek":"seek","shuffle":"main","vol-down":"main","vol-up":"main"};
  function seekTo(fraction) { toMain('seek', { fraction: Math.min(1, Math.max(0, fraction)) }); }

  doc.addEventListener('click', function (e) {
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
     at once, main seeks once a frame), not only where the button comes up. Main's next snapshot has the truth (a
     Soulseek song can't seek past what has arrived) */
  doc.addEventListener('pointerdown', function (e) {
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w || e.button !== 0 || !S.duration) return;
    e.preventDefault(); w.focus({ preventScroll: true });
    try { w.setPointerCapture(e.pointerId); } catch (err) { /* synthetic events */ }
    var f = 0, frame = 0;
    function at(ev) { var r = w.getBoundingClientRect(); return Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width)); }
    function show() { frame = 0; S.pos = f * S.duration; paintPlayer(doc); seekTo(f); }
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
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w || !S.duration) return;
    var step = e.key === 'ArrowRight' ? 5 : e.key === 'ArrowLeft' ? -5 : e.key === 'Home' ? -1e9 : e.key === 'End' ? 1e9 : 0;
    if (!step) return;
    e.preventDefault(); seekTo((S.pos + step) / S.duration);
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
      var render = function () {
        host.innerHTML = view === 'strip' ? D.miniStrip() : D.miniDevice();
        receive(doc, S, true);
        fitWindow($('[data-dk-player]', host));
      };
      Designs.ready(render);
      T.event.listen('player', function (e) { receive(doc, e.payload, false); });
      T.event.listen('player-art', function (e) { if (S.art !== e.payload.art) { S = Object.assign({}, S, { art: e.payload.art }); paintTrack(doc); } });
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
      toMain('hello');                                     // main answers with a snapshot
      window.addEventListener('pagehide', function () { toMain('bye'); });   // main stops sending snapshots
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
    fitting = new ResizeObserver(function () {
      var h = Math.ceil(el.getBoundingClientRect().height);
      if (h && h !== innerHeight) w.setSize(new LogicalSize(innerWidth, h)).catch(logFailure('mini player size'));
    });
    fitting.observe(el);
  }

  /* the app moves between its pages by changing the hash: same document, so render again */
  if ($('#dk-root')) window.addEventListener('hashchange', D.mountPage);

  /* =====================================================================
     8. Bridge in the web UI (wrapper only; DESKTOP.md 4 and 6.1). Tray items and mini-player buttons run
        the web UI's own actions; set_now_playing keeps the tray, the taskbar buttons and Discord current;
        'player' snapshots feed the mini player; 'appearance' tells the wrapper which design and theme its own
        pages should use. While Classic mode's window plays, what it reports is what's playing.
     ===================================================================== */
  if (window.App && App.player && D.inWrapper) {
    var P = App.player;
    var classicNow = null;                                 // { id, playing, pos, duration } from Webamp's window
    var playingId = function () { return classicNow ? classicNow.id : P.currentId(); };
    var run = function (a, detail) {
      var id = playingId(), ps = P.state;
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
        case 'hello': miniOpen = true; return sendPlayer(true);
        case 'bye': miniOpen = false; return;            // also sent by the app when it destroys the mini player
        default: console.warn('lull desktop: unknown action', a);
      }
    };
    T.event.listen('tray', function (e) { run(e.payload.action, e.payload); });
    T.event.listen('mini', function (e) { run(e.payload.action, e.payload); });
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
    var snapshot = function () {
      var t = P.current(), ps = P.state, id = t ? t.id : null;
      return { track: t ? { title: t.title, artist: t.artist, format: t.format || '', lossless: !!t.lossless } : null,
        playing: !!ps.playing, liked: !!(id && App.isLiked(id)),
        pos: Math.floor(ps.pos || 0), duration: P.duration() || 0, durationKnown: ps.durationKnown !== false,
        buffered: ps.buffered || [], received: ps.received == null ? 1 : Math.floor(ps.received * 100) / 100,
        shuffle: !!ps.shuffle, repeat: ps.repeat || 'off', volume: ps.muted ? 0 : ps.volume };
    };
    /* the cover goes on its own, once per song: inside the snapshot, a 100 KB data URL crossed to the mini player with
       every position update (four a second), and the page's memory grew by tens of MB a minute */
    var sentArt;
    var sendArt = function (force) {
      if (!miniOpen) return;
      var data = P.current() ? artFor() : null;
      if (data === null && art.src && P.current()) return;   // still fetching this song's: keep showing the last one
      if (!force && data === sentArt) return; sentArt = data;
      T.event.emit('player-art', { art: data }).catch(logFailure('cover art'));
    };
    /* snapshots feed only the mini player: none while it's closed ('hello' when it opens, 'bye' when it goes). pos in
       whole seconds and received in whole percent, as the mini player shows them, so a tick that moved nothing it draws
       sends nothing */
    var lastPlayer = '', miniOpen = false;
    var sendPlayer = function (force) {
      if (!miniOpen) return;
      sendArt(force);
      var snap = snapshot(), key = JSON.stringify(snap);
      if (!force && key === lastPlayer) return; lastPlayer = key;
      T.event.emit('player', snap).catch(logFailure('player snapshot'));
    };
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
    ['player', 'tick', 'like', 'save'].forEach(function (ev) { App.on(ev, function () { sendPlayer(false); }); });
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
