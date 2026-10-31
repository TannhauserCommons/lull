/* Desktop design board (preview.html) and its window pages (window.html). Board-only code:
   the wrapper itself uses desktop.js/desktop.css. Classic script after icons.js + desktop.js. */
(function () {
  'use strict';
  var D = window.LullDesktop, doc = document, root = doc.documentElement;
  function $(s, r) { return (r || doc).querySelector(s); }
  function $$(s, r) { return Array.prototype.slice.call((r || doc).querySelectorAll(s)); }
  function ic(n, cls) { return window.icon(n, null, cls); }
  function params(str) {
    var o = {};
    String(str).split('&').forEach(function (kv) { var p = kv.split('='); if (p[0]) o[p[0]] = decodeURIComponent(p[1] || ''); });
    return o;
  }

  /* =====================================================================
     window.html: one window's content
     ===================================================================== */
  function cover(c, size, thumb) { return '<span class="cover ' + c + (thumb ? ' cover--thumb' : '') + '" style="--cover-size-base:' + size + '" aria-hidden="true"></span>'; }
  function glyphSvg(state) { var n = state === 'ok' ? 'status-ok' : state === 'warn' ? 'status-warn' : 'status-down'; return ic(n, 'sg sg--' + state); }
  function nav(name, label, on) {
    return '<a class="nav-item" href="#"' + (on ? ' aria-current="page"' : '') + '>' + ic(name) + '<span class="nav-item__label">' + label + '</span></a>';
  }
  function tile(c, title, by, meta, isNew) {
    return '<div class="tile"><div class="tile__art">' + cover(c, '100%') + '<span class="tile__play">' + ic('play') + '</span></div>' +
      '<div class="tile__title">' + title + '</div><div class="tile__by">' + by + '</div>' +
      '<div class="tile__meta">' + (isNew ? '<span class="badge-new">new</span>' : '') + '<span>' + meta + '</span></div></div>';
  }
  function pill(name, label, on) {
    return '<a class="nav-pill" href="#"' + (on ? ' aria-current="page"' : '') + '>' + ic(name) + '<span class="nav-pill__label">' + label + '</span></a>';
  }
  function brand(where) {
    return String(Designs.slot('brand', { where: where }, Designs.brandMarkup));
  }
  /* a still of the app shell (same classes and hooks as prototype/index.html, every shell variant), inert */
  function appPicture() {
    return '<div class="sky-specks" aria-hidden="true"></div>' +
      '<p class="vh">Still picture of the lull app. The live app is prototype/index.html.</p>' +
      '<div class="app" inert>' +
      '<nav class="rail s-panel" data-region="rail"><div class="rail__head">' + brand('rail') + '</div>' +
      '<div class="rail__nav">' + nav('home', 'Home', true) + nav('search', 'Search') + nav('library', 'Library') + nav('playlists', 'Playlists') + nav('stats', 'Stats') + nav('settings', 'Settings') + '</div>' +
      '<div class="rail__scroll"><div class="rail__section"><div class="label-rule label-rule--sm"><h2>following</h2></div></div>' +
      '<div class="rail__tags"><span class="chip">dream-gaze</span><span class="chip">city-pop revival</span><span class="chip">chiptune-folk</span></div>' +
      '<div class="rail__section"><div class="label-rule label-rule--sm"><h2>playlists</h2></div></div><ul class="rail__list">' +
      '<li><span class="rail__pl">' + cover('c5', '36px', true) + '<span><b>sleepy sunday</b><small>9 songs</small></span></span></li>' +
      '<li><span class="rail__pl">' + cover('c4', '36px', true) + '<span><b>3am window seat</b><small>8 songs</small></span></span></li></ul></div>' +
      '<div class="rail__foot"><span>Press <kbd class="kbd">?</kbd> for shortcuts</span><span>v0.1</span></div></nav>' +
      '<header class="header" data-region="header"><div class="header__brand">' + brand('header') + '</div>' +
      '<nav class="header__nav">' + pill('home', 'Home', true) + pill('library', 'Library') + pill('playlists', 'Playlists') + pill('stats', 'Stats') + '</nav>' +
      '<div class="header__search search-field">' + ic('search') +
      '<input class="input" type="search" placeholder="Search songs, albums, artists" aria-label="Search"><span class="search-field__end"><kbd class="kbd">/</kbd></span></div>' +
      '<div class="spacer"></div><div class="header__tools"><span class="tray tray--full">' +
      '<span class="tray__pill">' + glyphSvg('ok') + 'soulseek</span><span class="tray__pill">' + glyphSvg('ok') + 'jellyfin</span>' +
      '<span class="tray__pill">' + glyphSvg('warn') + 'last.fm 2<span class="tray__label-long">&nbsp;queued</span></span>' +
      '<span class="tray__pill">' + glyphSvg('ok') + '<span class="tray__label-long">cache&nbsp;</span>3.1/5G</span></span>' +
      '<span class="tray tray--compact"><span class="tray__pill" data-worst="warn">' + glyphSvg('ok') + glyphSvg('ok') + glyphSvg('warn') + '<span class="tray__word">last.fm queued</span><span class="tray__gb">3.1G</span></span></span>' +
      '<span class="btn-icon header__shelf">' + ic('follow') + '</span><span class="btn-icon header__settings">' + ic('settings') + '</span>' +
      '<span class="btn-icon">' + ic('activity') + '<span class="btn-icon__badge">2</span></span>' +
      '<span class="btn-icon">' + ic(root.getAttribute('data-theme') === 'day' ? 'moon' : 'sun') + '</span></div></header>' +
      '<div class="banner-slot"></div>' +
      '<div class="main" data-region="content"><main class="view"><div class="view__inner">' +
      '<h1 class="t-display pv-greeting">good evening</h1><p class="fg-2 pv-summary">3 new releases from artists you follow, and fresh picks from dream-gaze.</p>' +
      '<section class="card" data-head="rule"><div class="label-rule card-head"><h2 class="card-head__title">new releases</h2><span class="btn btn--quiet">See all</span></div><div class="shelf">' +
      tile('c1', 'Lantern Season', 'Glass Harbour', 'album, Oct 29', true) + tile('c2', 'Snow on the Antenna', 'Juniper Coast', 'EP, Oct 27', true) +
      tile('c5', 'Pocket Valentine', 'Tiny Orbit', 'single, Oct 24') + tile('c4', 'Harbour Lights', 'Neon Wren', 'album, Oct 20') +
      tile('c7', 'Marble Soda Club', 'Pastel Relay', 'album, Oct 18') + '</div></section>' +
      '</div></main></div>' +
      '<section class="pbar s-panel" data-region="playerbar"><span class="region-title" aria-hidden="true">now playing</span><div class="pbar__now"><span class="bezel-thumb">' + cover('c4', '52px', true) + '</span>' +
      '<div class="pbar__text"><div class="pbar__title"><span>Paper Moon Static</span></div><div class="pbar__artist">Velvet Orchard</div>' +
      '<div class="pbar__badges"><span class="badge badge-slsk">' + ic('soulseek') + 'Soulseek</span><span class="q-chip q-chip--ll q-chip--solid">FLAC 16/44.1</span></div></div></div>' +
      '<div class="pbar__centre"><div class="pbar__controls"><span class="btn-icon btn-icon--bare">' + ic('shuffle') + '</span><span class="btn-icon btn-icon--bare">' + ic('prev') + '</span>' +
      '<span class="btn-gloss">' + ic('pause') + '</span><span class="btn-icon btn-icon--bare">' + ic('next') + '</span><span class="btn-icon btn-icon--bare" aria-pressed="true">' + ic('repeat') + '</span></div>' +
      '<div class="pbar__seek seek-row"><span class="seek-row__time">1:24</span><div class="seek" style="--pos:.36;--xfer:.81"><div class="seek__track"><div class="seek__xfer"></div>' +
      '<div class="seek__bufs"><div class="seek__buf" style="left:0;width:58%"></div></div><div class="seek__played"></div></div><div class="seek__rail"><div class="seek__knob"></div></div></div>' +
      '<span class="seek-row__time">~3:52</span></div></div>' +
      '<div class="pbar__tools"><span class="btn-icon btn-icon--bare is-liked">' + ic('heart-fill') + '</span><span class="btn-icon btn-icon--bare">' + ic('save') + '</span>' +
      '<span class="btn-icon btn-icon--bare">' + ic('lyrics') + '</span><span class="btn-icon btn-icon--bare">' + ic('queue') + '</span><span class="btn-icon btn-icon--bare">' + ic('device') + '</span></div></section>' +
      '</div>';
  }

  var COVER_TEXT = {
    minimize: ['Minimised to the taskbar', 'Restore'],
    hide: ['lull is in the tray', 'Show lull', 'Still playing Paper Moon Static. Click the tray icon or use its menu.'],
    quit: ['lull has quit', 'Open lull', 'Playback stopped. The tray icon is gone.']
  };
  function windowCover(action) {
    var t = COVER_TEXT[action];
    var el = doc.createElement('div');
    el.className = 'pv-cover s-sky';
    el.innerHTML = '<div class="pv-cover__card card"><p class="t-h2">' + t[0] + '</p>' + (t[2] ? '<p class="fg-2">' + t[2] + '</p>' : '') +
      '<button type="button" class="btn btn--primary" data-pv-restore>' + t[1] + '</button></div>';
    doc.body.appendChild(el);
    var b = $('[data-pv-restore]', el); b.focus();
    b.addEventListener('click', function () { el.remove(); });
  }
  function windowNote(text) {
    var n = $('.pv-wnote');
    if (!n) { n = doc.createElement('div'); n.className = 'pv-wnote'; n.setAttribute('role', 'status'); doc.body.appendChild(n); }
    n.textContent = text; n.classList.add('is-on');
    clearTimeout(n._t); n._t = setTimeout(function () { n.classList.remove('is-on'); }, 1800);
  }

  function renderWindow() {
    var q = params(location.hash.slice(1)), view = q.view || 'app', os = q.os || 'win';
    var host = $('#pv-root');
    if (!host) { host = doc.createElement('div'); host.id = 'pv-root'; doc.body.appendChild(host); }
    D.detachTitlebar();
    var specks = '<div class="sky-specks" aria-hidden="true"></div>';
    if (view === 'connect') host.innerHTML = specks + D.connectPage({ state: q.state });
    else if (view === 'offline') host.innerHTML = specks + D.offlinePage();
    else if (view === 'mini') host.innerHTML = D.miniDevice({ opaque: q.opaque === '1' });
    else if (view === 'strip') host.innerHTML = D.miniStrip();
    else host.innerHTML = appPicture();
    doc.title = ({ app: 'Home', connect: 'Connect', offline: 'Not connected', mini: 'Mini player', strip: 'Mini player' })[view] + ' - lull';
    if (/^(app|connect|offline)$/.test(view) && (os === 'win' || os === 'gnome')) D.attachTitlebar(os, q.layout);
    if (view === 'app' && os === 'gnome') {          // the header-bar buttons sit inside the inert still: keep only them live
      var still = $('.app', host); still.removeAttribute('inert');
      $$('.app > *, .app > .header > *', host).forEach(function (el) { if (!el.classList.contains('header') && !el.classList.contains('dk-hb')) el.setAttribute('inert', ''); });
    }
    root.classList.toggle('pv-show-drag', q.drag === '1');
    D.bind(host);
    if (q.state === 'testing') { var t = $('[data-dk="test"]', host); if (t) t.click(); }
    if (q.dialog === '1') D.requestClose({ force: true });
  }

  if (doc.body.classList.contains('pv-window')) {
    Designs.ready(renderWindow);                     // after the design's design.js registered its slots
    window.addEventListener('hashchange', function () {
      var m = /(?:^|&)theme=(day|night)/.exec(location.hash.slice(1)); if (m) Designs.applyTheme(m[1]);
      var want = Designs.wanted();                   // #...&design=x on a frame that already booted: swap, then repaint
      if (want !== Designs.current) Designs.set(want, renderWindow); else renderWindow();
    });
    doc.addEventListener('dk:window', function (e) {
      var a = e.detail.action, q = params(location.hash.slice(1));
      if (a === 'maximize') { if (parent !== window) parent.postMessage({ pv: 'maximize', on: e.detail.maximized }, '*'); }
      else if (COVER_TEXT[a]) windowCover(a);
      else if (a === 'change-server') location.hash = 'view=connect&os=' + (q.os || 'win') + '&theme=' + (q.theme || 'night');
      else if (a === 'connected') location.hash = 'view=app&os=' + (q.os || 'win') + '&theme=' + (q.theme || 'night');
      else if (a === 'full') windowNote('Opens the full window');
      else if (a === 'mini-close') windowNote('Closes the mini player');
    });
    return;
  }

  /* =====================================================================
     preview.html: the board
     ===================================================================== */
  /* every window frame shows the board's design (?design= flag, else the stored pref). The design goes in the frame's
     query string, not its hash: a src that differs only in the hash is a same-document navigation, so a frame that
     already booted would keep its old design. */
  $$('iframe[src^="window.html"]').forEach(function (f) {
    var src = f.getAttribute('src'), h = src.indexOf('#');
    f.src = 'window.html?design=' + Designs.current + (h >= 0 ? src.slice(h) : '');
  });
  /* the board header wordmark goes through the design's brand slot once its design.js has loaded */
  Designs.ready(function () {
    var b = $('[data-pv-brand]'); if (b) b.innerHTML = brand('desktop');
  });
  /* scale fixed-size window frames to their column (frames keep their true pixel size inside) */
  function fit() {
    $$('.pv-fit').forEach(function (f) {
      var w = +f.getAttribute('data-w'), h = +f.getAttribute('data-h');
      var s = Math.min(1, f.clientWidth / w);
      var inner = f.firstElementChild;
      inner.style.width = w + 'px'; inner.style.height = h + 'px';
      inner.style.transform = s < 1 ? 'scale(' + s + ')' : '';
      f.style.height = Math.ceil(h * s) + 'px';
    });
  }
  window.addEventListener('resize', fit);
  fit();

  /* a window page reports maximise: square its frame's corners */
  window.addEventListener('message', function (e) {
    if (!e.data || e.data.pv !== 'maximize') return;
    $$('iframe').forEach(function (f) {
      if (f.contentWindow === e.source) { var fr = f.closest('.pv-frame'); if (fr) fr.classList.toggle('is-max', !!e.data.on); }
    });
  });

  /* connect states rendered straight into the board (cards are cream in both themes) */
  $$('[data-pv-connect]').forEach(function (el, i) {
    el.innerHTML = D.connectPage({ inline: true, state: el.getAttribute('data-pv-connect'), idPrefix: 'pv-c' + i });
  });
  D.bind(doc);

  /* board theme toggle (the window frames carry their own theme) */
  var themeBtn = $('[data-pv-theme]');
  function syncThemeBtn() {
    var night = root.getAttribute('data-theme') === 'night';
    themeBtn.innerHTML = ic(night ? 'sun' : 'moon');
    themeBtn.setAttribute('aria-label', night ? 'Switch the board to day' : 'Switch the board to night');
    themeBtn.setAttribute('data-tip', night ? 'Day board' : 'Night board');
    return night;
  }
  themeBtn.addEventListener('click', function () {
    var night = syncThemeBtn();
    Designs.applyTheme(night ? 'day' : 'night');
    syncThemeBtn();
  });
  syncThemeBtn();

  /* tray menus: native menus mocked. Clicking an item logs the event the web UI receives. */
  var tray = { playing: true, liked: true, saved: false, mini: false };
  var EVENT = {
    'play-pause': function () { tray.playing = !tray.playing; return 'tray { action: "play-pause" } -> web UI toggles playback'; },
    next: function () { return 'tray { action: "next" } -> web UI plays the next song'; },
    previous: function () { return 'tray { action: "previous" } -> web UI goes back'; },
    like: function () { tray.liked = !tray.liked; return 'tray { action: "like" } -> web UI ' + (tray.liked ? 'likes' : 'unlikes') + ' the song'; },
    save: function () { tray.saved = true; return 'tray { action: "save" } -> web UI starts the save; the item becomes "Saved"'; },
    show: function () { return 'tray { action: "show" } -> wrapper shows and focuses the main window'; },
    mini: function () { tray.mini = !tray.mini; return 'tray { action: "mini" } -> wrapper ' + (tray.mini ? 'opens' : 'closes') + ' the mini player'; },
    classic: function () { return 'tray { action: "classic" } -> wrapper shows the window, web UI opens Classic mode'; },
    quit: function () { return 'wrapper quits (no event to the web UI)'; }
  };
  function paintMenus() {
    $$('[data-tray-act="play-pause"] .pv-menu__label').forEach(function (n) { n.textContent = tray.playing ? 'Pause' : 'Play'; });
    $$('[data-tray-act="like"]').forEach(function (b) { b.setAttribute('aria-checked', String(tray.liked)); });
    $$('[data-tray-act="mini"]').forEach(function (b) { b.setAttribute('aria-checked', String(tray.mini)); });
    $$('[data-tray-act="save"]').forEach(function (b) {
      b.disabled = tray.saved; $('.pv-menu__label', b).textContent = tray.saved ? 'Saved' : 'Save to library';
    });
  }
  doc.addEventListener('click', function (e) {
    var b = e.target.closest('[data-tray-act]'); if (!b) return;
    var line = EVENT[b.getAttribute('data-tray-act')]();
    paintMenus();
    var log = $('[data-pv-log]'); log.textContent = line;
  });
  doc.addEventListener('keydown', function (e) {          // arrow keys move within a menu, like the native one
    var b = e.target.closest && e.target.closest('[data-tray-act]'); if (!b || (e.key !== 'ArrowDown' && e.key !== 'ArrowUp')) return;
    var items = $$('[data-tray-act]:not(:disabled)', b.closest('[role="menu"]')), i = items.indexOf(b);
    e.preventDefault(); items[(i + (e.key === 'ArrowDown' ? 1 : items.length - 1)) % items.length].focus();
  });
  paintMenus();

  /* maskable preview: toggle the 80% safe-zone circle */
  var safe = $('[data-pv-safe]');
  if (safe) safe.addEventListener('click', function () {
    var on = safe.getAttribute('aria-pressed') !== 'true';
    safe.setAttribute('aria-pressed', String(on));
    $('.pv-mask').classList.toggle('is-safe', on);
  });
})();
