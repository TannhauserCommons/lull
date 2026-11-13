/* Offline (ADR-0005): the outbox and downloads.
   - App.outbox: likes, follows, plays and playlist edits made while the hub can't be reached wait in IndexedDB,
     each with a client-made id, and go to POST /api/sync in order once it answers again (idempotent by id).
   - App.downloads: songs kept on this device, with their cover and track JSON, behind one interface with two
     backends: the browser's (Cache Storage, made persistent, and an IndexedDB index) and the Android app's
     window.LullNative.downloads (docs/context/web.md#offline-downloads-and-outbox). A downloaded song plays from a
     Blob URL or the app's file (localUrl). Albums, playlists and Liked are kept as collections and follow their
     source while online.
   Events: 'downloads' (the index or progress changed), 'outbox' (the queue length changed). */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, $$ = App.$$;

  /* ---------- IndexedDB: one database, three stores ---------- */
  var dbp = null;
  function db() {
    if (dbp) return dbp;
    dbp = new Promise(function (resolve, reject) {
      if (!window.indexedDB) { reject(new Error('This browser has no IndexedDB')); return; }
      var req = indexedDB.open('lull', 1);
      req.onupgradeneeded = function () {
        var d = req.result;
        d.createObjectStore('outbox', { keyPath: 'seq', autoIncrement: true });
        d.createObjectStore('downloads', { keyPath: 'trackId' });
        d.createObjectStore('kept', { keyPath: 'key' });
      };
      req.onsuccess = function () { resolve(req.result); };
      req.onerror = function () { reject(req.error); };
    });
    return dbp;
  }
  /* run fn(store) in one transaction; resolves to what fn's request answered */
  function tx(store, mode, fn) {
    return db().then(function (d) {
      return new Promise(function (resolve, reject) {
        var t = d.transaction(store, mode), req = fn(t.objectStore(store));
        t.oncomplete = function () { resolve(req && req.result); };
        t.onerror = function () { reject(t.error); };
      });
    });
  }
  function all(store) { return tx(store, 'readonly', function (s) { return s.getAll(); }); }
  function uuid() { return crypto.randomUUID ? crypto.randomUUID() : 'op-' + Date.now().toString(36) + Math.random().toString(36).slice(2); }
  function now() { return Math.floor(Date.now() / 1000); }

  /* =====================================================================
     1. Outbox
     ===================================================================== */
  var OB = App.outbox = {}, waiting = 0, flushing = null, told = false;
  /* the hub is out of reach: no answer last time, or the browser says it's offline */
  OB.offline = function () { return App.server.reachable === false || navigator.onLine === false; };
  function count() { return all('outbox').then(function (ops) { waiting = ops.length; App.emit('outbox', waiting); return ops; }, function () { return []; }); }
  OB.size = function () { return waiting; };
  /* queue one op ({ op, ...fields }); the id and time are added here */
  OB.add = function (op) {
    var rec = Object.assign({ id: uuid(), at: now() }, op);
    if (!told) { told = true; App.toast('Kept on this device. It reaches your hub when it can.', { confirms: true, icon: 'offline' }); }
    return tx('outbox', 'readwrite', function (s) { return s.add(rec); }).then(count).then(function () { return { queued: true, id: rec.id }; });
  };
  /* a change for the hub: sent now when it can be, else queued. Resolves to the server's answer, or
     { queued: true } when it waits in the outbox; rejects only when the hub answered with a refusal. */
  OB.call = function (path, opts, op) {
    if (OB.offline()) return OB.add(op);
    return M.api(path, opts).then(null, function (err) { if (err.status === 0) return OB.add(op); throw err; });
  };
  /* send everything waiting, in order, in batches; ops the hub refuses are dropped and named */
  OB.flush = function () {
    if (flushing) return flushing;
    flushing = all('outbox').then(function (ops) {
      if (!ops.length) return 0;
      return sendBatches(ops, 0, { applied: 0, rejected: [], playlists: {} }).then(function (res) {
        /* a playlist made offline has its real id now: a page showing it follows */
        var m = /^\/playlist\/(pl-tmp-[^/]+)$/.exec(App.current.path);
        if (m && res.playlists[m[1]]) App.go('/playlist/' + res.playlists[m[1]], { replace: true });
        if (res.rejected.length) App.toast(res.rejected.length === 1 ? 'One change made offline was refused: ' + res.rejected[0]
          : res.rejected.length + ' changes made offline were refused. The first: ' + res.rejected[0], { kind: 'error' });
        told = false;
        if (res.applied) { App.toast('Changes made offline reached your hub', { kind: 'ok', icon: 'check' }); App.reloadCollections(); M.invalidate(['/api/history', '/api/stats', '/api/likes', '/api/playlists']); App.refresh(); }
        return res.applied;
      });
    }).then(function (n) { flushing = null; count(); return n; }, function (err) { flushing = null; count(); throw err; });
    return flushing;
  };
  function sendBatches(ops, from, res) {
    var batch = ops.slice(from, from + 200);
    if (!batch.length) return Promise.resolve(res);
    var body = { ops: batch.map(function (o) { var c = Object.assign({}, o); delete c.seq; return c; }) };
    return M.api('/api/sync', { method: 'POST', body: body }).then(function (r) {
      var done = (r.applied || []).concat((r.rejected || []).map(function (x) { return x.id; }));
      res.applied += (r.applied || []).length;
      (r.rejected || []).forEach(function (x) { res.rejected.push(x.message); });
      Object.assign(res.playlists, r.playlists || {});
      var seqs = batch.filter(function (o) { return done.indexOf(o.id) >= 0; }).map(function (o) { return o.seq; });
      return tx('outbox', 'readwrite', function (s) { seqs.forEach(function (q) { s.delete(q); }); return null; });
    }).then(function () { return sendBatches(ops, from + 200, res); });
  }
  function tryFlush() { if (!OB.offline() && App.server.authorized && App.server.checked) OB.flush().catch(function () {}); }
  App.on('server', function (srv) { if (srv.reachable) tryFlush(); });
  App.on('server:resync', tryFlush);
  window.addEventListener('online', tryFlush);
  count();

  /* =====================================================================
     2. Downloads
     ===================================================================== */
  var D = App.downloads = {};
  var native = window.LullNative && window.LullNative.downloads;
  var index = {};          // trackId -> { trackId, state: downloading|done|failed, sizeBytes, track, album, error, received, size }
  var kept = {};           // key ("album:al-…", "playlist:pl-…", "liked", "track:t-…") -> { key, kind, id, ids }
  var ready = null;
  function changed() { App.emit('downloads'); syncMarks(); }

  D.has = function (id) { var r = index[id]; return !!(r && r.state === 'done'); };
  D.state = function (id) { return index[id] || null; };
  D.list = function () { return Object.keys(index).map(function (k) { return index[k]; }); };
  D.kept = function () { return Object.keys(kept).map(function (k) { return kept[k]; }); };
  D.keptFor = function (key) { return kept[key] || null; };
  D.usedBytes = function () { return D.list().reduce(function (n, r) { return n + (r.state === 'done' ? r.sizeBytes || 0 : 0); }, 0); };
  /* an id set's state for its Download button: none | some (downloading or partly) | all */
  D.coverage = function (ids) {
    var done = ids.filter(D.has).length, busy = ids.filter(function (id) { return index[id] && index[id].state === 'downloading'; }).length;
    return { done: done, busy: busy, total: ids.length, state: done && done === ids.length ? 'all' : done || busy ? 'some' : 'none' };
  };
  D.available = function () { return !!native || !!(window.caches && window.indexedDB && window.isSecureContext); };
  /* why downloads can't work here, as a sentence, or null */
  D.unavailableReason = function () {
    if (D.available()) return null;
    return window.isSecureContext ? 'This browser can\'t keep songs for offline.' : 'Downloads need HTTPS. Open your hub\'s https address (tailscale serve) to keep songs on this device.';
  };

  /* ---------- the browser backend: Cache Storage holds audio and covers, IndexedDB the index ---------- */
  var CACHE = 'lull-downloads', blobs = {};
  function audioKey(id) { return new URL('downloads/audio/' + encodeURIComponent(id), location.href).href; }
  function streamFor(t) { return App.isSaved(t.id) ? M.url('/api/library/stream/' + encodeURIComponent(t.id)) : M.url('/api/stream/' + encodeURIComponent(t.id)); }
  var browser = {
    load: function () { return Promise.all([all('downloads'), all('kept')]); },
    /* fetch the audio with progress, then the covers the UI asks for */
    fetch: function (rec, onProgress) {
      var t = rec.track;
      return fetch(streamFor(t), { credentials: 'same-origin' }).then(function (res) {
        if (!res.ok) return res.json().then(function (b) { throw new Error(b.message); }, function () { throw new Error('The hub answered ' + res.status); });
        var size = +res.headers.get('Content-Length') || 0, type = res.headers.get('Content-Type') || 'audio/mpeg', got = 0, chunks = [];
        var reader = res.body.getReader();
        function pump() {
          return reader.read().then(function (r) {
            if (r.done) return new Blob(chunks, { type: type });
            chunks.push(r.value); got += r.value.length; onProgress(got, size);
            return pump();
          });
        }
        return pump();
      }, function () { throw new Error('Can\'t reach your hub'); }).then(function (blob) {
        return caches.open(CACHE).then(function (c) {
          var covers = t.albumId ? [96, 256, 512].map(function (s) {
            var u = App.artUrl(t.albumId, s);
            return fetch(u, { credentials: 'same-origin' }).then(function (r) { if (r.ok) return c.put(u, r); }).catch(function () {});
          }) : [];
          return Promise.all([c.put(audioKey(t.id), new Response(blob, { headers: { 'Content-Type': blob.type, 'Content-Length': String(blob.size) } }))].concat(covers));
        }).then(function () { return blob.size; });
      });
    },
    drop: function (ids, albumsGone) {
      return caches.open(CACHE).then(function (c) {
        return Promise.all(ids.map(function (id) { if (blobs[id]) { URL.revokeObjectURL(blobs[id]); delete blobs[id]; } return c.delete(audioKey(id)); })
          .concat([].concat.apply([], albumsGone.map(function (al) { return [96, 256, 512].map(function (s) { return c.delete(App.artUrl(al, s)); }); }))));
      });
    },
    localUrl: function (id) {
      if (blobs[id]) return Promise.resolve(blobs[id]);
      return caches.open(CACHE).then(function (c) { return c.match(audioKey(id)); }).then(function (res) {
        return res ? res.blob().then(function (b) { return (blobs[id] = URL.createObjectURL(b)); }) : null;
      });
    },
    estimate: function () {
      return navigator.storage && navigator.storage.estimate ? navigator.storage.estimate().then(function (e) { return { usedBytes: e.usage, freeBytes: e.quota - e.usage }; }) : Promise.resolve(null);
    }
  };

  /* ---------- the app backend: window.LullNative.downloads keeps files in app storage ---------- */
  var app = native && {
    load: function () {
      return Promise.all([Promise.resolve(native.list()), all('kept').catch(function () { return []; })]).then(function (r) {
        return [r[0].map(function (x) { return { trackId: x.trackId, state: x.state, sizeBytes: x.sizeBytes, track: x.meta && x.meta.track, album: x.meta && x.meta.album, error: x.error, fileUrl: x.fileUrl }; }), r[1]];
      });
    },
    fetch: function (rec, onProgress) {
      var t = rec.track;
      return new Promise(function (resolve, reject) {
        var offs = [native.on('progress', function (e) { if (e.trackId === t.id) onProgress(e.received, e.size); }),
          native.on('done', function (e) { if (e.trackId === t.id) { offs.forEach(function (f) { f(); }); rec.fileUrl = e.fileUrl; resolve(e.sizeBytes); } }),
          native.on('error', function (e) { if (e.trackId === t.id) { offs.forEach(function (f) { f(); }); reject(new Error(e.message)); } })];
        Promise.resolve(native.add({ trackId: t.id, url: new URL(streamFor(t), location.href).href,
          artwork: t.albumId ? new URL(App.artUrl(t.albumId, 512), location.href).href : null, meta: { track: rec.track, album: rec.album } }))
          .catch(function (err) { offs.forEach(function (f) { f(); }); reject(err); });
      });
    },
    drop: function (ids) { return Promise.resolve(native.remove(ids)); },
    localUrl: function (id) { return Promise.resolve(index[id] && index[id].fileUrl || null); },
    estimate: function () { return Promise.resolve(native.usage()); }
  };
  var backend = app || browser;

  /* the index survives in IndexedDB (browser) or the app (native); downloaded tracks resolve offline */
  ready = backend.load().then(function (r) {
    r[0].forEach(function (rec) { index[rec.trackId] = rec.state === 'downloading' ? Object.assign(rec, { state: 'failed', error: 'Interrupted' }) : rec; });
    r[1].forEach(function (k) { kept[k.key] = k; });
    var tracks = {}, albums = {};
    D.list().forEach(function (rec) { if (rec.track) tracks[rec.trackId] = rec.track; if (rec.album) albums[rec.album.id] = rec.album; });
    M.absorb({ tracks: tracks, albums: albums });
    changed();
  }, function (err) { console.warn('Downloads unavailable:', err.message); });

  D.localUrl = function (id) {
    return (ready || Promise.resolve()).then(function () { return D.has(id) ? backend.localUrl(id) : null; }).catch(function () { return null; });
  };
  function saveRec(rec) {
    if (app) return Promise.resolve();
    var copy = Object.assign({}, rec); delete copy.received; delete copy.size;
    return tx('downloads', 'readwrite', function (s) { return s.put(copy); });
  }

  /* ---------- the download queue: two at a time ---------- */
  var queue = [], active = 0, persisted = false, progressAt = 0;
  function enqueue(ids) {
    ids.forEach(function (id) {
      var t = M.track(id); if (!t || (index[id] && (index[id].state === 'done' || index[id].state === 'downloading'))) return;
      index[id] = { trackId: id, state: 'downloading', sizeBytes: 0, track: t, album: M.album(t.albumId) || null, received: 0, size: t.sizeBytes || 0 };
      queue.push(id);
    });
    changed(); pump();
  }
  function pump() {
    while (active < 2 && queue.length) run(queue.shift());
  }
  function run(id) {
    var rec = index[id]; if (!rec || rec.state !== 'downloading') { pump(); return; }
    active++;
    backend.fetch(rec, function (got, size) {
      rec.received = got; rec.size = size || rec.size;
      if (Date.now() - progressAt > 400) { progressAt = Date.now(); App.emit('downloads'); }
    }).then(function (sizeBytes) {
      if (index[id] !== rec) return;                    // removed while it downloaded
      rec.state = 'done'; rec.sizeBytes = sizeBytes; rec.error = null; rec.at = now();
      return saveRec(rec);
    }, function (err) {
      if (index[id] !== rec) return;
      rec.state = 'failed'; rec.error = err.message;
      return saveRec(rec);
    }).then(function () { active--; changed(); pump(); if (!queue.length && !active) finished(); });
  }
  function finished() {
    var failed = D.list().filter(function (r) { return r.state === 'failed'; });
    if (failed.length) App.toast(failed.length === 1 ? 'Couldn\'t download ' + (failed[0].track ? failed[0].track.title : 'a song') + ': ' + failed[0].error : failed.length + ' songs didn\'t download. Retry them in Downloads.', { kind: 'error', action: 'Downloads', onAction: function () { App.go('/downloads'); } });
  }
  /* ask once for storage the browser won't clear under pressure */
  function persist() {
    if (persisted || app || !navigator.storage || !navigator.storage.persist) return;
    persisted = true;
    navigator.storage.persist().then(function (ok) { if (!ok) console.warn('Storage not persisted: the browser may clear downloads when space runs low'); });
  }

  /* ---------- collections: a song, an album, a playlist or Liked, kept on this device ---------- */
  function collectionIds(k) {
    if (k.kind === 'track') return [k.id];
    if (k.kind === 'album') { var a = M.album(k.id); return a ? a.trackIds : k.ids; }
    if (k.kind === 'playlist') { var p = M.playlist(k.id); return p ? p.items.map(function (i) { return i.trackId; }) : k.ids; }
    if (k.kind === 'liked') return Object.keys(App.store.get('liked') || {});
    return k.ids;
  }
  function saveKept(k) { return tx('kept', 'readwrite', function (s) { return s.put(k); }).catch(function () {}); }
  /* keep({ kind, id }, ids): download these and keep following the collection */
  D.keep = function (c, ids) {
    var why = D.unavailableReason(); if (why) { App.toast(why, { kind: 'error', icon: 'lock' }); return; }
    persist();
    var key = c.kind === 'liked' ? 'liked' : c.kind + ':' + c.id, k = { key: key, kind: c.kind, id: c.id || null, ids: ids.slice() };
    kept[key] = k; saveKept(k);
    var missing = ids.filter(function (id) { return !D.has(id); });
    enqueue(missing);
    if (c.kind !== 'track') App.toast(missing.length ? 'Downloading ' + missing.length + (missing.length === 1 ? ' song' : ' songs') : 'Already on this device', missing.length ? { confirms: true, group: 'download', n: missing.length, together: function (n) { return 'Downloading ' + n + ' songs'; }, icon: 'download' } : { icon: 'download' });
  };
  /* stop keeping a collection: its songs go unless another kept collection still has them */
  D.unkeep = function (key) {
    var k = kept[key]; if (!k) return Promise.resolve();
    delete kept[key];
    tx('kept', 'readwrite', function (s) { return s.delete(key); }).catch(function () {});
    return removeUnneeded(k.ids);
  };
  function needed(id) { return Object.keys(kept).some(function (key) { return kept[key].ids.indexOf(id) >= 0; }); }
  function removeUnneeded(ids) {
    var gone = ids.filter(function (id) { return index[id] && !needed(id); });
    if (!gone.length) { changed(); return Promise.resolve(); }
    var albums = gone.map(function (id) { return index[id].track && index[id].track.albumId; });
    gone.forEach(function (id) { delete index[id]; });
    queue = queue.filter(function (id) { return gone.indexOf(id) < 0; });
    var albumsGone = albums.filter(function (al, i) { return al && albums.indexOf(al) === i && !D.list().some(function (r) { return r.track && r.track.albumId === al; }); });
    if (!app) tx('downloads', 'readwrite', function (s) { gone.forEach(function (id) { s.delete(id); }); return null; }).catch(function () {});
    return backend.drop(gone, albumsGone).then(changed, changed);
  };
  /* remove songs whatever keeps them (the Downloads screen) */
  D.remove = function (ids) {
    Object.keys(kept).forEach(function (key) {
      var k = kept[key]; k.ids = k.ids.filter(function (id) { return ids.indexOf(id) < 0; });
      if (!k.ids.length || k.kind === 'track' && ids.indexOf(k.id) >= 0) { delete kept[key]; tx('kept', 'readwrite', function (s) { return s.delete(key); }).catch(function () {}); }
      else saveKept(k);
    });
    return removeUnneeded(ids);
  };
  D.retry = function (ids) { ids.forEach(function (id) { if (index[id] && index[id].state === 'failed') delete index[id]; }); enqueue(ids); };
  D.estimate = function () { return backend.estimate().catch(function () { return null; }); };
  /* collections follow their source while online: new songs download, dropped ones go */
  function follow() {
    if (OB.offline()) return;
    Object.keys(kept).forEach(function (key) {
      var k = kept[key]; if (k.kind === 'track') return;
      var ids = collectionIds(k); if (JSON.stringify(ids) === JSON.stringify(k.ids)) return;
      var dropped = k.ids.filter(function (id) { return ids.indexOf(id) < 0; });
      k.ids = ids.slice(); saveKept(k);
      enqueue(ids.filter(function (id) { return !D.has(id); }));
      removeUnneeded(dropped);
    });
  }
  App.store.subscribe(function (keys) { if (keys.indexOf('playlists') >= 0 || keys.indexOf('liked') >= 0) follow(); });

  /* ---------- UI parts: the Download button and the downloaded mark on rows ---------- */
  /* ui.downloadButton({ kind, id }, ids): an icon button, live through the 'downloads' event */
  ui.downloadButton = function (c, ids) {
    var key = c.kind === 'liked' ? 'liked' : c.kind + ':' + c.id, cov = D.coverage(ids), on = !!kept[key] && cov.state === 'all';
    var label = cov.busy ? 'Downloading, ' + cov.done + ' of ' + cov.total : on ? 'Downloaded. Remove from this device' : 'Download to this device';
    return html`<button class="btn-icon dl-btn ${on ? 'is-on' : ''}" data-act="dl-toggle" data-dl="${key}" data-kind="${c.kind}" data-id="${c.id || ''}" data-ids="${ids.join(',')}" aria-pressed="${on ? 'true' : 'false'}" aria-label="${label}" data-tip="${cov.busy ? 'Downloading ' + cov.done + ' of ' + cov.total : on ? 'Downloaded' : 'Download'}">${cov.busy ? html`<span class="spin-ring" aria-hidden="true"></span>` : icon(on ? 'downloaded' : 'download')}</button>`;
  };
  App.action('dl-toggle', function (el) {
    var key = el.getAttribute('data-dl'), ids = el.getAttribute('data-ids').split(',').filter(Boolean);
    if (kept[key]) {
      App.confirm({ title: 'remove the download?', text: 'The songs leave this device unless another download keeps them. They still play from your hub when it can be reached.', confirm: 'Remove', danger: true },
        function () { D.unkeep(key).then(function () { App.toast('Removed from this device', { group: 'device-remove', together: function (n) { return 'Removed ' + n + ' from this device'; }, icon: 'trash' }); }); });
      return;
    }
    D.keep({ kind: el.getAttribute('data-kind'), id: el.getAttribute('data-id') }, ids);
  });
  function syncMarks() {
    $$('[data-dl]').forEach(function (b) {
      var c = { kind: b.getAttribute('data-kind'), id: b.getAttribute('data-id') }, ids = b.getAttribute('data-ids').split(',').filter(Boolean);
      var tmp = document.createElement('div'); tmp.innerHTML = String(ui.downloadButton(c, ids));
      var nb = tmp.firstChild; if (nb.outerHTML === b.outerHTML) return;
      var focused = b === document.activeElement; b.replaceWith(nb); if (focused) nb.focus();
    });
    $$('[data-dl-mark]').forEach(function (m) {
      var on = D.has(m.getAttribute('data-dl-mark'));
      if (m.hasAttribute('data-on') === on) return;
      if (on) { m.setAttribute('data-on', ''); m.innerHTML = window.icon('downloaded', 'Downloaded', 'ic-16'); } else { m.removeAttribute('data-on'); m.innerHTML = ''; }
    });
  }
  App.on('route', syncMarks);
})();
