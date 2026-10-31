/* lull desktop wrapper UI (Tauri v2). Spec: DESKTOP.md. Styles: desktop.css.
   Classic script; needs only ../icons.js. Exposes window.LullDesktop:
     attachTitlebar(os, layout) / detachTitlebar()   Windows caption buttons or GNOME header-bar buttons
     connectPage(opts) / offlinePage(opts)            local pages the wrapper bundles (first run, unreachable)
     miniDevice(opts) / miniStrip()                   mini-player windows
     requestClose()                                   close button: one-time "keep playing in the tray?" dialog
     bind(root)                                       start timers/state for markup inserted above
   Inside Tauri (app.withGlobalTauri = true) window buttons call the real window API. Anywhere else
   (the design board, the prototype) they dispatch a 'dk:window' event { action } for a mock to show. */
(function () {
  'use strict';
  var D = window.LullDesktop = {};
  var doc = document, root = doc.documentElement;

  function $(sel, el) { return (el || doc).querySelector(sel); }
  function $$(sel, el) { return Array.prototype.slice.call((el || doc).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; }); }
  function ic(name, cls) { return window.icon(name, null, cls); }
  function store(key, val) {           // per-device convenience only; the real wrapper keeps prefs in Rust
    try {
      if (arguments.length > 1) localStorage.setItem('lull:desktop:' + key, JSON.stringify(val));
      else return JSON.parse(localStorage.getItem('lull:desktop:' + key));
    } catch (e) { return null; }
  }
  function fmt(sec) { sec = Math.max(0, Math.round(sec)); return Math.floor(sec / 60) + ':' + ('0' + sec % 60).slice(-2); }
  function announce(text) {
    var el = $('#dk-announcer');
    if (!el) { el = doc.createElement('div'); el.id = 'dk-announcer'; el.className = 'vh'; el.setAttribute('aria-live', 'polite'); doc.body.appendChild(el); }
    el.textContent = ''; setTimeout(function () { el.textContent = text; }, 30);
  }
  function reduced() { return root.getAttribute('data-motion') === 'reduce' || (root.getAttribute('data-motion') !== 'full' && matchMedia('(prefers-reduced-motion: reduce)').matches); }

  /* ---------- window API (real in Tauri, event elsewhere) ---------- */
  function tauri() { return window.__TAURI__ || null; }
  function currentWindow() { var T = tauri(); return T && T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null; }
  D.windowAction = function (action, detail) {
    var w = currentWindow(), T = tauri();
    if (w) {
      if (action === 'minimize') return w.minimize();
      if (action === 'maximize') return w.toggleMaximize();
      if (action === 'hide') return w.hide();                         // close to tray
      if (action === 'quit') return T.core.invoke('quit');
      return T.core.invoke('window_action', Object.assign({ action: action }, detail || {}));   // full, mini-close, connected, change-server
    }
    doc.dispatchEvent(new CustomEvent('dk:window', { detail: Object.assign({ action: action }, detail || {}) }));
  };

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
  D.attachTitlebar = function (os, layout) {
    if (root.getAttribute('data-titlebar') === os) return;
    D.detachTitlebar();
    root.setAttribute('data-titlebar', os);
    DRAG.forEach(function (sel) { $$(sel).forEach(function (el) { el.setAttribute('data-tauri-drag-region', ''); }); });
    var header = $('.header');
    var host = doc.createElement('div');
    host.className = 'dk-titlebar' + (header ? '' : ' dk-titlebar--bare');   // bare = a local page without the app header
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
  /* Windows dims caption glyphs in an inactive window; start active, follow real blur/focus */
  window.addEventListener('blur', function () { root.setAttribute('data-window-inactive', ''); });
  window.addEventListener('focus', function () { root.removeAttribute('data-window-inactive'); });
  (function watchMaximize() {            // keep the glyph honest when the user snaps or double-clicks the bar
    var w = currentWindow(); if (!w) return;
    w.onResized(function () { w.isMaximized().then(D.setMaximized); });
  })();

  /* =====================================================================
     2. Local pages: first-run connect, server unreachable
     ===================================================================== */
  // PROTO: demo values and the mock D.testServer below; the wrapper calls GET <url>/health with the token instead
  var DEMO = { url: 'https://music.home.arpa', token: 'sd_demo_7f3k9q2m4x8p' };
  var STATE_VALUES = {             // field values that make each state reproducible with Test
    idle: DEMO, testing: DEMO, ok: DEMO,
    unreachable: { url: 'https://music.home.lan', token: DEMO.token },
    rejected: { url: DEMO.url, token: 'sd_old' },
    invalid: { url: 'music home', token: DEMO.token }
  };
  function hostOf(url) { try { return new URL(url).host; } catch (e) { return url; } }
  function normalise(url) {
    url = String(url || '').trim();
    if (url && !/^[a-z]+:\/\//i.test(url)) url = 'https://' + url;     // "music.home.arpa" is fine
    try { var u = new URL(url); if (!/^https?:$/.test(u.protocol) || !u.hostname || /\s/.test(url)) return null; return u.origin; }
    catch (e) { return null; }
  }
  /* the design's brand slot (designs/designs.js) when the page loads it, else the plain wordmark; the wrapper names it */
  function wordmark() {
    var plain = function () {
      return '<span class="wordmark brand" aria-hidden="true"><span class="brand__name">lull</span><span class="brand__orn wordmark__spark"></span></span>';
    };
    var name = window.Designs ? Designs.product : 'lull';
    return '<span class="brand-mark" role="img" aria-label="' + name + '">' +
      (window.Designs ? String(Designs.slot('brand', { where: 'desktop' }, Designs.brandMarkup)) : plain()) + '</span>';
  }
  function statusMarkup(state, url) {
    var host = esc(hostOf(url));
    switch (state) {
      case 'testing': return '<span class="spin-ring" aria-hidden="true"></span><span class="dk-status__text">Testing ' + host + '&hellip; <button type="button" class="btn btn--quiet" data-dk="cancel">Cancel</button></span>';
      case 'ok': return window.icon('status-ok', 'OK', 'sg sg--ok') + '<span class="dk-status__text">Connected to lull 0.1. Soulseek, Jellyfin and Last.fm answered.</span>';
      case 'unreachable': return window.icon('status-down', 'Error', 'sg sg--down') + '<span class="dk-status__text">Can&rsquo;t reach ' + host + '. Check the address and that this computer is on your home network or VPN.</span>';
      case 'rejected': return window.icon('status-down', 'Error', 'sg sg--down') + '<span class="dk-status__text">The server answered but didn&rsquo;t accept this token.</span>';
      case 'invalid': return ic('warning') + '<span class="dk-status__text">That doesn&rsquo;t look like an address yet.</span>';
      default: return ic('info') + '<span class="dk-status__text">lull checks the address and token first.</span>';
    }
  }
  function fieldError(id, key, text) {
    return '<span class="field__error" id="' + id + '" data-err="' + key + '"' + (text ? '' : ' hidden') + '>' + ic('warning', 'ic-16') + '<span>' + (text || '') + '</span></span>';
  }
  D.connectPage = function (o) {
    o = o || {};
    var state = o.state || 'idle', v = STATE_VALUES[state] || DEMO, id = o.idPrefix || 'dk';
    var urlErr = state === 'unreachable' ? 'No answer from this address' : state === 'invalid' ? 'Enter a full address, like https://music.home.arpa' : '';
    var tokErr = state === 'rejected' ? 'Token rejected' : '';
    return '<div class="dk-page' + (o.inline ? ' dk-page--inline' : '') + '"><div class="dk-page__inner">' + wordmark() +
      '<section class="card dk-card" aria-labelledby="' + id + '-title" data-dk-connect data-state="' + state + '">' +
      '<h1 class="dk-card__title" id="' + id + '-title">connect to your server</h1>' +
      '<p class="dk-card__lede">lull plays from the server you run at home. You only do this once.</p>' +
      '<form class="dk-form" novalidate>' +
      '<div class="field"><label class="field__label" for="' + id + '-url">Server address</label>' +
      '<input class="input" id="' + id + '-url" name="url" type="url" inputmode="url" autocomplete="url" spellcheck="false" value="' + esc(v.url) + '" aria-describedby="' + id + '-url-help ' + id + '-url-err"' + (urlErr ? ' aria-invalid="true"' : '') + '>' +
      '<span class="field__help" id="' + id + '-url-help" data-help="url"' + (urlErr ? ' hidden' : '') + '>The address you open lull at in a browser.</span>' + fieldError(id + '-url-err', 'url', urlErr) + '</div>' +
      '<div class="field"><label class="field__label" for="' + id + '-token">Access token</label>' +
      '<div class="dk-field-row"><input class="input" id="' + id + '-token" name="token" type="password" autocomplete="off" spellcheck="false" value="' + esc(v.token) + '" aria-describedby="' + id + '-token-help ' + id + '-token-err"' + (tokErr ? ' aria-invalid="true"' : '') + '>' +
      '<button type="button" class="btn btn--quiet btn--sm dk-reveal" data-dk="reveal" aria-pressed="false" aria-controls="' + id + '-token">Show</button></div>' +
      '<span class="field__help" id="' + id + '-token-help" data-help="token"' + (tokErr ? ' hidden' : '') + '>Only if your server sets one. It&rsquo;s in the server&rsquo;s config.</span>' + fieldError(id + '-token-err', 'token', tokErr) + '</div>' +
      '<label class="switch-row"><span><span class="switch-row__label">Remember on this device</span><span class="switch-row__help">Kept in the system keychain.</span></span>' +
      '<button type="button" class="switch" role="switch" aria-checked="true" data-dk="switch"></button></label>' +
      '<div class="dk-status" role="status" data-state="' + state + '">' + statusMarkup(state, v.url) + '</div>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="test">Test connection</button>' +
      '<button type="submit" class="btn btn--primary">Connect</button></div>' +
      '</form></section></div></div>';
  };

  /* real wrapper: GET <url>/health (10 s timeout), then GET <url>/api/ping with the token. Mocked here. */
  D.testServer = function (url, token, done) {   // PROTO: mock test
    return setTimeout(function () {
      if (hostOf(url) !== 'music.home.arpa') done('unreachable');
      else if (!token || token.length < 12) done('rejected');
      else done('ok');
    }, 1400);
  };
  function setConnectState(card, state, url) {
    card.setAttribute('data-state', state);
    var st = $('.dk-status', card); st.setAttribute('data-state', state); st.innerHTML = statusMarkup(state, url);
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
  function runTest(card, thenConnect) {
    var urlIn = $('[name="url"]', card), tokIn = $('[name="token"]', card);
    clearTimeout(card._dkTimer);
    var url = normalise(urlIn.value);
    if (!url) { setConnectState(card, 'invalid', urlIn.value); urlIn.focus(); return; }
    urlIn.value = url;
    setConnectState(card, 'testing', url);
    card._dkTimer = D.testServer(url, tokIn.value.trim(), function (result) {
      setConnectState(card, result, url);
      if (result === 'unreachable') urlIn.focus();
      else if (result === 'rejected') tokIn.focus();
      else if (thenConnect) card._dkTimer = setTimeout(function () { D.windowAction('connected', { url: url }); }, 700);
    });
  }

  var STAR_ART = '<svg viewBox="0 0 112 88"><path d="M56 10l9 20 22 3-16 15 4 22-19-10-19 10 4-22-16-15 22-3z"/><path d="M56 30l-4 12 6 4-5 12"/></svg>';
  var BACKOFF = [2, 4, 8, 16, 30];
  D.offlinePage = function (o) {
    o = o || {};
    var url = o.url || DEMO.url;
    return '<div class="dk-page' + (o.inline ? ' dk-page--inline' : '') + '"><div class="dk-page__inner">' + wordmark() +
      '<section class="card dk-card dk-offline" aria-labelledby="dk-o-title" data-dk-offline data-attempt="3">' +
      '<div class="empty__art" aria-hidden="true">' + STAR_ART + '<span class="sparkle"></span></div>' +
      '<h1 class="empty__title" id="dk-o-title">Can&rsquo;t reach your server</h1>' +
      '<p class="dk-offline__text">' + esc(hostOf(url)) + ' didn&rsquo;t answer. Check that the server is running and that this computer is on your home network or VPN.</p>' +
      '<div class="dk-retry"><span class="dk-retry__label" aria-hidden="true">retrying in 8s</span>' +
      '<div class="progress" aria-hidden="true"><i style="--p:0"></i></div><span class="vh" role="status" data-dk-retry-status></span></div>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk="change-server">Change server</button>' +
      '<button type="button" class="btn btn--primary" data-dk="retry">Retry now</button></div>' +
      '<details class="dk-details"><summary>' + ic('chevron-down') + 'Details</summary><dl class="dk-kv">' +
      '<dt>Address</dt><dd>' + esc(url) + '</dd><dt>Error</dt><dd>Connection timed out after 10 s</dd>' +
      '<dt>Last connected</dt><dd>Oct 30, 21:14</dd><dt>Attempt</dt><dd class="px" data-dk-attempt>3</dd></dl></details>' +
      '</section></div></div>';
  };
  function startRetry(card) {
    var label = $('.dk-retry__label', card), bar = $('.progress > i', card), status = $('[data-dk-retry-status]', card);
    var attempt = +card.getAttribute('data-attempt'), left, total;
    function arm() { total = left = BACKOFF[Math.min(attempt - 1, BACKOFF.length - 1)]; tick(); }
    function tick() {
      label.textContent = 'retrying in ' + left + 's';
      bar.style.setProperty('--p', String(1 - left / total));
    }
    function attemptNow() {
      clearInterval(card._dkTimer);
      label.textContent = 'trying now';
      bar.style.setProperty('--p', '1');
      // real wrapper: GET <url>/health; on success load the web UI. The mock never succeeds.
      setTimeout(function () {
        attempt++; card.setAttribute('data-attempt', attempt);
        $('[data-dk-attempt]', card).textContent = attempt;
        status.textContent = 'Still can\'t reach the server. Attempt ' + attempt + '.';
        arm(); card._dkTimer = setInterval(step, 1000);
      }, 1200);
    }
    function step() { if (doc.hidden) return; left--; if (left <= 0) attemptNow(); else tick(); }
    card._dkRetry = attemptNow;
    arm(); card._dkTimer = setInterval(step, 1000);
  }

  /* =====================================================================
     3. Close to tray: ask once
     ===================================================================== */
  D.requestClose = function (opts) {
    if (!(opts && opts.force) && store('askOnClose') === false) return D.windowAction('hide');
    var opener = doc.activeElement;
    var host = doc.createElement('div');
    host.className = 'dk-modal';
    host.innerHTML = '<div class="dk-modal__scrim"></div>' +
      '<section class="card dk-card" role="dialog" aria-modal="true" aria-labelledby="dk-m-title" aria-describedby="dk-m-text">' +
      '<h2 class="dk-card__title" id="dk-m-title">keep playing in the tray?</h2>' +
      '<p id="dk-m-text">lull keeps playing with the window closed. Open it again or quit from the tray icon.</p>' +
      '<label class="switch-row"><span class="switch-row__label">Don&rsquo;t ask again</span><button type="button" class="switch" role="switch" aria-checked="true" data-dk="switch"></button></label>' +
      '<div class="dk-actions"><button type="button" class="btn" data-dk-close="quit">Quit lull</button>' +
      '<button type="button" class="btn btn--primary" data-dk-close="hide">Keep playing</button></div></section>';
    doc.body.appendChild(host);
    var focusables = $$('button', host);
    focusables[focusables.length - 1].focus();
    function close(action) {
      if (action) store('askOnClose', $('[role="switch"]', host).getAttribute('aria-checked') !== 'true');
      host.remove(); doc.removeEventListener('keydown', onKey, true);
      if (opener && opener.focus) opener.focus();
      if (action) D.windowAction(action);
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
     4. Mini player. In the wrapper these mirror the web UI's player (Media Session state over
        Tauri events); here a small mock player drives them.
        PROTO: TRACKS, P, startTicking and the `mini` handlers' state changes are the mock player.
     ===================================================================== */
  var TRACKS = [                    // PROTO: mock player data
    { title: 'Paper Moon Static', artist: 'Velvet Orchard', cover: 'c4', fmt: 'FLAC 16/44.1', ll: true, dur: 232, buf: 135, recv: .81 },
    { title: 'Lantern Season', artist: 'Glass Harbour', cover: 'c1', fmt: 'FLAC 24/96', ll: true, dur: 254, buf: 254, recv: 1 },
    { title: 'Glasshouse Bloom', artist: 'Moonpail', cover: 'c3', fmt: 'MP3 320', ll: false, dur: 198, buf: 90, recv: .62 }
  ];
  var P = { i: 0, pos: 84, playing: true, liked: { 0: true }, volume: .7, shuffle: false, repeat: true };
  function track() { return TRACKS[P.i]; }
  var BARS = 40;
  function barHeight(i) { return (0.32 + 0.6 * Math.abs(Math.sin(i * 1.7) * Math.cos(i * .43))).toFixed(2); }
  function chip(t) { return '<span class="q-chip ' + (t.ll ? 'q-chip--ll' : 'q-chip--ly') + ' q-chip--solid" title="' + (t.ll ? 'Lossless, ' : '') + t.fmt + '">' + t.fmt + '</span>'; }
  function cover(t, size) { return '<span class="cover ' + t.cover + (size <= 64 ? ' cover--thumb' : '') + '" style="--cover-size-base:' + size + 'px" aria-hidden="true"></span>'; }
  function waveText(t) { return fmt(P.pos) + ' of about ' + fmt(t.dur) + ', downloaded to ' + fmt(t.buf); }
  function waveMarkup() {
    var b = ''; for (var i = 0; i < BARS; i++) b += '<i class="wave__bar" style="--h:' + barHeight(i) + '"></i>';
    return '<div class="wave" role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax="100" data-dk="seek">' + b +
      '<div class="wave__rail"><div class="wave__head"></div></div></div>';
  }
  function nowMarkup(t) {
    return '<div class="dk-mini__now">' + cover(t, 84) + '<div><div class="device__title">' + esc(t.title) + '</div>' +
      '<div class="device__artist">' + esc(t.artist) + '</div><div class="device__facts">' + chip(t) +
      (t.recv < 1 ? '<span class="device__fact">peer ' + Math.round(t.recv * 100) + '%</span>' : '') + '</div></div></div>';
  }
  function playIcon() { return P.playing ? ic('pause') : ic('play', 'ic-play'); }
  function likeIcon() { return ic(P.liked[P.i] ? 'heart-fill' : 'heart'); }

  /* miniPlayer slot: a design.js may replace the window content. ctx = { style: 'device'|'strip', opaque,
     track { title, artist, format, lossless }, playing, liked, parts { device(), strip(), cover(size), wave() } }.
     Keep data-dk-player and the data-dk="..." buttons / data-dk-* fields in an override: the mock player
     (and the real wrapper bridge) drive those. */
  function miniPlayer(style, o) {
    var t = track();
    var ctx = { style: style, opaque: !!o.opaque, playing: P.playing, liked: !!P.liked[P.i],
      track: { title: t.title, artist: t.artist, format: t.fmt, lossless: t.ll },
      parts: { device: function () { return deviceMarkup(o); }, strip: stripMarkup, cover: function (size) { return cover(t, size); }, wave: waveMarkup } };
    var dflt = function (c) { return c.style === 'strip' ? stripMarkup() : deviceMarkup(o); };
    return window.Designs ? String(Designs.slot('miniPlayer', ctx, dflt)) : dflt(ctx);
  }
  D.miniDevice = function (o) { return miniPlayer('device', o || {}); };
  D.miniStrip = function () { return miniPlayer('strip', {}); };
  function deviceMarkup(o) {
    var t = track(), grill = ''; for (var g = 0; g < 9; g++) grill += '<i></i>';
    return '<div class="dk-mini' + (o.opaque ? ' dk-mini--opaque' : '') + '" data-dk-player="device">' +
      '<section class="device" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="dk-mini__head" data-tauri-drag-region><span class="dk-mini__led" aria-hidden="true"><i></i>on</span><span class="spacer" data-tauri-drag-region></span>' +
      '<button class="dk-mini__win" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="dk-mini__win" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button></div>' +
      '<div class="device__bezel" data-tauri-drag-region><div class="device__screen s-screen">' +
      '<div class="device__page"><div class="device__page-head"><span>now playing</span><span data-dk-vol>vol 70%</span></div>' +
      '<div data-dk-now aria-live="polite">' + nowMarkup(t) + '</div>' + waveMarkup() +
      '<div class="wave-times"><b data-dk-pos>' + fmt(P.pos) + '</b><span data-dk-dur>~' + fmt(t.dur) + '</span></div></div>' +
      '<div class="dk-mini__note" aria-hidden="true"></div></div>' +
      '<div class="device__brand">' + wordmark() + '</div></div>' +
      '<div class="device__controls"><div class="dpad">' +
      '<button class="dpad__btn dpad__btn--up" data-dk="vol-up" aria-label="Volume up" data-tip="Volume up">' + ic('plus') + '</button>' +
      '<button class="dpad__btn dpad__btn--left" data-dk="prev" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<span class="dpad__hub"></span>' +
      '<button class="dpad__btn dpad__btn--right" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="dpad__btn dpad__btn--down" data-dk="vol-down" aria-label="Volume down" data-tip="Volume down">' + ic('minus') + '</button></div>' +
      '<div class="device__ab"><span class="device__key device__key--b"><button class="btn-gloss btn-gloss--48' + (P.liked[P.i] ? ' is-liked' : '') + '" data-dk="like" aria-pressed="' + !!P.liked[P.i] + '" aria-label="Like" data-tip="Like">' + likeIcon() + '</button>like</span>' +
      '<span class="device__key device__key--a"><button class="btn-gloss" data-dk="play" aria-label="' + (P.playing ? 'Pause' : 'Play') + '" data-tip="' + (P.playing ? 'Pause' : 'Play') + '">' + playIcon() + '</button><span data-dk-play-label>' + (P.playing ? 'pause' : 'play') + '</span></span></div></div>' +
      '<div class="device__pills"><button class="device__pill device__pill--shuffle" data-dk="shuffle" aria-pressed="' + P.shuffle + '"><i></i>shuffle</button>' +
      '<button class="device__pill device__pill--repeat" data-dk="repeat" aria-pressed="' + P.repeat + '"><i></i>repeat</button></div>' +
      '<div class="device__grill" aria-hidden="true">' + grill + '</div></section></div>';
  };

  function stripMarkup() {
    var t = track();
    return '<div class="dk-strip-wrap" data-dk-player="strip"><section class="dk-strip s-panel" aria-label="lull mini player" data-tauri-drag-region>' +
      '<div class="seek seek--line" aria-hidden="true"><div class="seek__track"><div class="seek__xfer"></div><div class="seek__bufs"><div class="seek__buf" data-dk-buf></div></div><div class="seek__played"></div></div></div>' +
      '<button class="bezel-thumb" data-dk="full" aria-label="Open full window" data-tip="Open full window" data-dk-thumb>' + cover(t, 40) + '</button>' +
      '<div class="dk-strip__text" data-tauri-drag-region aria-live="polite"><div class="dk-strip__title" data-dk-title>' + esc(t.title) + '</div><div class="dk-strip__artist" data-dk-artist>' + esc(t.artist) + '</div></div>' +
      '<div class="dk-strip__keys">' +
      '<button class="btn-icon btn-icon--bare" data-dk="prev" aria-label="Previous" data-tip="Previous">' + ic('prev') + '</button>' +
      '<button class="btn-gloss btn-gloss--40" data-dk="play" aria-label="' + (P.playing ? 'Pause' : 'Play') + '" data-tip="' + (P.playing ? 'Pause' : 'Play') + '">' + playIcon() + '</button>' +
      '<button class="btn-icon btn-icon--bare" data-dk="next" aria-label="Next" data-tip="Next">' + ic('next') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm' + (P.liked[P.i] ? ' is-liked' : '') + '" data-dk="like" aria-pressed="' + !!P.liked[P.i] + '" aria-label="Like" data-tip="Like">' + likeIcon() + '</button>' +
      '<span class="dk-sep" aria-hidden="true"></span>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
      '<button class="btn-icon btn-icon--bare btn-icon--sm" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button>' +
      '</div></section></div>';
  };

  function paintPlayer(scope) {
    var t = track(), f = P.pos / t.dur;
    $$('[data-dk-player]', scope).forEach(function (el) {
      $$('.wave', el).forEach(function (w) {
        w.style.setProperty('--pos', f.toFixed(4));
        w.classList.toggle('is-playing', P.playing);
        $$('.wave__bar', w).forEach(function (b, i) {
          var x = (i + .5) / BARS;
          b.className = 'wave__bar' + (x <= f ? ' is-played' : x <= t.buf / t.dur ? ' is-buf' : x <= t.recv ? ' is-xfer' : '');
        });
        w.setAttribute('aria-valuenow', Math.round(f * 100)); w.setAttribute('aria-valuetext', waveText(t));
      });
      $$('[data-dk-pos]', el).forEach(function (n) { n.textContent = fmt(P.pos); });
      $$('[data-dk-dur]', el).forEach(function (n) { n.textContent = (t.recv < 1 ? '~' : '') + fmt(t.dur); });
      $$('.seek--line', el).forEach(function (s) {
        s.style.setProperty('--pos', f.toFixed(4)); s.style.setProperty('--xfer', t.recv);
        var buf = $('[data-dk-buf]', s); buf.style.left = '0'; buf.style.width = (t.buf / t.dur * 100) + '%';
      });
      $$('[data-dk="play"]', el).forEach(function (b) {
        var label = P.playing ? 'Pause' : 'Play';
        b.innerHTML = playIcon(); b.setAttribute('aria-label', label); b.setAttribute('data-tip', label);
      });
      $$('[data-dk-play-label]', el).forEach(function (n) { n.textContent = P.playing ? 'pause' : 'play'; });
      $$('[data-dk="like"]', el).forEach(function (b) {
        var on = !!P.liked[P.i];
        b.innerHTML = likeIcon(); b.setAttribute('aria-pressed', String(on)); b.classList.toggle('is-liked', on);
        b.setAttribute('data-tip', on ? 'Liked' : 'Like');
      });
      $$('[data-dk="shuffle"]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(P.shuffle)); });
      $$('[data-dk="repeat"]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(P.repeat)); });
    });
  }
  function changeTrack(scope, dir) {
    P.i = (P.i + dir + TRACKS.length) % TRACKS.length; P.pos = 0;
    var t = track();
    $$('[data-dk-now]', scope).forEach(function (n) { n.innerHTML = nowMarkup(t); });
    $$('[data-dk-thumb]', scope).forEach(function (n) { n.innerHTML = cover(t, 40); });
    $$('[data-dk-title]', scope).forEach(function (n) { n.textContent = t.title; });
    $$('[data-dk-artist]', scope).forEach(function (n) { n.textContent = t.artist; });
    paintPlayer(scope);
  }
  function note(scope, text) {
    $$('.dk-mini__note', scope).forEach(function (n) {
      n.textContent = text; n.classList.add('is-on');
      clearTimeout(n._t); n._t = setTimeout(function () { n.classList.remove('is-on'); }, 1600);
    });
    announce(text);
  }
  var ticking = null;
  function startTicking(scope) {
    if (ticking) return;
    ticking = setInterval(function () {
      if (!P.playing || doc.hidden) return;
      P.pos += 1;
      if (P.pos >= track().dur) return changeTrack(scope, 1);
      if (P.pos > track().buf) { P.pos = track().buf; }       // mock: playback waits at the buffered edge
      paintPlayer(scope);
    }, 1000);
  }

  /* =====================================================================
     5. One delegated handler for everything above
     ===================================================================== */
  var mini = {
    play: function (s) { P.playing = !P.playing; paintPlayer(s); announce(P.playing ? 'Playing' : 'Paused'); },
    next: function (s) { changeTrack(s, 1); },
    prev: function (s) { if (P.pos > 3) { P.pos = 0; paintPlayer(s); } else changeTrack(s, -1); },
    like: function (s) { P.liked[P.i] = !P.liked[P.i]; paintPlayer(s); announce(P.liked[P.i] ? 'Liked' : 'Removed from liked'); },
    'vol-up': function (s) { P.volume = Math.min(1, P.volume + .1); vol(s); },
    'vol-down': function (s) { P.volume = Math.max(0, P.volume - .1); vol(s); },
    shuffle: function (s) { P.shuffle = !P.shuffle; paintPlayer(s); note(s, P.shuffle ? 'shuffle on' : 'shuffle off'); },
    repeat: function (s) { P.repeat = !P.repeat; paintPlayer(s); note(s, P.repeat ? 'repeat all' : 'repeat off'); },
    full: function () { D.windowAction('full'); },
    'mini-close': function () { D.windowAction('mini-close'); }
  };
  function vol(s) {
    var pct = Math.round(P.volume * 100) + '%';
    $$('[data-dk-vol]', s).forEach(function (n) { n.textContent = 'vol ' + pct; });
    note(s, 'volume ' + pct);
  }
  function seekTo(wave, frac) {
    var t = track(), want = frac * t.dur, limit = t.buf;
    var scope = wave.closest('[data-dk-player]');
    if (want > limit) {
      P.pos = limit; note(scope, 'Not downloaded yet (' + Math.round(t.recv * 100) + '%)');
    } else P.pos = Math.max(0, want);
    paintPlayer(scope);
  }

  doc.addEventListener('click', function (e) {
    var el = e.target.closest('[data-dk]'); if (!el) return;
    var act = el.getAttribute('data-dk'), scope = el.closest('[data-dk-player]');
    if (scope && mini[act]) return mini[act](scope);
    var card = el.closest('[data-dk-connect]');
    switch (act) {
      case 'minimize': return D.windowAction('minimize');
      case 'maximize': if (!currentWindow()) D.setMaximized(!maximized); return D.windowAction('maximize', { maximized: maximized });
      case 'close': return D.requestClose();
      case 'switch': el.setAttribute('aria-checked', String(el.getAttribute('aria-checked') !== 'true')); return;
      case 'reveal':
        var input = $('#' + el.getAttribute('aria-controls')), show = input.type === 'password';
        input.type = show ? 'text' : 'password'; el.textContent = show ? 'Hide' : 'Show'; el.setAttribute('aria-pressed', String(show)); return;
      case 'test': return runTest(card, false);
      case 'cancel': clearTimeout(card._dkTimer); setConnectState(card, 'idle', $('[name="url"]', card).value); return;
      case 'retry': var off = el.closest('[data-dk-offline]'); if (off && off._dkRetry) off._dkRetry(); return;
      case 'change-server': return D.windowAction('change-server');
      case 'seek': return seekTo(el, Math.min(1, Math.max(0, (e.clientX - el.getBoundingClientRect().left) / el.offsetWidth)));
    }
  });
  doc.addEventListener('submit', function (e) {
    var card = e.target.closest('[data-dk-connect]'); if (!card) return;
    e.preventDefault();
    if (card.getAttribute('data-state') === 'ok') D.windowAction('connected', { url: $('[name="url"]', card).value });
    else runTest(card, true);
  });
  doc.addEventListener('keydown', function (e) {
    var w = e.target.closest && e.target.closest('.wave[data-dk="seek"]'); if (!w) return;
    var t = track(), step = e.key === 'ArrowRight' ? 5 : e.key === 'ArrowLeft' ? -5 : e.key === 'Home' ? -1e9 : e.key === 'End' ? 1e9 : 0;
    if (!step) return;
    e.preventDefault(); seekTo(w, Math.min(1, Math.max(0, (P.pos + step) / t.dur)));
  });

  /* start whatever the markup in root needs: retry countdown, mock player clock */
  D.bind = function (scope) {
    scope = scope || doc;
    $$('[data-dk-offline]', scope).forEach(function (card) { if (!card._dkRetry) startRetry(card); });
    if ($('[data-dk-player]', scope)) { paintPlayer(scope); startTicking(scope); }
  };

  /* =====================================================================
     6. Tooltips when the prototype's App isn't on the page
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
     7. PROTO: prototype integration: when loaded after app.js, list every desktop state in the switcher.
        ?titlebar=win|gnome on any route previews the title bar inside the real shell.
     ===================================================================== */
  if (window.App && App.registerStates) {
    var CONNECT = ['idle', 'testing', 'ok', 'unreachable', 'rejected', 'invalid'];
    var LABEL = { idle: 'first run', testing: 'testing', ok: 'connected', unreachable: 'server unreachable', rejected: 'token rejected', invalid: 'address not valid' };
    App.route('/desktop', {
      title: 'Desktop app', nav: 'settings',
      render: function (ctx) {
        var st = ctx.state || 'desktop-connect-idle';
        var body = st === 'desktop-offline' ? D.offlinePage({ inline: true }) : D.connectPage({ inline: true, state: st.replace('desktop-connect-', '') });
        return '<div class="dk-proto"><h1 class="t-display">desktop app</h1><p class="fg-2 dk-proto__lede">Screens the desktop wrapper shows before the web UI loads. ' +
          '<a class="btn btn--quiet" href="../desktop/preview.html">Open the desktop design board</a></p>' + body + '</div>';
      },
      mount: function (ctx) {
        D.bind(ctx.el);
        var card = $('[data-dk-connect]', ctx.el);
        if (card && card.getAttribute('data-state') === 'testing') runTest(card, false);
        ctx.onCleanup(function () { $$('[data-dk-offline], [data-dk-connect]', ctx.el).forEach(function (c) { clearInterval(c._dkTimer); clearTimeout(c._dkTimer); }); });
      }
    });
    App.registerStates('desktop', CONNECT.map(function (s) {
      return { id: 'desktop-connect-' + s, label: 'Desktop: connect, ' + LABEL[s], route: '/desktop' };
    }).concat([
      { id: 'desktop-offline', label: 'Desktop: server unreachable at launch', route: '/desktop' },
      { id: 'desktop-titlebar-win', label: 'Desktop: Windows title bar in the app header', route: '/home' },
      { id: 'desktop-titlebar-gnome', label: 'Desktop: Linux lull title bar (GNOME layout)', route: '/home' },
      { id: 'desktop-close-dialog', label: 'Desktop: keep playing in the tray? (first close)', route: '/home' }
    ]));
    var SHELL_STATE = { 'desktop-titlebar-win': 'win', 'desktop-titlebar-gnome': 'gnome', 'desktop-close-dialog': 'win' };
    App.on('route', function () {
      var cur = App.current || {}, q = cur.query || {};
      var shell = q.titlebar || SHELL_STATE[cur.state];
      if (shell === 'win' || shell === 'gnome') D.attachTitlebar(shell); else D.detachTitlebar();
      if (cur.state === 'desktop-close-dialog') setTimeout(function () { D.requestClose({ force: true }); }, 50);
    });
    var MOCK_TEXT = {              // PROTO: toasts that stand in for the wrapper's window actions
      minimize: 'The wrapper minimises the window.', maximize: 'The wrapper maximises or restores the window.',
      hide: 'The window hides; lull keeps playing in the tray.', quit: 'lull quits.',
      full: 'The full window opens.', 'mini-close': 'The mini player closes.', 'change-server': 'Opens the connect screen.',
      connected: 'The wrapper saves the server and loads the web UI from it.'
    };
    doc.addEventListener('dk:window', function (e) {
      if (App.toast) App.toast(MOCK_TEXT[e.detail.action] || 'Window action', { icon: 'info' });
    });
  }

  /* =====================================================================
     8. Tauri bridge (inside the wrapper only, with the web UI's App on the page; DESKTOP.md section 4):
        'tray' events run the web UI's own actions; set_now_playing keeps the tray labels and tooltip current.
     ===================================================================== */
  var TT = tauri();
  if (window.App && App.player && TT && TT.event && TT.core) {
    TT.event.listen('tray', function (e) {
      var a = e.payload && e.payload.action, id = App.player.currentId();
      if (a === 'play-pause') App.player.toggle();
      else if (a === 'next') App.player.next();
      else if (a === 'previous') App.player.prev();
      else if (a === 'like' && id) App.toggleLike(id);
      else if (a === 'save' && id && !App.isSaved(id)) App.saves.start([id]);
      else if (a === 'classic') App.go('/classic');
    });
    var lastNow = '';
    var sendNow = function () {
      var t = App.player.current(), id = t ? t.id : null;
      var now = { title: t ? t.title : '', artist: t ? t.artist : '', playing: !!App.player.state.playing,
        liked: !!(id && App.isLiked(id)), saved: !!(id && App.isSaved(id)),
        connected: !App.banner.list().some(function (b) { return b.id === 'sys-unreachable'; }) };
      var key = JSON.stringify(now); if (key === lastNow) return; lastNow = key;
      TT.core.invoke('set_now_playing', now).catch(function (err) { console.error('set_now_playing failed', err); });
    };
    ['player', 'like', 'save', 'ready'].forEach(function (ev) { App.on(ev, sendNow); });
    sendNow();
  }
})();
