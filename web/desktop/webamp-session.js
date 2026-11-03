/* desktop/webamp-session.js: one Webamp playing lull's queue, and the listener's Classic skin. Shared by Classic
   mode in the web UI (screens/classic.js, loaded at boot for screens/skins.js too) and by the desktop app's Classic
   window (desktop/winamp.js), which has no App: everything it needs comes in the hand-off.
     handoff = { tracks: [{ id, url, title, artist, album, duration, art }], index, pos, playing, shuffle,
                 repeat (Webamp's is on or off), volume (0..1) }, URLs absolute
   LullWebamp.session(webamp, hooks) -> session
     hooks.listen(id, duration) -> { listened, end(how) }   a listening record, App.recordPlay's shape
     hooks.change()                                        the song, play/pause or position jumped
     session.handIn(handoff)    the queue into Webamp, at the same song and second, playing if it was
     session.handBack()         { ids, index, pos, playing, shuffle, repeat, local }: Webamp's list as lull
                                ids, its song and modes; `local` counts files it played that aren't lull's
     session.add(track)         one more lull song as a Webamp track (songs dropped on Webamp)
     session.idOf(url)          the lull id of a Webamp track's URL, or null (a local file)
     session.now()              { track, playing, pos, duration }: the hand-off's track or null
     session.stop()             the listening record ends as 'stopped' (leaving Classic)
   Listening: a record begins once a lull song plays and gains the seconds heard in steady playback
   (seeks don't count), as the app engine counts them. Webamp has no play/pause or seek events, so its
   store and media clock are watched instead.

   Skins: Webamp reads a skin with fetch(url) and reports a failure with window.alert(). lull never lets it:
     LullWebamp.quietAlerts(onAlert)   Webamp's alerts come here instead of the browser's dialog; during a
                                       loadSkin they fail that load, any other goes to onAlert(message)
     LullWebamp.loadSkin(webamp, src)  src: a skin URL (below), fetched here first (the hub's sentence on a
                                       failure), a Blob, or null for Webamp's base skin. Webamp gets the bytes
                                       as a blob: URL; resolves once it shows them, rejects with a sentence.
     LullWebamp.skinFile(url)          the .wsz as a Blob, or a rejection with the sentence
   The listener's Classic skin is per device: localStorage SKIN_KEY = { md5, name, screenshotUrl } or null
   (Webamp's base skin), chosen in Settings > Appearance (screens/skins.js). Both Classic players apply it when
   they open, and follow its storage event while open (a change made in another tab or window).
     LullWebamp.skinUrl(base, skin)    its file: <base>api/skins/<md5>.wsz for a museum skin, or
                                       <base>local-skins/<md5>.wsz for a .wsz kept on this device (md5
                                       'local-…'), a URL on the hub's origin that nothing serves: skinFile
                                       reads it from IndexedDB 'lull-skins' (store 'files', by md5)
     LullWebamp.localSkins()           [{ md5, name, file, blob, added }] kept on this device, newest first
     LullWebamp.keepLocal(entry), dropLocal(md5) */
