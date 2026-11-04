/* screens/classic.js: Classic mode (/classic). Webamp plays the queue; nothing of the app shows behind it.
   - In the desktop app (LullDesktop.openClassic), Webamp gets its own borderless window, like Winamp itself, and the
     app's window hides (app/src/desktop/classic.rs, docs/context/desktop.md). This page hands over the queue, leaves
     /classic at once, records what the window reports hearing, and takes the queue back when Webamp closes.
   - In a browser, Webamp sits alone on a full-viewport stage on <body> over the hidden app, with one quiet way
     back (Esc, W, Webamp's close button, or the small Back to lull button). design/DESIGN.md section 12 is the
     design; docs/context/web.md the data layer and the player engine.
   - The skin and the size are the ones Settings > Appearance > Classic chose (screens/skins.js, App.classicSkin and
     App.classicSize): on when Webamp starts and whenever the choice changes ('classic-skin', 'classic-size'). Each
     opening asks App.classicSkin.opening() first, which picks a new random skin when the listener wants one.
     Skins load with LullWebamp.loadSkin. Webamp's alerts are lull's toasts or the stage's sentence
     (LullWebamp.quietAlerts); a skin that fails leaves the previous one on.
   - Size: Webamp lives in a same-origin iframe (.cls-frame) drawn at the size with a transform, 1/size of the stage
     big, so the stage stays covered. Pointer events reach the iframe in its own coordinates, so every bit of
     Webamp's pointer math (window moves, playlist resize and drag, sliders, the marquee) stays right, and its
     image-rendering: pixelated keeps the skin sharp. A new size scales the windows about the page's centre; a size
     the stage can't hold gives way to the largest that fits. Webamp's Double Size (Ctrl+D, Options, the clutter
     bar's D) switches between 1x and 2x.

   Webamp 2.3.1 (vendor/webamp/webamp.bundle.min.js, copied by scripts/vendor-web.sh) loads in the iframe the first
   time the stage opens, never on boot. One instance renders into the iframe's #webamp-stage; later visits close()
   and reopen() it (dispose() still leaks), so the stage is kept: closed, it is hidden, not removed, since moving
   an iframe reloads it.
   - Hand-off (desktop/webamp-session.js, shared with the desktop window): on enter the app engine pauses and Webamp
     gets the queue (URLs from App.player.streamUrl) with title, artist, album and duration, plus the index, position,
     shuffle, repeat and volume. On exit the playlist, current song, position, shuffle and repeat come back into
     App.player. While open, App.hooks.media points media keys at Webamp and songs heard there are recorded with
     App.recordPlay. Webamp's Skins menu lists the favourite skins (GET /api/skins/favourites). */
