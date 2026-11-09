/* lull data layer: the API client, the entity lookup cache and the server event stream.
   Exposes window.LullData, which app.js publishes as App.data (screens call it M).
   - M.track/album/artist/tag/playlist(id), M.albumsBy(artistId), M.tracksBy(artistId) answer synchronously
     from what responses have carried so far (every response's `entities`, docs/context/api.md#conventions).
   - M.get(path) is the per-route read: it returns { loading, error, data } at once and fetches the first time;
     when the fetch settles, the route that asked is re-rendered (App.refresh), so screens render the skeleton,
     then the data or the honest error.
   - M.api(path, { method, body }) is the plain call for actions; it rejects with an ApiError carrying the
     server's sentence ({ error, message }), which the UI shows as is.
   - M.connect(onEvent) opens the one EventSource('/api/events'). */
window.LullData = (function () {
  'use strict';

  var tracks = {}, albums = {}, artists = {}, tags = {};
  var M = {
    saveSteps: ['Completing transfer', 'Matching on MusicBrainz', 'Fetching cover art', 'Tagging and writing to library', 'Adding to your library'],
    /* quality chip facts by format label ("FLAC 16/44.1"), learned from the tracks that carry them */
    formats: {},
    playlists: []
  };

  /* ---------- lookups ---------- */
  M.track = function (id) { return tracks[id]; };
  M.album = function (id) { return albums[id]; };
  M.artist = function (id) { return artists[id]; };
  /* a tag's id is its name; tags no response described yet still resolve to their name */
  M.tag = function (id) { return id == null ? undefined : tags[id] || { id: id, name: id, desc: '', related: [] }; };
  M.playlist = function (id) { return M.playlists.filter(function (p) { return p.id === id; })[0]; };
  M.albumsBy = function (artistId) {
    return values(albums).filter(function (a) { return a.artistId === artistId; })
      .sort(function (a, b) { return (b.year || 0) - (a.year || 0); });
  };
  M.tracksBy = function (artistId) { return values(tracks).filter(function (t) { return t.artistId === artistId; }); };
  function values(map) { return Object.keys(map).map(function (k) { return map[k]; }); }

  /* ---------- entities: merged in from every response, normalised to the shapes the screens read ---------- */
  /* a merge that moves a song to another album, or gives an album a cover, hands M.onMoved those ids (songs' and albums')
     once it is whole, so whatever shows the song draws it again, whichever response or event brought the change */
  M.absorb = function (e) {
    if (!e) return;
    var moved = {};
    each(e.artists, function (a) { artists[a.id] = Object.assign(artists[a.id] || {}, a, { tags: a.tags || [] }); });
    each(e.albums, function (a) { var o = albums[a.id]; if (!o || a.art && o.art !== a.art) moved[a.id] = 1; albums[a.id] = normAlbum(Object.assign(o || {}, a)); });
    each(e.tracks, function (t) { var o = tracks[t.id]; if (o && t.albumId && o.albumId !== t.albumId) moved[t.id] = 1; tracks[t.id] = normTrack(Object.assign(o || {}, t)); });
    each(e.tags, function (t) { tags[t.id] = Object.assign(tags[t.id] || {}, t); });
    each(e.albums, function (a) { albumFacts(albums[a.id]); });
    if (M.onMoved && Object.keys(moved).length) M.onMoved(moved);
  };
  function each(map, fn) { if (map) Object.keys(map).forEach(function (k) { fn(map[k]); }); }
  function normTrack(t) {
    t.tags = t.tags || [];
    t.duration = t.duration || 0;
    t.sizeMB = t.sizeBytes ? t.sizeBytes / 1e6 : t.sizeMB || 0;
    if (t.format && !M.formats[t.format]) {
      M.formats[t.format] = { codec: t.codec || t.format.split(' ')[0], label: t.format, lossless: !!t.lossless,
        khz: t.khz || null, bits: t.bits || null, kbps: t.kbps || null };
    }
    return t;
  }
  function normAlbum(a) {
    a.trackIds = a.trackIds || [];
    a.tags = a.tags || [];
    a.type = a.type || 'album';
    a.released = a.released || (a.year ? String(a.year) : '');
    return a;
  }
  /* album-level facts the screens show (size, format, duration) are the sum of its known tracks */
  function albumFacts(a) {
    var known = a.trackIds.map(M.track).filter(Boolean);
    if (!known.length) return;
    a.sizeMB = known.reduce(function (s, t) { return s + (t.sizeMB || 0); }, 0);
    if (!a.duration) a.duration = known.reduce(function (s, t) { return s + (t.duration || 0); }, 0);
    if (!a.format) a.format = known[0].format;
  }
  /* local knowledge the server confirms by event (a finished save flips a track to saved) */
  M.patchTrack = function (id, patch) { if (tracks[id]) Object.assign(tracks[id], patch); };

  /* ---------- HTTP ---------- */
  function ApiError(status, kind, message) {
    this.status = status; this.kind = kind; this.message = message;
  }
  ApiError.prototype = Object.create(Error.prototype);
  M.ApiError = ApiError;
  /* same origin, relative to the page, so lull also works behind a reverse-proxy sub-path */
  M.url = function (path) { return path.charAt(0) === '/' ? path.slice(1) : path; };
  /* set by app.js: called on any 401 so the token gate can open */
  M.onUnauthorized = function () {};

  /* a read already on its way is shared: two parts of the page that ask for the same thing at once (the player's
     settings and Settings > Playback, private listening and the device list, at start) make one request. A write
     forgets every read on its way, so a read sent after a change never answers with what was there before it */
  var reading = {};
  M.api = function (path, o) {
    o = o || {};
    if ((o.method || 'GET') !== 'GET') reading = {};
    else if (o.body === undefined && !o.keepalive) {
      if (reading[path]) return reading[path];
      var mine = reading[path] = send(path, o);
      var done = function () { if (reading[path] === mine) delete reading[path]; };
      mine.then(done, done);
      return mine;
    }
    return send(path, o);
  };
  function send(path, o) {
    var init = { method: o.method || 'GET', headers: {}, credentials: 'same-origin', keepalive: !!o.keepalive };
    if (o.body !== undefined) {
      var file = typeof Blob !== 'undefined' && o.body instanceof Blob;     // a file goes as it is (imports)
      init.body = typeof o.body === 'string' || file ? o.body : JSON.stringify(o.body);
      init.headers['Content-Type'] = file ? o.body.type || 'application/octet-stream' : typeof o.body === 'string' ? 'text/plain' : 'application/json';
    }
    return fetch(M.url(path), init).then(function (res) {
      if (res.status === 204) return null;
      var json = (res.headers.get('Content-Type') || '').indexOf('json') >= 0;
      var body = json ? res.json() : res.text();
      return body.then(function (data) {
        if (!res.ok) {
          if (res.status === 401) M.onUnauthorized();
          throw new ApiError(res.status, data && data.error || 'http_' + res.status,
            data && data.message || 'The server answered ' + res.status + ' for ' + path + '.');
        }
        if (data && data.entities) M.absorb(data.entities);
        return data;
      });
    }, function () {
      throw new ApiError(0, 'unreachable', 'Can\'t reach the lull server. Check your connection.');
    });
  }

  /* ---------- per-route reads ---------- */
  var records = {}, generation = 0;
  /* set by app.js: re-render the route when a read it made settles */
  M.onSettled = function () {};
  M.get = function (path) {
    var r = records[path];
    if (!r) {
      r = records[path] = { loading: true, error: null, data: null };
      load(path, r);
    }
    r.generation = generation;
    return r;
  };
  M.peek = function (path) { var r = records[path]; return r && r.data; };
  /* M.onRead(path): set by a part a theme placed away from its screen (designs/parts.js), told each time a read settles */
  M.onRead = null;
  function load(path, r) {
    var mine = r.seq = (r.seq || 0) + 1;
    M.api(path).then(function (data) {
      if (records[path] !== r || r.seq !== mine) return;
      r.loading = false; r.error = null; r.data = data;
      settled(r, path);
    }, function (err) {
      if (records[path] !== r || r.seq !== mine) return;
      r.loading = false; r.error = err;
      settled(r, path);
    });
  }
  function settled(r, path) { if (r.generation === generation) M.onSettled(); if (M.onRead) M.onRead(path); }
  /* the route changed: earlier routes' reads no longer re-render anything */
  M.nextRoute = function () { generation += 1; };
  /* refetch a read in place (the old data stays visible until the new answer arrives) */
  M.reload = function (path) {
    var r = records[path];
    if (!r) return M.get(path);
    r.generation = generation;
    load(path, r);
    return r;
  };
  /* forget reads whose path starts with any prefix (all when none given); the next M.get refetches */
  M.invalidate = function (prefixes) {
    Object.keys(records).forEach(function (p) {
      if (!prefixes || [].concat(prefixes).some(function (x) { return p.indexOf(x) === 0; })) delete records[p];
    });
  };

  /* ---------- server events ---------- */
  /* one EventSource for the whole app; onEvent(topic, payload). Reconnects by itself (EventSource does);
     onOpen runs on every (re)connect so the app can refetch what it may have missed.
     The topics are the ones the app listens for: M.listen(topic), which App.on('server:<topic>') and the engine's own
     handlers call, so a screen's subscription always reaches the stream (named SSE events need one listener each). */
  var topics = {}, live = null, deliver = null;
  function attach(es, topic) {
    es.addEventListener(topic, function (e) {
      var data = null;
      try { data = JSON.parse(e.data); } catch (err) { console.error('Bad ' + topic + ' event', e.data); return; }
      if (deliver) deliver(topic, data);
    });
  }
  M.listen = function (topic) {
    if (topics[topic]) return;
    topics[topic] = true;
    if (live) attach(live, topic);
  };
  M.connect = function (onEvent, onOpen, onDown) {
    var es = live = new EventSource(M.url('/api/events'));
    deliver = onEvent;
    Object.keys(topics).forEach(function (topic) { attach(es, topic); });
    es.onopen = onOpen;
    es.onerror = onDown;
    return es;
  };

  return M;
})();