(function () {
  'use strict';
  var SKIN_TIMEOUT_MS = 10000;
  var SKIN_KEY = 'lull:classicSkin';
  var LOCAL = /\/local-skins\/(local-[0-9a-z]+)\.wsz$/;
  function absolute(url) { return new URL(url, location.href).href; }

  function skinUrl(base, skin) {
    if (!skin) return null;
    var local = /^local-/.test(skin.md5);
    return new URL((local ? 'local-skins/' : 'api/skins/') + encodeURIComponent(skin.md5) + '.wsz', base).href;
  }
  /* .wsz files from this device, in IndexedDB (shared by every lull page and window of this browser or app) */
  function files(mode, use) {
    return new Promise(function (resolve, reject) {
      var req = indexedDB.open('lull-skins', 1);
      req.onupgradeneeded = function () { req.result.createObjectStore('files', { keyPath: 'md5' }); };
      req.onerror = function () { reject(req.error); };
      req.onsuccess = function () {
        var db = req.result, tx = db.transaction('files', mode), op = use(tx.objectStore('files'));
        tx.oncomplete = function () { db.close(); resolve(op.result); };
        tx.onerror = tx.onabort = function () { db.close(); reject(tx.error); };
      };
    });
  }
  function localSkins() {
    return files('readonly', function (s) { return s.getAll(); }).then(function (all) {
      return all.sort(function (a, b) { return b.added - a.added; });
    });
  }
  function keepLocal(entry) { return files('readwrite', function (s) { return s.put(entry); }); }
  function dropLocal(md5) { return files('readwrite', function (s) { return s.delete(md5); }); }

  var failSkin = null;                                            // the loadSkin in progress, if any
  function quietAlerts(onAlert) {
    window.alert = function (message) {
      message = String(message);
      if (failSkin && /skin/i.test(message)) failSkin(message); else onAlert(message);
    };
  }
  function skinFile(src) {
    var local = LOCAL.exec(src);
    if (local) {
      return files('readonly', function (s) { return s.get(local[1]); }).then(function (entry) {
        if (!entry) throw new Error('the file isn\'t kept on this device any more.');
        return entry.blob;
      }, function () { throw new Error('this device\'s skin files can\'t be read.'); });
    }
    /* the hub serves .wsz files as immutable (named by md5), so any cached copy will do, also offline */
    return fetch(src, { credentials: 'same-origin', cache: 'force-cache' }).then(function (res) {
      if (res.ok) return res.blob();
      return res.json().then(function (e) { return e && e.message; }, function () { return null; }).then(function (msg) {
        var err = new Error(msg || 'the lull hub answered ' + res.status + '.');
        err.status = res.status;
        throw err;
      });
    }, function () { throw new Error('can\'t reach the lull hub.'); });
  }
  /* one load at a time: callers wait for the last one before starting the next */
  function loadSkin(webamp, src) {
    /* Webamp has no public call for its base skin: this is what its own Skins > <Base Skin> sends (2.3.1) */
    if (!src) { webamp.store.dispatch({ type: 'LOAD_DEFAULT_SKIN' }); return Promise.resolve(); }
    return (src instanceof Blob ? Promise.resolve(src) : skinFile(src)).then(function (blob) {
      var url = URL.createObjectURL(blob);
      return new Promise(function (resolve, reject) {
        var settled = false, timer = setTimeout(function () {
          done(new Error('Webamp didn\'t finish loading it within 10 seconds.'));
        }, SKIN_TIMEOUT_MS);
        function done(err) {
          if (settled) return;
          settled = true; failSkin = null; clearTimeout(timer);
          URL.revokeObjectURL(url);                               // Webamp has read it by now, or never will
          if (err) reject(err); else resolve();
        }
        failSkin = function (said) {
          done(new Error(/download/i.test(said) ? 'Webamp couldn\'t open the file lull handed it.'
            : 'Webamp couldn\'t read it, so it may not be a classic Winamp skin.'));
        };
        webamp.setSkinFromUrl(url);
        webamp.skinIsLoaded().then(function () { done(null); }, done);
      });
    });
  }

  function session(webamp, hooks) {
    var byUrl = {}, currentUrl = null, pendingSeek = null, handedOver = false, listen = null;
    var lastStatus = null, lastTime = 0;

    function add(t) {
      byUrl[absolute(t.url)] = t;
      return { url: t.url, metaData: { title: t.title, artist: t.artist, album: t.album }, duration: t.duration || undefined };
    }
    function handIn(h) {
      byUrl = {};
      var tracks = h.tracks.map(add);
      webamp.setTracksToPlay([]);                                 // empties Webamp's list without starting anything
      webamp.appendTracks(tracks);
      if (webamp.isShuffleEnabled() !== !!h.shuffle) webamp.toggleShuffle();
      if (webamp.isRepeatEnabled() !== !!h.repeat) webamp.toggleRepeat();
      webamp.setVolume(Math.round(h.volume * 100));
      currentUrl = null;
      if (tracks.length) {
        var index = Math.min(h.index, tracks.length - 1);
        currentUrl = absolute(h.tracks[index].url);              // Webamp says so only once it plays
        pendingSeek = h.pos > 1 ? h.pos : null;
        webamp.setCurrentTrack(index);
        if (h.playing) webamp.play();
      }
      handedOver = true;
    }
    function handBack() {
      var playing = webamp.getMediaStatus() === 'PLAYING';
      var pos = pendingSeek !== null ? pendingSeek : webamp.media.timeElapsed();   // never played: where it was handed in
      handedOver = false; pendingSeek = null;
      var ids = [], index = -1, local = 0;
      webamp.getPlaylistTracks().forEach(function (wt) {
        var url = absolute(wt.url), t = byUrl[url];
        if (!t) { local += 1; return; }
        if (index < 0 && url === currentUrl) index = ids.length;
        ids.push(t.id);
      });
      if (index < 0) { index = 0; pos = 0; playing = false; }     // Webamp was on a local file
      return { ids: ids, index: index, pos: pos, playing: playing, shuffle: webamp.isShuffleEnabled(),
        repeat: webamp.isRepeatEnabled(), local: local };
    }
    function now() {
      return { track: currentUrl && byUrl[currentUrl] || null, playing: webamp.getMediaStatus() === 'PLAYING',
        pos: webamp.media.timeElapsed(), duration: webamp.media.duration() || 0 };
    }

    function nearEnd(l) { return l.dur > 0 && l.last >= l.dur - 1.5; }
    function endListen(how) { if (listen) { listen.rec.end(how); listen = null; } }
    webamp.onTrackDidChange(function (wt) {
      currentUrl = wt ? absolute(wt.url) : null;
      if (listen) endListen(nearEnd(listen) ? 'completed' : 'skipped');
      if (hooks.change) hooks.change();
    });
    /* Webamp's media clock: several times a second while playing, and on seeks */
    webamp.media.on('timeupdate', function () {
      var playing = webamp.getMediaStatus() === 'PLAYING', t = webamp.media.timeElapsed();
      var jumped = Math.abs(t - lastTime) > 2;
      lastTime = t;
      if (jumped && hooks.change) hooks.change();
      if (!handedOver) return;
      if (listen && t < listen.last - 1 && nearEnd(listen)) endListen('completed');   // the same song again (repeat)
      if (!listen) {
        var song = currentUrl && byUrl[currentUrl];
        if (!playing || !song) return;
        listen = { rec: hooks.listen(song.id, webamp.media.duration() || song.duration), last: t, dur: 0 };
      }
      var dt = t - listen.last;
      if (playing && dt > 0 && dt < 1.5) listen.rec.listened += dt;
      listen.last = t; listen.dur = webamp.media.duration();
    });
    /* Webamp can seek only once the file's duration is known */
    webamp.media.on('fileLoaded', function () {
      if (pendingSeek !== null) { var at = pendingSeek; pendingSeek = null; webamp.seekToTime(at); }
    });
    webamp.store.subscribe(function () {
      var status = webamp.getMediaStatus();
      if (status === lastStatus) return;
      lastStatus = status;
      if (hooks.change) hooks.change();
    });

    return { handIn: handIn, handBack: handBack, add: add, now: now, stop: function () { endListen('stopped'); },
      idOf: function (url) { var t = byUrl[absolute(url)]; return t ? t.id : null; } };
  }

  window.LullWebamp = { session: session, quietAlerts: quietAlerts, loadSkin: loadSkin, skinFile: skinFile,
    SKIN_KEY: SKIN_KEY, skinUrl: skinUrl, localSkins: localSkins, keepLocal: keepLocal, dropLocal: dropLocal };
})();