(function () {
  'use strict';
  var html = App.html, M = App.data, P = App.player, icon = App.icon, $ = App.$;

  var BUNDLE = 'vendor/webamp/webamp.bundle.min.js';
  /* the iframe's page: Webamp's mount and its bundle; relative URLs resolve against this page, the CSP is this page's */
  var FRAME = '<!doctype html><html lang="en"><head><meta charset="utf-8"><style>'
    + 'html,body{margin:0;height:100%;overflow:hidden;background:transparent;color-scheme:normal}'
    + '#webamp-stage{position:absolute;inset:0}</style></head><body>'
    + '<div id="webamp-stage" data-webamp role="region" aria-label="Webamp"></div>'
    + '<script src="' + BUNDLE + '"></script></body></html>';

  var S = {
    player: { state: 'idle', message: '' },   // Webamp: idle | loading | ready | failed (message says why)
    skin: null                                // { state: loading | failed, name, message }
  };
  function onClassic() { return /^#\/classic(?:[/?]|$)/.test(location.hash); }
  function base() { return new URL('./', location.href).href; }
  function skinFileUrl(md5) { return M.url('/api/skins/' + encodeURIComponent(md5) + '.wsz'); }

  /* ---------- the stage (lives on <body>, above the hidden app; kept between visits, hidden) ---------- */
  var root = null, open = false, frame = null, lastPage = null, restore = null, timers = [], visit = 0;
  /* loaded on first use (app.js App.needClassic, in a browser): the page still showing is the one Classic is opened from */
  if (App.current.route && App.current.path !== '/classic') lastPage = { path: App.current.path, params: App.current.params, query: App.current.query, route: App.current.route };

  /* remember the page Classic was opened from: leaving returns to it */
  App.on('route', function (cur) {
    if (cur.path === '/classic') return;
    lastPage = { path: cur.path, params: cur.params, query: cur.query, route: cur.route };
    if (restore && restore.path === cur.path) {         // exit: scroll position and focus come back too
      var r = restore; restore = null;
      $('#view').scrollTop = r.scroll;
      if (r.opener && document.contains(r.opener) && r.opener.offsetParent !== null) r.opener.focus({ preventScroll: true });
    } else restore = null;
  });

  function lastHref() {
    if (!lastPage) return '/home';
    var q = Object.keys(lastPage.query)
      .map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(lastPage.query[k]); }).join('&');
    return lastPage.path + (q ? '?' + q : '');
  }

  /* the iframe Webamp runs in; a new one after a failed start (React may own the old mount) */
  function makeFrame() {
    var f = document.createElement('iframe');
    f.className = 'cls-frame'; f.title = 'Webamp';
    f.srcdoc = FRAME;
    var slot = $('.cls-slot', root);
    if (frame) slot.replaceChild(f, frame); else slot.appendChild(f);
    frame = f;
    scale = 1;
    return f;
  }
  /* the iframe's window once its page and Webamp's bundle have loaded */
  function frameReady() {
    var f = frame;
    return new Promise(function (resolve, reject) {
      function check() {
        var w = f.contentWindow;
        if (w && w.Webamp) resolve(w);
        else reject(new Error('the player file (' + BUNDLE + ') didn\'t load. Check your connection; a server built from source needs `mise run setup`.'));
      }
      if (f.contentDocument && f.contentDocument.readyState === 'complete' && f.contentWindow.Webamp) check();
      else f.addEventListener('load', check, { once: true });
    });
  }

  /* ---------- markup: Webamp, what it is doing when that needs saying, and the way back ---------- */
  function stageMarkup() {
    return html`<div class="cls-scrim" aria-hidden="true"></div>
      <h1 class="vh" id="cls-title">Classic mode</h1>
      <div class="cls-desk" id="cls-desk"><div class="cls-slot"></div><div class="cls-desk__status" id="cls-status">${statusMarkup()}</div></div>
      <button class="btn btn--sm cls-back" data-act="cls-exit" aria-keyshortcuts="Escape">${icon('chevron-left', null, 'ic-16')}Back to lull<kbd class="kbd" aria-hidden="true">Esc</kbd></button>`;
  }
  function statusMarkup() {
    if (S.player.state === 'loading') return busyBox('Starting Webamp', 'Loading the player, only the first time Classic mode opens');
    if (S.player.state === 'failed') {
      return html`<div class="cls-failed banner banner--down s-card" role="alert">
        ${icon('warning')}<span class="banner__msg">Webamp didn't start: ${S.player.message}</span>
        <button class="btn btn--sm" data-act="cls-start">Try again</button>
      </div>`;
    }
    var k = S.skin;
    if (!k) return '';
    if (k.state === 'loading') return busyBox('Loading ' + k.name, 'Your Classic skin, from Settings › Appearance');
    return html`<div class="cls-failed banner banner--down s-card" role="alert">
      ${icon('warning')}<span class="banner__msg">Couldn't load ${k.name}: ${k.message} Choose another in <a href="#/settings/appearance">Settings › Appearance</a>.</span>
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-dismiss" aria-label="Dismiss" data-tip="Dismiss">${icon('close', null, 'ic-16')}</button>
    </div>`;
  }
  function busyBox(title, text) {
    return html`<div class="cls-busy s-card" role="status">
      <span class="spin-ring" aria-hidden="true"></span>
      <div class="cls-busy__text"><strong>${title}</strong><span class="fg-2">${text}</span></div>
    </div>`;
  }
  function renderStatus() {
    if (!open) return;
    var box = $('#cls-status', root), hadFocus = box.contains(document.activeElement);
    box.innerHTML = String(statusMarkup());
    if (hadFocus) $('.cls-back', root).focus({ preventScroll: true });
  }

  /* ---------- Webamp: load, create once, reopen ---------- */
  var webamp = null, session = null, frameWin = null, webampClosed = false, closingByUs = false, prepared = null, creating = null;
  /* favourites for Webamp's Skins menu, once per page load; settles either way (the menu is then empty) */
  var favourites = null;
  function readFavourites() {
    if (!favourites) {
      favourites = M.api('/api/skins/favourites').then(function (res) {
        return ((res && res.items) || []).filter(function (s) { return s && /^[0-9a-f]{32}$/i.test(s.md5); })
          .map(function (s) { return { md5: s.md5, name: String(s.name || s.md5) }; });
      }, function (err) { console.warn('Webamp\'s Skins menu stays empty: favourites didn\'t load', err); favourites = null; return []; });
    }
    return favourites;
  }
  /* the iframe with the bundle, and what the constructor needs; a failed load can be retried with a new iframe */
  function prepare() {
    if (!prepared) {
      if (!frame) makeFrame();
      prepared = Promise.all([frameReady(), readFavourites()]).then(function (r) {
        frameWin = r[0];
        LullWebamp.quietAlerts(webampSaid, frameWin);
        wireFrame(frameWin.document);
        return [frameWin.Webamp, r[1]];
      }, function (err) { prepared = null; makeFrame(); throw err; });
    }
    return prepared;
  }
  function createWebamp(Webamp, favs) {
    if (!Webamp.browserIsSupported()) throw new Error('this browser lacks Web Audio or canvas, which Webamp needs.');
    var phone = window.innerWidth < 640;                       // the equalizer closes so 275 px fits a phone
    var w = new Webamp({
      availableSkins: favs.map(function (s) { return { url: skinFileUrl(s.md5), name: s.name }; }),
      windowLayout: {
        main: { position: { top: 0, left: 0 } },
        equalizer: { position: { top: 116, left: 0 }, closed: phone },
        playlist: { position: { top: phone ? 116 : 232, left: 0 }, size: { extraWidth: 0, extraHeight: 0 } }
      },
      enableHotkeys: false,                                     // lull owns the shortcuts (wireFrame)
      enableMediaSession: false,                                // and Media Session: the top frame's handlers drive Webamp
      handleTrackDropEvent: onWebampDrop,
      handleSaveListEvent: exportM3u,
      handleLoadListEvent: importM3u
    });
    w.onWillClose(function (cancel) { if (closingByUs) return; cancel(); exit(); });   // its close button = leave Classic
    var s = LullWebamp.session(w, { listen: App.recordPlay });
    LullWebamp.doubleSize(w, function () { App.classicSize.double(); });
    return w.renderInto(frameWin.document.getElementById('webamp-stage')).then(function () {
      webamp = w; session = s; shown = null;
    }, function (err) {
      w.dispose();
      prepared = null; makeFrame();                             // React may own the old mount: start clean next time
      throw err;
    });
  }
  /* every visit: make sure Webamp is up and the skin to open in is settled, then hand it the queue. A visit that ended
     meanwhile does nothing */
  function startWebamp() {
    var mine = visit;
    function current() { return mine === visit && open; }
    if (!webamp) { S.player = { state: 'loading', message: '' }; renderStatus(); }
    Promise.all([prepare(), App.classicSkin.opening()]).then(function (r) { return r[0]; }).then(function (r) {
      if (webamp || !current()) return null;
      if (!creating) {
        creating = createWebamp(r[0], r[1]).then(function () { creating = null; }, function (err) { creating = null; throw err; });
      }
      return creating;
    }).then(function () {
      if (current() && webamp) webampReady();
    }, function (err) {
      if (!current()) return;
      S.player = { state: 'failed', message: err && err.message || String(err) };
      renderStatus();
    });
  }
  function webampReady() {
    S.player = { state: 'ready', message: '' };
    if (webampClosed) { webamp.reopen(); webampClosed = false; }
    handOver();
    App.hooks.media = webampControls;                          // media keys and the lock screen drive Webamp now
    renderStatus();
    followSize();
    followSkin();
  }

  /* ---------- the skin: Settings' choice, on now and whenever it changes ---------- */
  var shown = null, skinJob = Promise.resolve();               // shown: the md5 Webamp shows (null: its base skin)
  function followSkin() {
    if (!webamp || !open) return;
    var skin = App.classicSkin.get(), md5 = skin ? skin.md5 : null;
    if (md5 === shown) return;
    var before = shown;
    shown = md5;                                                // asked for: the same choice again waits for this one
    if (skin) { S.skin = { state: 'loading', name: skin.name || 'your skin' }; renderStatus(); }
    skinJob = skinJob.then(function () {
      return LullWebamp.loadSkin(webamp, LullWebamp.skinUrl(base(), skin)).then(function () {
        if (S.skin && S.skin.state === 'loading') S.skin = null;
      }, function (err) {
        if (err.status === 401) M.onUnauthorized();
        if (shown === md5) shown = before;                      // Webamp kept the skin it had
        S.skin = { state: 'failed', name: skin.name || 'your skin', message: err.message };
        App.announce('Couldn\'t load ' + S.skin.name + '. ' + err.message);
      }).then(renderStatus);
    });
  }
  App.on('classic-skin', followSkin);

  /* ---------- the size: Settings' choice, or the largest that fits the stage; windows scale about its centre ---------- */
  var scale = 1, sizeSaid = null;
  var WINDOWS = ['main', 'equalizer', 'playlist'];
  function followSize() {
    if (!webamp || !open) return;
    var slot = $('.cls-slot', root), roomW = slot.clientWidth, roomH = slot.clientHeight;
    if (!roomW || !roomH) return;
    var want = App.classicSize.get(), box = windowsBox();
    var next = box ? LullWebamp.fitSize(want, box.w, box.h, roomW, roomH) : want;
    if (next < want && sizeSaid !== want + '>' + next) {
      sizeSaid = want + '>' + next;
      App.toast(want + '\u00d7 doesn\'t fit this window, so Classic shows ' + next + '\u00d7', { icon: 'info' });
    }
    if (next === scale) return;
    /* a window at x shows at x*scale; keeping the page's centre still moves every window by the same amount */
    var dx = roomW / 2 * (1 / next - 1 / scale), dy = roomH / 2 * (1 / next - 1 / scale);
    var positions = {}, wins = webamp.store.getState().windows.genWindows;
    WINDOWS.forEach(function (k) {
      var at = wins[k] && wins[k].position;
      if (at) positions[k] = { x: Math.round(at.x + dx), y: Math.round(at.y + dy) };
    });
    scale = next;
    frame.style.width = 100 / scale + '%'; frame.style.height = 100 / scale + '%';
    frame.style.transform = scale === 1 ? '' : 'scale(' + scale + ')';
    webamp.store.dispatch({ type: 'UPDATE_WINDOW_POSITIONS', positions: positions });   // Webamp keeps them on screen after
  }
  /* Webamp's open windows at 1x, in its own coordinates */
  function windowsBox() {
    var doc = frameWin.document, l = Infinity, t = Infinity, r = -Infinity, b = -Infinity;
    WINDOWS.forEach(function (k) {
      var el = doc.getElementById(k === 'main' ? 'main-window' : k + '-window');
      var rect = el && el.getBoundingClientRect();
      if (!rect || !rect.width) return;
      l = Math.min(l, rect.left); t = Math.min(t, rect.top); r = Math.max(r, rect.right); b = Math.max(b, rect.bottom);
    });
    return r > l ? { w: r - l, h: b - t } : null;
  }
  App.on('classic-size', followSize);
  /* what Webamp says outside a skin load (a skin from its own Skins menu, a menu item it lacks) */
  function webampSaid(message) {
    App.toast(/skin/i.test(message) ? 'Webamp couldn\'t load that skin' : 'Webamp: ' + message, { kind: 'error', icon: 'warning' });
  }

  /* ---------- hand-off between the app engine and Webamp (desktop/webamp-session.js has its shape) ---------- */
  var handedOver = false;
  function absolute(url) { return new URL(url, location.href).href; }
  function sameOrigin(url) { return new URL(url).origin === location.origin; }
  /* a queue entry as a hand-off track; duration spares Webamp fetching each file's head, art feeds the OS overlay */
  function handoffTrack(trackId, source) {
    var t = M.track(trackId), url = t && P.streamUrl(t.id, source);
    if (!url || !sameOrigin(absolute(url))) return null;
    var art = t.albumId ? absolute(App.artUrl(t.albumId, 256)) : null;
    return { id: t.id, url: absolute(url), title: t.title || '', artist: t.artist || '', album: t.album || '',
      duration: t.duration || null, art: art && sameOrigin(art) ? art : null,
      artistId: t.artistId || '', albumId: t.albumId || '', liked: App.isLiked(t.id), saved: App.isSaved(t.id),
      gainDb: P.replayGainDb(t.id) };
  }
  function handoff() {
    var ps = P.state, index = 0, tracks = [];
    ps.queue.forEach(function (q, i) {
      var t = handoffTrack(q.trackId, q.source);
      if (!t) return;
      t.recommended = q.group === 'radio';
      t.group = q.group; t.source = q.source || null; t.orig = q.orig == null ? null : q.orig;
      if (i === ps.index) index = tracks.length;
      tracks.push(t);
    });
    return { tracks: tracks, index: index, pos: ps.pos || 0, playing: !!ps.playing, shuffle: !!ps.shuffle,
      repeat: ps.repeat !== 'off', volume: ps.muted ? 0 : ps.volume };
  }
  function handOver() {
    var h = handoff();
    if (h.playing) P.pause();
    session.handIn(h);
    handedOver = true;
  }
  /* Webamp's list, song, position and modes back into the app engine (b: session.handBack()). Songs Webamp played
     from local files have no place in a lull queue and are left out */
  function takeBack(b) {
    /* from the native window (no webview): its equaliser, size and windows' layout, the plays and likes the hub couldn't take then (the
       outbox sends them), and the queue whole, played by this page's rules, radio and groups kept */
    if (b.player) {
      if (b.eq) LullEq.write(b.eq);
      if (b.size) App.classicSize.set(b.size);
      if (b.layout) writeLayout(b.layout);
      (b.ops || []).forEach(function (op) { App.outbox.add(op); });
      P.adopt(b.player, b.playing);
      return;
    }
    syncModes(b.shuffle, b.repeat);
    if (b.local) App.toast(b.local === 1 ? 'The local file you added in Webamp isn\'t in your queue' : 'The ' + b.local + ' local files you added in Webamp aren\'t in your queue', { icon: 'info' });
    if (!b.ids.length) return;                                  // nothing of lull's left in Webamp: the queue stays
    var ps = P.state, same = b.ids.length === ps.queue.length && b.ids.every(function (id, i) { return ps.queue[i].trackId === id; });
    if (!same) P.playList(b.ids, b.index, { at: b.pos, autoplay: b.playing, context: ps.context });   // the same listening, edited in Webamp
    else if (b.index !== ps.index) P.jumpTo(b.index, { at: b.pos, autoplay: b.playing });   // keeps radio and queue groups
    else {
      if (Math.abs(b.pos - ps.pos) > 1) P.seek(b.pos);
      if (b.playing) P.play();
    }
  }
  /* where native Classic's windows were (classic_native's Layout), per device as the size is: it comes back with the
     queue and goes with the next hand-off */
  var LAYOUT_KEY = 'lull:classicLayout';
  function readLayout() {
    try { return JSON.parse(localStorage.getItem(LAYOUT_KEY)); } catch (err) { console.warn('lull: Classic layout unreadable', err); return null; }
  }
  function writeLayout(layout) {
    try { localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout)); } catch (err) { console.warn('lull: Classic layout not kept', err); }
  }
  /* App.hooks.media while Classic is open */
  var webampControls = {
    play: function () { webamp.play(); }, pause: function () { webamp.pause(); },
    prev: function () { webamp.previousTrack(); }, next: function () { webamp.nextTrack(); },
    seek: function (sec) { webamp.seekToTime(sec); },
    seekBy: function (sec) { if (sec < 0) webamp.seekBackward(-sec); else webamp.seekForward(sec); }
  };

  /* Webamp's repeat is on or off: on keeps the app's all / one, off turns it off */
  function syncModes(shuffle, repeat) {
    var ps = P.state;
    if (shuffle !== ps.shuffle) P.setShuffle(shuffle);
    if (repeat && ps.repeat === 'off') P.cycleRepeat();
    while (!repeat && ps.repeat !== 'off') P.cycleRepeat();
  }

  /* files dropped on Webamp's windows: a .wsz becomes the Classic skin (kept on this device); lull songs dragged from
     another lull window join the list (and the queue on exit); anything else (audio files) Webamp plays by itself */
  function onWebampDrop(e) {
    var dt = e.dataTransfer, skinFiles = Array.prototype.filter.call(dt.files || [], function (f) { return /\.wsz$/i.test(f.name); });
    if (skinFiles.length) { App.classicSkin.keepFiles(skinFiles); return []; }
    var tracks = (dt.getData('text/plain') || '').split(',').map(function (id) { return handoffTrack(id.trim(), null); })
      .filter(Boolean).map(session.add);
    return tracks.length ? tracks : null;
  }
  /* Webamp's playlist menu. Save list: the lull songs in it as an M3U download. Load list: the library's M3U import,
     which matches each line and makes a lull playlist (Webamp's list stays as it is) */
  function exportM3u(tracks) {
    var lines = ['#EXTM3U'];
    tracks.forEach(function (wt) {
      var url = wt.url && absolute(wt.url), id = url && session.idOf(url), t = id && M.track(id);
      if (t) lines.push('#EXTINF:' + Math.round(t.duration || -1) + ',' + t.artist + ' - ' + t.title, url);
    });
    if (lines.length === 1) { App.toast('Nothing to save: the list holds no lull songs', { icon: 'info' }); return null; }
    var a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([lines.join('\n') + '\n'], { type: 'audio/x-mpegurl' }));
    a.download = 'lull-queue.m3u8';
    a.click();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 1000);   // the download has started by then
    App.toast('Saved the list as lull-queue.m3u8', { icon: 'check', kind: 'ok' });
    return null;
  }
  function importM3u() {
    var input = document.createElement('input');
    input.type = 'file'; input.accept = '.m3u,.m3u8,audio/x-mpegurl'; input.hidden = true;
    input.addEventListener('change', function () { if (input.files[0]) App.hooks.importFile(input.files[0]); input.remove(); });
    document.body.appendChild(input); input.click();
    return null;
  }

  /* ---------- actions ---------- */
  function exit() {
    if (!open || root.classList.contains('is-leaving')) return;
    root.classList.add('is-leaving');
    later(function () { App.go(lastHref()); }, App.reducedMotion() ? 130 : 200);
  }
  App.action('cls-exit', exit);
  App.action('cls-start', startWebamp);
  App.action('cls-dismiss', function () { S.skin = null; renderStatus(); });

  /* keyboard, on the stage and in Webamp's iframe: Esc and W leave Classic (W not inside Webamp), Ctrl+D is Webamp's
     Double Size, Tab stays on the stage (the app behind is hidden) */
  function onKey(e) {
    if (!open) return;
    if (document.querySelector('body > .popover, body > .menu, #overlay-host > .is-open')) return;   // the app's overlay owns Esc / Tab
    var a = e.target && e.target.nodeType === 1 ? e.target : document.activeElement;
    var typing = a && /^(INPUT|TEXTAREA)$/.test(a.tagName) && a.type !== 'file';
    var w = (e.key === 'w' || e.key === 'W') && !typing && !e.ctrlKey && !e.metaKey && !e.altKey && !(a && a.closest && a.closest('[data-webamp]'));
    if (e.key === 'Escape' || w) { e.preventDefault(); e.stopPropagation(); exit(); return; }
    if (LullWebamp.isDoubleKey(e)) {
      e.preventDefault(); e.stopPropagation(); App.classicSize.double(); return;
    }
    if (e.key === 'Tab') {
      var f = App.$$('a[href], button:not([disabled])', root).filter(function (n) { return n.offsetParent !== null; });
      var i = f.indexOf(a);
      if (i < 0 || (e.shiftKey && i === 0) || (!e.shiftKey && i === f.length - 1)) { e.preventDefault(); f[e.shiftKey ? f.length - 1 : 0].focus(); }
    }
  }
  /* what happens in the iframe that lull handles: its keys, and files dropped beside Webamp's windows (on them,
     Webamp takes them: onWebampDrop) go where a drop anywhere on lull goes */
  function wireFrame(doc) {
    doc.addEventListener('keydown', onKey, true);
    function files(e) { return e.dataTransfer && [].indexOf.call(e.dataTransfer.types || [], 'Files') >= 0; }
    doc.addEventListener('dragover', function (e) { if (files(e)) e.preventDefault(); });
    doc.addEventListener('drop', function (e) {
      if (!files(e) || e.defaultPrevented) return;
      e.preventDefault();
      App.openFiles([].slice.call(e.dataTransfer.files));
    });
  }
  function onResize() { followSize(); }

  function later(fn, ms) { var t = setTimeout(fn, ms); timers.push(t); return t; }

  /* ---------- open / close ---------- */
  function openStage() {
    visit += 1;
    open = true;
    if (restore) restore.path = lastPage ? lastPage.path : '/home';
    if (!root) {
      root = document.createElement('div');
      root.className = 's-sky cls-root';
      root.setAttribute('role', 'dialog'); root.setAttribute('aria-modal', 'true'); root.setAttribute('aria-labelledby', 'cls-title');
      root.setAttribute('tabindex', '-1');                     // the dialog itself takes focus; Tab moves into it
      root.innerHTML = String(stageMarkup());
      document.body.appendChild(root);
    } else {
      $('#cls-status', root).innerHTML = String(statusMarkup());
      root.classList.remove('is-leaving');
      root.classList.remove('is-closed'); root.removeAttribute('inert');
    }
    document.documentElement.setAttribute('data-classic', 'on');
    $('#app').setAttribute('inert', ''); $('#app').setAttribute('aria-hidden', 'true');
    requestAnimationFrame(function () { requestAnimationFrame(function () { if (open) root.classList.add('is-on'); }); });
    later(function () { if (open) root.focus({ preventScroll: true }); }, 60);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    startWebamp();
  }
  function closeStage() {
    visit += 1;                                                // a Webamp start still in flight won't hand over now
    open = false;
    if (webamp) {
      App.hooks.media = null;                                  // back to the app engine
      session.stop();
      if (handedOver) { handedOver = false; takeBack(session.handBack()); }
      closingByUs = true; webamp.close(); closingByUs = false;  // close() asks onWillClose too: let it through
      webampClosed = true;
    }
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('resize', onResize);
    timers.forEach(clearTimeout); timers = [];
    if (S.skin && S.skin.state === 'failed') S.skin = null;    // said once; the next visit tries again
    sizeSaid = null;
    App.closePopover();
    /* hidden, not removed (Webamp's iframe would reload) and not display: none (its size, so Webamp's layout, stays) */
    root.classList.remove('is-on');
    root.classList.add('is-closed'); root.setAttribute('inert', '');
    document.documentElement.removeAttribute('data-classic');
    $('#app').removeAttribute('inert'); $('#app').removeAttribute('aria-hidden');
  }

  /* ---------- the desktop app: Webamp in its own window (app/src/desktop/classic.rs) ---------- */
  /* desktop.js loads after this file: ask when Classic opens */
  function desktopApp() { var D = window.LullDesktop; return D && D.inWrapper && D.openClassic ? D : null; }
  var windowListen = null, hearing = false, windowWasPlaying = false;
  /* from the start, not only once this page opened the window: a page that reloaded while Classic mode played
     (its process crashed, app/src/desktop/recovery.rs) still takes the queue back */
  App.on('ready', function () {                                // desktop.js loads after this file
    var D = desktopApp();
    if (D && !hearing) { hearing = true; D.onClassic(fromWindow); }
    /* the native Classic window closed this page while it played, and hands its queue to the new one */
    if (D && D.classicBack) D.classicBack().then(function (b) { if (b) takeBack(b); }, function (err) { console.warn('Classic hand-back', err); });
  });
  /* the hand-off plus the skin to start in (later choices reach the window by the storage event) and the favourites
     for Webamp's Skins menu */
  function openWindow(D) {
    if (!hearing) { hearing = true; D.onClassic(fromWindow); }
    var h = handoff();
    /* a window drawn natively (no webview) takes its size and equaliser here, and lull comes back on this page; a skin
       kept on this device comes along as its file, which only this page's storage holds */
    h.size = App.classicSize.get(); h.page = lastHref(); h.eq = LullEq.read(); h.layout = readLayout();
    h.repeatMode = P.state.repeat; h.radio = P.state.radio; h.context = P.state.context;
    return Promise.all([readFavourites(), App.classicSkin.opening()]).then(function (r) {
      var favs = r[0], skin = r[1];
      return keptFile(skin).then(function (bytes) {
        if (bytes) h.skinData = base64(bytes);
        h.skin = skin ? { url: LullWebamp.skinUrl(base(), skin), name: skin.name || 'Your skin' } : null;
        h.skins = favs.map(function (s) { return { url: absolute(skinFileUrl(s.md5)), name: s.name }; });
        return D.openClassic(h);
      });
    }).then(function () {                                       // the window starts Webamp where the app was
      windowWasPlaying = h.playing;
      if (h.playing) P.pause();
    });
  }
  /* a skin kept on this device (`local-` md5) as its file's bytes; null for any other */
  function keptFile(skin) {
    if (!skin || !/^local-/.test(skin.md5)) return Promise.resolve(null);
    return LullWebamp.localSkins().then(function (all) {
      var e = all.filter(function (x) { return x.md5 === skin.md5; })[0];
      return e ? e.blob.arrayBuffer() : null;
    });
  }
  function base64(buffer) {
    var bytes = new Uint8Array(buffer), out = '';
    for (var i = 0; i < bytes.length; i += 0x8000) out += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(out);
  }
  /* what Webamp's window reports (desktop.js passes on its 'classic' events) */
  function fromWindow(m) {
    if (m.type === 'listen') windowListen = App.recordPlay(m.id, m.duration);
    else if (m.type === 'listened' && windowListen) {
      windowListen.listened = m.seconds; windowListen.end(m.how); windowListen = null;
    } else if (m.type === 'back') {
      if (m.back) takeBack(m.back);
      else if (windowWasPlaying) P.play();                      // Webamp never started: carry on here
    }
    else if (m.type === 'failed') App.toast('Webamp didn\'t start: ' + m.message, { kind: 'error', icon: 'warning' });
  }

  /* ---------- route ---------- */
  App.route('/classic', {
    title: 'Classic mode',
    render: function (ctx) {
      if (!open) restore = { scroll: ctx.el.scrollTop, opener: document.activeElement };
      return '';
    },
    mount: function (ctx) {
      var D = desktopApp();
      if (D && !open) {
        /* Classic lives in its own window: back to the page it came from. An app without that window (older than
           this page) refuses, and the stage opens here instead */
        openWindow(D).then(function () { if (onClassic()) App.go(lastHref()); }, function (err) {
          console.warn('Classic window unavailable, using the stage', err);
          if (onClassic() && !open) openStage();
        });
      } else if (!open) openStage();
      /* a re-render of /classic keeps the stage; leaving closes it */
      ctx.onCleanup(function () { if (!onClassic() && open) closeStage(); });
    }
  });
})();
