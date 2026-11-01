/* lull desktop wrapper UI (Tauri v2). Spec: design/desktop/DESKTOP.md; wiring: docs/context/desktop.md.
   Styles: desktop.css. Classic script; needs only ../icons.js. Loaded by the web UI (index.html) and by
   the wrapper's own page (wrapper.html). Outside the wrapper it does nothing. Exposes window.LullDesktop:
     inWrapper                                       true inside the desktop app
     attachTitlebar(os, layout) / detachTitlebar()   Windows caption buttons or GNOME header-bar buttons
     connectPage(opts) / offlinePage(opts)           bundled pages (first run, server unreachable)
     miniDevice() / miniStrip()                      mini-player markup (the miniPlayer design slot)
     askClose()                                      one-time "keep playing in the tray?" dialog
     windowAction(action, detail)                    window operations (DESKTOP.md 4, window_action)
     settings() / setSettings(patch)                 Settings > App inside the wrapper (Promises)
     openMini()                                      the player bar's Device view button
     mountPage()                                     wrapper.html: render the view named in the hash
   window.__LULL_DESKTOP__ ({ os, titlebar, layout }) is set by the wrapper before any page script runs. */
(function () {
  'use strict';
  var D = window.LullDesktop = {};
  var doc = document, root = doc.documentElement;
  var T = window.__TAURI__ || null;
  var W = window.__LULL_DESKTOP__ || null;      // only in the wrapper's main window
  D.inWrapper = !!T;

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
  D.openMini = function () { return D.windowAction('mini').catch(logFailure('mini player')); };

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
     2. Bundled pages: first-run connect, server unreachable
     ===================================================================== */
  function hostOf(url) { try { return new URL(url).host; } catch (e) { return url; } }
  /* same rule as the wrapper (server.rs origin): "music.home.arpa" means https, paths are dropped */
  function normalise(url) {
    url = String(url || '').trim();
    if (url && !/^[a-z]+:\/\//i.test(url)) url = 'https://' + url;
    try { var u = new URL(url); if (!/^https?:$/.test(u.protocol) || !u.hostname || /\s/.test(url)) return null; return u.origin; }
    catch (e) { return null; }
  }
  /* the design's brand slot (designs/designs.js) when the page loads it, else the plain wordmark */
  function wordmark() {
    var plain = function () {
      return '<span class="wordmark brand" aria-hidden="true"><span class="brand__name">lull</span><span class="brand__orn wordmark__spark"></span></span>';
    };
    var name = window.Designs ? Designs.product : 'lull';
    return '<span class="brand-mark" role="img" aria-label="' + name + '">' +
      (window.Designs ? String(Designs.slot('brand', { where: 'desktop' }, Designs.brandMarkup)) : plain()) + '</span>';
  }
  var SERVICE_NAMES = { soulseek: 'Soulseek', library: 'Library', lastfm: 'Last.fm' };
  function answered(health) {
    var names = Object.keys(SERVICE_NAMES).filter(function (k) {
      var s = health.services && health.services[k]; return s && (s.state === 'ok' || s.state === 'warn');
    }).map(function (k) { return SERVICE_NAMES[k]; });
    if (!names.length) return '';
    var list = names.length > 1 ? names.slice(0, -1).join(', ') + ' and ' + names[names.length - 1] : names[0];
    return ' ' + list + ' answered.';
  }
  function statusMarkup(state, url, health) {
    var host = esc(hostOf(url));
    switch (state) {
      case 'testing': return '<span class="spin-ring" aria-hidden="true"></span><span class="dk-status__text">Testing ' + host + '&hellip; <button type="button" class="btn btn--quiet" data-dk="cancel">Cancel</button></span>';
      case 'ok': return window.icon('status-ok', 'OK', 'sg sg--ok') + '<span class="dk-status__text">Connected to lull ' + esc(health.version) + '.' + esc(answered(health)) + '</span>';
      case 'unreachable': return window.icon('status-down', 'Error', 'sg sg--down') + '<span class="dk-status__text">Can&rsquo;t reach ' + host + '. Check the address and that this computer is on your home network or VPN.</span>';
      case 'rejected': return window.icon('status-down', 'Error', 'sg sg--down') + '<span class="dk-status__text">The server answered but didn&rsquo;t accept this token.</span>';
      case 'invalid': return ic('warning') + '<span class="dk-status__text">That doesn&rsquo;t look like an address yet.</span>';
      default: return ic('info') + '<span class="dk-status__text">lull checks the address and token before saving.</span>';
    }
  }
  function fieldError(id, key) {
    return '<span class="field__error" id="' + id + '" data-err="' + key + '" hidden>' + ic('warning', 'ic-16') + '<span></span></span>';
  }
  D.connectPage = function (o) {
    o = o || {};
    var id = 'dk';
    return '<div class="dk-page"><div class="dk-page__inner">' + wordmark() +
      '<section class="card dk-card" aria-labelledby="' + id + '-title" data-dk-connect data-state="idle">' +
      '<h1 class="dk-card__title" id="' + id + '-title">connect to your server</h1>' +
      '<p class="dk-card__lede">lull plays from the server you run at home. You only do this once.</p>' +
      '<form class="dk-form" novalidate>' +
      '<div class="field"><label class="field__label" for="' + id + '-url">Server address</label>' +
      '<input class="input" id="' + id + '-url" name="url" type="url" inputmode="url" autocomplete="url" spellcheck="false" placeholder="https://music.home.arpa" value="' + esc(o.url || '') + '" aria-describedby="' + id + '-url-help ' + id + '-url-err">' +
      '<span class="field__help" id="' + id + '-url-help" data-help="url">The address you open lull at in a browser.</span>' + fieldError(id + '-url-err', 'url') + '</div>' +
      '<div class="field"><label class="field__label" for="' + id + '-token">Access token</label>' +
      '<div class="dk-field-row"><input class="input" id="' + id + '-token" name="token" type="password" autocomplete="off" spellcheck="false" aria-describedby="' + id + '-token-help ' + id + '-token-err">' +
      '<button type="button" class="btn btn--quiet btn--sm dk-reveal" data-dk="reveal" aria-pressed="false" aria-controls="' + id + '-token">Show</button></div>' +
      '<span class="field__help" id="' + id + '-token-help" data-help="token">Only if your server sets one. It&rsquo;s in the server&rsquo;s config.</span>' + fieldError(id + '-token-err', 'token') + '</div>' +
      '<label class="switch-row"><span><span class="switch-row__label">Remember on this device</span><span class="switch-row__help">Kept by lull on this computer. Off: asked again next time.</span></span>' +
      '<button type="button" class="switch" role="switch" aria-checked="true" data-dk="switch" name="remember"></button></label>' +
      '<div class="dk-status" role="status" data-state="idle">' + statusMarkup('idle', '') + '</div>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="test">Test connection</button>' +
      '<button type="submit" class="btn btn--primary">Connect</button></div>' +
      '</form></section></div></div>';
  };
  function setConnectState(card, state, url, health) {
    card.setAttribute('data-state', state);
    var st = $('.dk-status', card); st.setAttribute('data-state', state); st.innerHTML = statusMarkup(state, url, health);
    var urlIn = $('[name="url"]', card), tokIn = $('[name="token"]', card);
    var urlErr = state === 'unreachable' ? 'No answer from this address' : state === 'invalid' ? 'Enter a full address, like https://music.home.arpa' : '';
    var tokErr = state === 'rejected' ? 'Token rejected' : '';
    [[urlIn, 'url', urlErr], [tokIn, 'token', tokErr]].forEach(function (f) {   // the error takes the help line's place: no shift
      var err = $('[data-err="' + f[1] + '"]', card), help = $('[data-help="' + f[1] + '"]', card);
      if (f[2]) { f[0].setAttribute('aria-invalid', 'true'); err.lastChild.textContent = f[2]; }
      else f[0].removeAttribute('aria-invalid');
      err.hidden = !f[2]; help.hidden = !!f[2];
    });
  }
  /* GET <url>/health, then /api/ping with the token, done by the wrapper (the page's origin can't read them) */
  function runTest(card, thenConnect) {
    var urlIn = $('[name="url"]', card), tokIn = $('[name="token"]', card);
    var url = normalise(urlIn.value), token = tokIn.value.trim();
    var run = card._dkRun = (card._dkRun || 0) + 1;        // Cancel and newer tests make older answers stale
    if (!url) { setConnectState(card, 'invalid', urlIn.value); urlIn.focus(); return; }
    urlIn.value = url;
    setConnectState(card, 'testing', url);
    invoke('test_server', { url: url, token: token }).then(function (r) {
      if (run !== card._dkRun) return;
      setConnectState(card, r.state, url, r.health);
      if (r.state === 'unreachable') urlIn.focus();
      else if (r.state === 'rejected') tokIn.focus();
      else if (thenConnect) connect(card, url, token);
    }, function () {
      if (run === card._dkRun) { setConnectState(card, 'invalid', url); urlIn.focus(); }
    });
  }
  function connect(card, url, token) {
    var remember = $('[name="remember"]', card).getAttribute('aria-checked') === 'true';
    D.windowAction('connected', { url: url, token: token, remember: remember }).catch(function (err) {
      setConnectState(card, 'unreachable', url);
      $('.dk-status__text', card).textContent = String(err);
    });
  }

  var STAR_ART = '<svg viewBox="0 0 112 88"><path d="M56 10l9 20 22 3-16 15 4 22-19-10-19 10 4-22-16-15 22-3z"/><path d="M56 30l-4 12 6 4-5 12"/></svg>';
  var BACKOFF = [2, 4, 8, 16, 30];
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
      '<h1 class="empty__title" id="dk-o-title">Can&rsquo;t reach your server</h1>' +
      '<p class="dk-offline__text">' + esc(hostOf(o.url || '')) + ' didn&rsquo;t answer. Check that the server is running and that this computer is on your home network or VPN.</p>' +
      '<div class="dk-retry"><span class="dk-retry__label" aria-hidden="true"></span>' +
      '<div class="progress" aria-hidden="true"><i style="--p:0"></i></div><span class="vh" role="status" data-dk-retry-status></span></div>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="change-server">Change server</button>' +
      '<button type="button" class="btn btn--primary" data-dk="retry">Retry now</button></div>' +
      '<details class="dk-details"><summary>' + ic('chevron-down') + 'Details</summary><dl class="dk-kv">' +
      '<dt>Address</dt><dd>' + esc(o.url || '') + '</dd><dt>Error</dt><dd data-dk-error>' + esc(o.error || '') + '</dd>' +
      '<dt>Last connected</dt><dd>' + esc(when(+o.last)) + '</dd><dt>Attempt</dt><dd class="px" data-dk-attempt>1</dd></dl></details>' +
      '</section></div></div>';
  };
  function startRetry(card) {
    var label = $('.dk-retry__label', card), bar = $('.progress > i', card), status = $('[data-dk-retry-status]', card);
    var attempt = +card.getAttribute('data-attempt'), left, total, busy = false;
    function arm() { total = left = BACKOFF[Math.min(attempt - 1, BACKOFF.length - 1)]; tick(); }
    function tick() {
      label.textContent = 'retrying in ' + left + 's';
      bar.style.setProperty('--p', String(1 - left / total));
    }
    function attemptNow() {
      if (busy) return;
      busy = true; clearInterval(card._dkTimer);
      label.textContent = 'trying now';
      bar.style.setProperty('--p', '1');
      // on success the wrapper loads the web UI in this window, and this page goes away
      D.windowAction('retry').catch(function (err) {
        busy = false;
        attempt++; card.setAttribute('data-attempt', attempt);
        $('[data-dk-attempt]', card).textContent = attempt;
        $('[data-dk-error]', card).textContent = String(err);
        status.textContent = 'Still can\'t reach the server. Attempt ' + attempt + '.';
        arm(); card._dkTimer = setInterval(step, 1000);
      });
    }
    function step() { if (doc.hidden) return; left--; if (left <= 0) attemptNow(); else tick(); }
    card._dkRetry = attemptNow;
    arm(); card._dkTimer = setInterval(step, 1000);
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
    var prev = S; S = next;
    var key = function (s) { return JSON.stringify([s.track, s.art, s.received < 1]); };
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
  var MINI_ACTIONS = ['play-pause', 'previous', 'next', 'like', 'shuffle', 'repeat', 'vol-up', 'vol-down'];
  function seekTo(fraction) { toMain('seek', { fraction: Math.min(1, Math.max(0, fraction)) }); }

  doc.addEventListener('click', function (e) {
    var el = e.target.closest('[data-dk]'); if (!el) return;
    var act = el.getAttribute('data-dk'), scope = el.closest('[data-dk-player]');
    if (scope && MINI_ACTIONS.indexOf(act) >= 0) return toMain(act);
    var card = el.closest('[data-dk-connect]');
    switch (act) {
      case 'minimize': case 'maximize': case 'close': case 'full': case 'mini-close': case 'change-server':
        return D.windowAction(act).catch(logFailure(act));
      case 'switch': el.setAttribute('aria-checked', String(el.getAttribute('aria-checked') !== 'true')); return;
      case 'reveal':
        var input = $('#' + el.getAttribute('aria-controls')), show = input.type === 'password';
        input.type = show ? 'text' : 'password'; el.textContent = show ? 'Hide' : 'Show'; el.setAttribute('aria-pressed', String(show)); return;
      case 'test': return runTest(card, false);
      case 'cancel': card._dkRun++; setConnectState(card, 'idle', $('[name="url"]', card).value); return;
      case 'retry': var off = el.closest('[data-dk-offline]'); if (off && off._dkRetry) off._dkRetry(); return;
      case 'seek': return seekTo((e.clientX - el.getBoundingClientRect().left) / el.offsetWidth);
    }
  });
  doc.addEventListener('submit', function (e) {
    var card = e.target.closest('[data-dk-connect]'); if (!card) return;
    e.preventDefault();
    var url = normalise($('[name="url"]', card).value);
    if (card.getAttribute('data-state') === 'ok' && url) connect(card, url, $('[name="token"]', card).value.trim());
    else runTest(card, true);
  });
  doc.addEventListener('keydown', function (e) {
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w || !S.duration) return;
    var step = e.key === 'ArrowRight' ? 5 : e.key === 'ArrowLeft' ? -5 : e.key === 'Home' ? -1e9 : e.key === 'End' ? 1e9 : 0;
    if (!step) return;
    e.preventDefault(); seekTo((S.pos + step) / S.duration);
  });
  /* any typing in the connect form makes an earlier "ok" stale */
  doc.addEventListener('input', function (e) {
    var card = e.target.closest && e.target.closest('[data-dk-connect]');
    if (card && card.getAttribute('data-state') !== 'idle') { card._dkRun = (card._dkRun || 0) + 1; setConnectState(card, 'idle', ''); }
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
    if (view === 'connect' || view === 'offline') {
      $$('[data-dk-offline]').forEach(function (c) { clearInterval(c._dkTimer); });   // leaving the unreachable page
      doc.title = view === 'connect' ? 'Connect to your server' : 'Can’t reach your server';
      host.innerHTML = view === 'connect' ? D.connectPage({ url: q.get('url') }) : D.offlinePage({ url: q.get('url'), error: q.get('error'), last: q.get('last') });
      attachConfigured();
      $$('[data-dk-offline]').forEach(startRetry);
      var first = $('[data-dk-connect] [name="url"]'); if (first) first.focus();
    } else if (view === 'mini' || view === 'strip') {
      doc.title = 'lull mini player';
      miniOpts = { opaque: !!q.get('opaque') };
      var render = function () {
        host.innerHTML = view === 'strip' ? D.miniStrip() : D.miniDevice();
        receive(doc, S, true);
      };
      Designs.ready(render);
      T.event.listen('player', function (e) { receive(doc, e.payload, false); });
      T.event.listen('appearance', function (e) {        // DESKTOP.md 6.1: follow the web UI live
        Designs.applyTheme(e.payload.theme);
        if (e.payload.design !== Designs.current) Designs.set(e.payload.design, render);
      });
      toMain('hello');                                     // main answers with a snapshot
      if (q.get('wayland')) setTimeout(function () {
        note(doc, 'Your desktop decides what stays on top: use the window menu (Alt+Space) > Always on Top.', 8000);
      }, 400);
    }
  };

  /* the wrapper moves between connect and unreachable by changing the hash: same document, so render again */
  if ($('#dk-root')) window.addEventListener('hashchange', D.mountPage);

  /* =====================================================================
     8. Bridge in the web UI (wrapper only; DESKTOP.md 4 and 6.1). Tray items and mini-player buttons run
        the web UI's own actions; set_now_playing keeps the tray current; 'player' snapshots feed the mini
        player; 'appearance' tells the wrapper which design and theme its own pages should use.
     ===================================================================== */
  if (window.App && App.player && T) {
    var P = App.player;
    var run = function (a, detail) {
      var id = P.currentId(), ps = P.state;
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
        case 'hello': return sendPlayer(true);
        default: console.warn('lull desktop: unknown action', a);
      }
    };
    T.event.listen('tray', function (e) { run(e.payload.action, e.payload); });
    T.event.listen('mini', function (e) { run(e.payload.action, e.payload); });
    T.event.listen('ask-close', function () { D.askClose(); });

    var lastNow = '';
    var sendNow = function () {
      var t = P.current(), id = t ? t.id : null;
      var now = { title: t ? t.title : '', artist: t ? t.artist : '', playing: !!P.state.playing,
        liked: !!(id && App.isLiked(id)), saved: !!(id && App.isSaved(id)),
        connected: !App.banner.list().some(function (b) { return b.id === 'sys-unreachable'; }) };
      var key = JSON.stringify(now); if (key === lastNow) return; lastNow = key;
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
        reader.onload = function () { if (art.src === src) { art.data = reader.result; sendPlayer(true); } };
        reader.readAsDataURL(blob);
      }).catch(logFailure('cover art'));
      return null;
    };
    var snapshot = function () {
      var t = P.current(), ps = P.state, id = t ? t.id : null;
      return { track: t ? { title: t.title, artist: t.artist, format: t.format || '', lossless: !!t.lossless } : null,
        art: t ? artFor() : null, playing: !!ps.playing, liked: !!(id && App.isLiked(id)),
        pos: ps.pos || 0, duration: P.duration() || 0, durationKnown: ps.durationKnown !== false,
        buffered: ps.buffered || [], received: ps.received == null ? 1 : ps.received,
        shuffle: !!ps.shuffle, repeat: ps.repeat || 'off', volume: ps.muted ? 0 : ps.volume };
    };
    var lastPlayer = '';
    var sendPlayer = function (force) {
      var snap = snapshot(), key = JSON.stringify(snap);
      if (!force && key === lastPlayer) return; lastPlayer = key;
      T.event.emit('player', snap).catch(logFailure('player snapshot'));
    };
    var sendAppearance = function () {
      T.event.emit('appearance', { design: App.design(), theme: App.prefs().theme }).catch(logFailure('appearance'));
    };

    ['player', 'like', 'save', 'ready'].forEach(function (ev) { App.on(ev, sendNow); });
    ['player', 'tick', 'like', 'save'].forEach(function (ev) { App.on(ev, function () { sendPlayer(false); }); });
    ['design', 'prefs'].forEach(function (ev) { App.on(ev, sendAppearance); });
    App.on('ready', reattach); App.on('design', reattach); App.on('route', markDragRegions);
    doc.addEventListener('keydown', function (e) {        // Ctrl+Shift+M: mini player
      if (e.ctrlKey && e.shiftKey && !e.altKey && (e.key === 'M' || e.key === 'm')) { e.preventDefault(); D.openMini(); }
    });
    if (!('mediaSession' in navigator)) D.windowAction('media-keys').catch(logFailure('media keys'));
    attachConfigured();
    sendNow(); sendAppearance();
  }
})();
