/* desktop/winamp.js: the desktop app's Classic window (app/src/desktop/classic.rs, docs/context/desktop.md). Webamp
   alone in a borderless, transparent window that fits its windows and moves by their title bars, like Winamp itself.
   The app's own window hides meanwhile; its page (screens/classic.js) built the hand-off and takes the queue back.
   - Start: invoke('classic_handoff') -> the queue, skin and favourites; Webamp starts where the app was.
   - To `main`, as 'classic' events: { type: 'now', id, playing, pos, duration } (song, play/pause, seeks: the tray,
     taskbar buttons and Discord follow it), 'listen' / 'listened' (its listening history), 'action' (Like and
     Save from the tray, which only the app can do), 'back' with the queue when Webamp closes, 'failed'.
   - From the app: 'tray' (tray and taskbar buttons) and 'classic-close' (Alt+F4, the tray's Show lull).
   - The system's media overlay and media keys (Media Session) follow Webamp.
   - Skins load through LullWebamp.loadSkin, and Webamp's alerts show as a note in this window (#note), never as a
     browser dialog. */
(function () {
  'use strict';
  var T = window.__TAURI__, win = T.window.getCurrentWindow(), LogicalSize = (T.dpi || T.window).LogicalSize;
  var stage = document.getElementById('stage'), note = document.getElementById('note');
  var webamp = null, session = null, finished = false;
  var WINDOWS = ['#main-window', '#equalizer-window', '#playlist-window'];

  function toMain(message) { return T.event.emitTo('main', 'classic', message); }
  function logFailure(what) { return function (err) { console.error('lull classic: ' + what + ' failed', err); }; }

  T.core.invoke('classic_handoff').then(function (h) {
    if (!h) throw new Error('nothing was handed over (the window reloaded?)');
    return start(h);
  }).catch(fail);

  function start(h) {
    if (typeof window.Webamp !== 'function') throw new Error('the player file didn\'t load');
    LullWebamp.quietAlerts(say);
    webamp = new window.Webamp({
      availableSkins: h.skins.map(function (s) { return { url: s.url, name: s.name }; }),
      windowLayout: {
        main: { position: { top: 0, left: 0 } },
        equalizer: { position: { top: 116, left: 0 } },
        playlist: { position: { top: 232, left: 0 }, size: { extraWidth: 0, extraHeight: 0 } }
      },
      enableHotkeys: true,                                      // only Webamp is here: its keys are the window's
      enableMediaSession: false                                 // set below, with lull's covers
    });
    webamp.onWillClose(function (cancel) { cancel(); finish(); });   // its close button: back to lull
    webamp.onMinimize(function () { win.minimize().catch(logFailure('minimize')); });
    session = LullWebamp.session(webamp, { listen: listen, change: report });
    return webamp.renderInto(stage).then(function () {
      session.handIn(h);
      wireMediaSession();
      /* the window shows once the listener's skin is on, so Webamp's base skin never flashes */
      return h.skin && LullWebamp.loadSkin(webamp, h.skin.url).catch(function (err) {
        say('Couldn\'t load ' + h.skin.name + ': ' + err.message);
      });
    }).then(function () {
      fit();
      new MutationObserver(scheduleFit).observe(stage, { subtree: true, childList: true, attributes: true, attributeFilter: ['style', 'class'] });
      return win.show().then(function () { return win.setFocus(); });
    });
  }

  /* ---------- the window is exactly Webamp's windows: they stay stacked, and the window moves instead ---------- */
  var fitting = false, size = '';
  function scheduleFit() { if (!fitting) { fitting = true; requestAnimationFrame(fit); } }
  function fit() {
    fitting = false;
    var right = 0, bottom = 0;
    WINDOWS.forEach(function (sel) {
      var el = stage.querySelector(sel); if (!el) return;
      var r = el.getBoundingClientRect(); if (!r.width) return;
      right = Math.max(right, r.right); bottom = Math.max(bottom, r.bottom);
    });
    var next = Math.ceil(right) + 'x' + Math.ceil(bottom);
    if (!right || next === size) return;
    size = next;
    win.setSize(new LogicalSize(Math.ceil(right), Math.ceil(bottom))).catch(logFailure('fit'));
  }
  /* a press on a title bar (Webamp's .draggable, not one of its buttons, which carry ids) drags the window */
  document.addEventListener('mousedown', function (e) {
    if (e.button !== 0) return;
    var bar = e.target.closest && e.target.closest('.draggable');
    if (!bar) return;
    for (var el = e.target; el && el !== bar; el = el.parentElement) if (el.id) return;
    e.preventDefault(); e.stopPropagation();
    win.startDragging().catch(logFailure('drag'));
  }, true);

  /* a sentence over Webamp for a few seconds (a click dismisses it); the window is only as big as Webamp */
  var noteTimer = 0;
  function say(message) {
    note.textContent = message;
    note.hidden = false;
    clearTimeout(noteTimer);
    noteTimer = setTimeout(function () { note.hidden = true; }, 8000);
  }
  note.addEventListener('click', function () { note.hidden = true; });

  /* ---------- what plays, for the app and the system ---------- */
  function report() {
    var n = session.now(), t = n.track;
    toMain({ type: 'now', id: t ? t.id : null, playing: n.playing, pos: n.pos, duration: n.duration || (t && t.duration) || 0 })
      .catch(logFailure('now playing'));
    if (!('mediaSession' in navigator)) return;
    var ms = navigator.mediaSession;
    try {
      if (!t) ms.metadata = null;
      else if (!ms.metadata || ms.metadata.title !== t.title || ms.metadata.artist !== t.artist) {
        ms.metadata = new MediaMetadata({ title: t.title, artist: t.artist, album: t.album,
          artwork: t.art ? [{ src: t.art, sizes: '256x256' }] : [] });
      }
      ms.playbackState = !t ? 'none' : n.playing ? 'playing' : 'paused';
      if (t && n.duration && ms.setPositionState) ms.setPositionState({ duration: n.duration, playbackRate: 1, position: Math.min(n.pos, n.duration) });
    } catch (err) { console.warn('Media Session rejected an update', err); }
  }
  function wireMediaSession() {
    if (!('mediaSession' in navigator)) return;
    var h = {
      play: function () { webamp.play(); }, pause: function () { webamp.pause(); }, stop: function () { webamp.stop(); },
      previoustrack: function () { webamp.previousTrack(); }, nexttrack: function () { webamp.nextTrack(); },
      seekto: function (d) { webamp.seekToTime(d.seekTime); }
    };
    Object.keys(h).forEach(function (k) { try { navigator.mediaSession.setActionHandler(k, h[k]); } catch (e) { /* unsupported action */ } });
  }

  /* listening records live in the app (App.recordPlay); this window says when one starts and how it ended */
  function listen(id, duration) {
    toMain({ type: 'listen', id: id, duration: duration || null }).catch(logFailure('listen'));
    var rec = { listened: 0, end: function (how) {
      toMain({ type: 'listened', how: how, seconds: rec.listened }).catch(logFailure('listened'));
    } };
    return rec;
  }

  /* the tray and the taskbar buttons */
  T.event.listen('tray', function (e) {
    if (!webamp) return;
    var a = e.payload.action, t = session.now().track;
    if (a === 'play-pause') { if (webamp.getMediaStatus() === 'PLAYING') webamp.pause(); else webamp.play(); }
    else if (a === 'next') webamp.nextTrack();
    else if (a === 'previous') webamp.previousTrack();
    else if ((a === 'like' || a === 'save') && t) toMain({ type: 'action', action: a, id: t.id }).catch(logFailure(a));
  });
  T.event.listen('classic-close', finish);

  /* ---------- back to lull: the queue goes to the app, then the app closes this window ---------- */
  function finish() {
    if (finished) return;
    finished = true;
    var back = null;
    if (session) { session.stop(); back = session.handBack(); webamp.pause(); }
    toMain({ type: 'back', back: back }).catch(logFailure('hand back')).then(function () {
      return T.core.invoke('window_action', { request: { action: 'classic-done' } });
    }).catch(logFailure('close'));
  }
  function fail(err) {
    var message = err && err.message || String(err);
    console.error('lull classic: Webamp didn\'t start', err);
    toMain({ type: 'failed', message: message }).catch(logFailure('report'));
    finish();
  }
})();
