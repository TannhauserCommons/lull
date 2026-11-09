/* model.js: lull's data, as a theme reads it (ADR-0101, amended again 2026-11-09; docs/themes.md sections 6 and 14).
   One module that turns anything lull holds into a record, so a theme's lists, parts and values read lull itself
   rather than a menu of it:
   - LullModel.item(x): any id (its prefix says its kind: t- a track, al- an album, ar- an artist, pl- a playlist) or any
     object naming one ({ trackId, at }, { kind, id }) as that record, with what the object adds;
   - LullModel.record(kind, id): a record with the fields lull derives for it (DERIVE: a track's source and liked);
   - LullModel.list(spec): the items of a list, by a name (NAMED: the queue, Home's feed, history, liked) or of any GET
     read under /api/ (spec.read, the array at spec.key), with where, sort and count; and what it reads and changes on.
   Loaded by designs.js before parts.js and live.js, only for a theme that uses them. It reads what the page already
   holds (App.data, the player); a read it names is lull's own (App.data.get), fetched once and shared with the screen
   that shows it. Without App (the desktop app's own pages) it holds nothing. */
(function () {
  'use strict';
  var Model = window.LullModel = {};

  /* ---------- records ---------- */
  var PREFIX = [['t-', 'track'], ['al-', 'album'], ['ar-', 'artist'], ['pl-', 'playlist']];
  function kindOf(id) {
    for (var i = 0; i < PREFIX.length; i++) if (String(id).indexOf(PREFIX[i][0]) === 0) return PREFIX[i][1];
    return null;
  }
  Model.kindOf = kindOf;
  function data() { return window.App && App.data; }
  var GET = {
    track: function (id) { return data().track(id); },
    album: function (id) { return data().album(id); },
    artist: function (id) { return data().artist(id); },
    playlist: function (id) { return data().playlist(id); }
  };
  /* where a song would play from: this device's download, the library, else Soulseek (what plays decides YouTube) */
  function sourceOf(t) {
    if (App.downloads && App.downloads.has(t.id)) return 'device';
    return App.isSaved(t.id) ? 'library' : 'soulseek';
  }
  /* the fields lull works out for a record, beside those it holds: the one place a derived field is written */
  var DERIVE = {
    track: function (t) { return { kind: 'track', source: sourceOf(t), liked: App.isLiked(t.id) }; },
    album: function () { return { kind: 'album' }; },
    artist: function () { return { kind: 'artist' }; },
    playlist: function () { return { kind: 'playlist' }; }
  };
  Model.record = function (kind, id) {
    if (!data() || !GET[kind] || id == null) return null;
    var r = GET[kind](id);
    return r ? Object.assign({}, r, DERIVE[kind](r)) : null;
  };
  /* an id or an object as its record; the object's own fields (a play's `at`, a queued song's `group`) stay beside it */
  Model.item = function (x) {
    if (x == null) return null;
    if (typeof x !== 'object') { var k = kindOf(x); return (k && Model.record(k, x)) || { kind: 'text', id: String(x), name: String(x) }; }
    var id = x.trackId || x.albumId || x.artistId || x.playlistId || x.id;
    var kind = x.trackId ? 'track' : x.albumId ? 'album' : x.artistId ? 'artist' : x.kind || kindOf(id);
    var r = kind && id ? Model.record(kind, id) : null;
    return r ? Object.assign({}, x, r) : Object.assign({ kind: kind || 'object' }, x);
  };

  /* ---------- lists ---------- */
  var GROUPS = { next: 'yours', context: 'place', radio: 'radio' };
  /* named lists: what lull holds in the page (the queue), or a read and where its items are */
  var NAMED = {
    queue: { on: ['track', 'queue', 'ended'], items: function () {
      var ps = App.player.state;
      return (ps.queue || []).slice((ps.index || 0) + 1).map(function (q) { return { trackId: q.trackId, group: GROUPS[q.group] || q.group || '' }; });
    } },
    history: { read: function () { return '/api/history?tz=' + -new Date().getTimezoneOffset(); }, key: 'days.items' },
    liked: { read: '/api/likes', key: 'items' }
  };
  /* the value at a dotted key, flattening arrays on the way (days.items: every day's items, in order) */
  function at(v, key) {
    (key ? key.split('.') : []).forEach(function (k) {
      v = Array.isArray(v) ? [].concat.apply([], v.map(function (x) { return x == null ? [] : [].concat(x[k] == null ? [] : x[k]); })) : v == null ? v : v[k];
    });
    return v;
  }
  /* a read a theme may name: a GET under /api/, as lull's own screens read them */
  function readable(path) { return typeof path === 'string' && /^\/api\/[\w\-./?=&%:,]*$/.test(path) && path.indexOf('..') < 0; }
  /* where="group=yours,kind=album": every field equal (a value may list choices: group=place|radio) */
  function matches(where) {
    var tests = String(where || '').split(',').map(function (w) { var i = w.indexOf('='); return i > 0 ? [w.slice(0, i).trim(), w.slice(i + 1).split('|')] : null; }).filter(Boolean);
    return function (it) { return tests.every(function (t) { return t[1].indexOf(String(at(it, t[0]))) >= 0; }); };
  }
  /* sort="title" or "-duration": by a field, ascending, or descending with a minus */
  function sorter(by) {
    var down = by.charAt(0) === '-', k = down ? by.slice(1) : by;
    return function (a, b) { var x = at(a, k), y = at(b, k); return (x > y ? 1 : x < y ? -1 : 0) * (down ? -1 : 1); };
  }
  /* spec: { of, read, key, where, sort, count } -> { items, read (the path it reads, or null), on (player kinds) } */
  Model.list = function (spec) {
    if (!data()) return { items: [], read: null, on: [] };
    var named = NAMED[spec.of], feed = /^home\.([\w-]+)$/.exec(spec.of || '');
    var src = named || (feed ? { read: '/api/feed', key: feed[1] === 'jump' ? 'jumpBackIn' : feed[1] === 'new' ? 'newReleases' : feed[1] === 'radio' ? 'radioStarters' : feed[1] } : { read: spec.read, key: spec.key });
    var read = typeof src.read === 'function' ? src.read() : src.read;
    if (read && !readable(read)) return { items: [], read: null, on: [] };
    var raw = src.items ? src.items() : read ? at(data().peek(read), src.key) : [];
    var items = [].concat(raw || []).map(Model.item).filter(Boolean).filter(matches(spec.where));
    if (spec.sort) items.sort(sorter(spec.sort));
    var n = parseInt(spec.count, 10);
    return { items: items.slice(0, n > 0 ? Math.min(n, 200) : 50), read: read || null, on: src.on || [] };
  };
  Model.readable = readable;
})();
