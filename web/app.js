/* lull core: one global `App` namespace, no modules, no framework.
   Sections: 1 utilities + html template, 2 events + store, 3 preferences (theme/motion/text size/design),
   4 router, 5 render helpers (App.ui), 6 overlays (sheet, dialog, popover, menu, tooltip,
   toast, banner), 7 save pipeline, 8 player engine, 9 live bindings (seek bars, waveforms, rows),
   10 shell (rail, header, tray, context panel, player bar, mini-player), 11 actions, 12 keyboard,
   13 media session, 14 drag and drop, 15 server events and boot.
   The API used by screens is design/DESIGN.md section 7; the data layer and player engine are in
   docs/context/web.md. Data comes from data.js (App.data). */
(function () {
  'use strict';
  var App = window.App = {};
  var M = window.LullData;

  /* =====================================================================
     1. Utilities + html tagged template
     ===================================================================== */
  function Safe(s) { this.s = s; }
  Safe.prototype.toString = function () { return this.s; };
  function raw(s) { return new Safe(String(s)); }
  function esc(v) {
    return String(v).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }
  function part(v) {
    if (v == null || v === false || v === true) return '';
    if (v instanceof Safe) return v.s;
    if (Array.isArray(v)) return v.map(part).join('');
    return esc(v);
  }
  /* html`<b>${text}</b>` escapes interpolated strings; Safe values (from html, App.raw, App.ui.*,
     App.icon) and arrays of them are inserted as markup. Returns a Safe (use String(x) or innerHTML = x). */
  function html(strings) {
    var out = strings[0];
    for (var i = 1; i < strings.length; i++) out += part(arguments[i]) + strings[i];
    return new Safe(out);
  }
  function icon(name, label, cls) { return raw(window.icon(name, label, cls)); }
  function $(sel, root) { return (root || document).querySelector(sel); }
  function $$(sel, root) { return Array.prototype.slice.call((root || document).querySelectorAll(sel)); }
  function clamp(v, a, b) { return Math.max(a, Math.min(b, v)); }
  function fmtTime(sec, estimated) {
    sec = Math.max(0, Math.round(sec || 0));
    return (estimated ? '~' : '') + Math.floor(sec / 60) + ':' + String(sec % 60).padStart(2, '0');
  }
  var MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  /* an ISO date as precise as known ("2026", "2026-10", "2026-10-29") or unix seconds -> "Oct 29, 2026" */
  function fmtDate(when, withYear) {
    if (when == null || when === '') return '';
    if (typeof when === 'number') when = new Date(when * 1000).toISOString().slice(0, 10);
    var p = String(when).split('-');
    if (!p[1]) return p[0];
    if (!p[2]) return MONTHS[+p[1] - 1] + ' ' + p[0];
    return MONTHS[+p[1] - 1] + ' ' + (+p[2]) + (withYear ? ', ' + p[0] : '');
  }
  function fmtLength(sec) {                 // "48 min" / "1 h 12 min"
    var m = Math.round(sec / 60);
    return m >= 60 ? Math.floor(m / 60) + ' h ' + (m % 60) + ' min' : m + ' min';
  }
  function hash(str) {                     // FNV-1a, 32-bit: stable across loads and browsers
    var h = 0x811c9dc5;
    for (var i = 0; i < str.length; i++) { h ^= str.charCodeAt(i); h = Math.imul(h, 0x01000193); }
    return h >>> 0;
  }
  function storage(key, value) {           // localStorage wrapped in try/catch (private mode, file://)
    try {
      if (arguments.length === 1) return JSON.parse(localStorage.getItem('lull:' + key));
      localStorage.setItem('lull:' + key, JSON.stringify(value));
    } catch (e) { return null; }
  }
  var uid = 0;
  function nextId(prefix) { uid += 1; return (prefix || 'id') + '-' + uid; }

  App.html = html; App.raw = raw; App.esc = esc; App.icon = icon; App.$ = $; App.$$ = $$;
  function fmtSize(mb) {                    // "30.2 MB" / "1.4 GB"
    mb = +mb || 0;
    return mb >= 1000 ? (Math.round(mb / 100) / 10) + ' GB' : (mb >= 100 ? Math.round(mb) : Math.round(mb * 10) / 10) + ' MB';
  }
  function fmtClock(sec) {                  // "00:36:12" (album heads, media-player LCDs)
    sec = Math.max(0, Math.round(sec || 0));
    return [Math.floor(sec / 3600), Math.floor(sec / 60) % 60, sec % 60].map(function (n) { return String(n).padStart(2, '0'); }).join(':');
  }
  /* one line of file facts for a track (or a source { format, sizeMB }):
     formatLine(t) -> "FLAC :: 44.1 kHz, 1040 kbps, 30.2 MB"; { bits: true } adds "16-bit, " before the rate */
  function formatLine(t, opts) {
    if (!t) return '';
    var f = M.formats[t.format] || {}, o = opts || {}, parts = [];
    var khz = t.khz || f.khz, kbps = t.kbps || f.kbps, bits = t.bits !== undefined ? t.bits : f.bits;
    if (khz) parts.push((o.bits && bits ? bits + '-bit, ' : '') + khz + ' kHz');
    if (kbps) parts.push(kbps + ' kbps');
    if (t.sizeMB) parts.push(fmtSize(t.sizeMB));
    return (f.codec || String(t.format || '').split(' ')[0]) + ' :: ' + parts.join(', ');
  }
  /* album facts: "Tracks: 8  Duration: 00:36:12  Size: 250 MB" */
  function albumLine(a) {
    return a ? 'Tracks: ' + a.trackIds.length + '  Duration: ' + fmtClock(a.duration) + (a.sizeMB ? '  Size: ' + fmtSize(a.sizeMB) : '') : '';
  }
  App.fmt = { time: fmtTime, date: fmtDate, length: fmtLength, size: fmtSize, clock: fmtClock, formatLine: formatLine, albumLine: albumLine };
  App.storage = storage; App.data = M; App.clamp = clamp; App.uid = nextId;

  /* =====================================================================
     2. Events + store
     ===================================================================== */
  var listeners = {};
  App.on = function (name, fn) {
    (listeners[name] = listeners[name] || []).push(fn);
    return function off() { listeners[name] = (listeners[name] || []).filter(function (f) { return f !== fn; }); };
  };
  App.emit = function (name, detail) {
    (listeners[name] || []).slice().forEach(function (fn) {
      try { fn(detail); } catch (e) { console.error('[' + name + ' listener]', e); }
    });
  };

  /* the four services the tray shows; /health and `service` events fill them in (state off = not reported) */
  var SERVICE_LABELS = { soulseek: 'soulseek', jellyfin: 'jellyfin', lastfm: 'last.fm', cache: 'cache' };
  function serviceDefaults() {
    var s = {};
    Object.keys(SERVICE_LABELS).forEach(function (k) { s[k] = { state: 'off', label: SERVICE_LABELS[k], detail: 'Waiting for the server' }; });
    return s;
  }
  var state = {
    liked: {},                     // { trackId: true }            GET /api/likes
    followed: {},                  // { artistId | tagId: true }   GET /api/follows
    saves: {},                     // { trackId: { step 1-5, status: saving|saved|failed|pending, pct, error, jobId } }
    services: serviceDefaults(),   // GET /health + `service` events
    activity: [],                  // GET /api/jobs + `job` / `transfer` events
    playlists: [],                 // GET /api/playlists
    recentSearches: storage('recentSearches') || [],   // this device only
    hiddenArtists: {},
    ctxTab: 'queue',
    crossfadeSec: 0
  };
  /* App.store: get(key), set(key, value) or set({ key: value }), update(key, fn), subscribe(fn(keys)) -> unsubscribe */
  App.store = {
    get: function (key) { return key ? state[key] : state; },
    set: function (key, value) {
      var patch = typeof key === 'object' ? key : {}; if (typeof key === 'string') patch[key] = value;
      Object.keys(patch).forEach(function (k) { state[k] = patch[k]; });
      if ('recentSearches' in patch) storage('recentSearches', state.recentSearches);
      if ('playlists' in patch) M.playlists = state.playlists;
      App.emit('store', Object.keys(patch));
    },
    update: function (key, fn) { var v = fn(state[key]); App.store.set(key, v === undefined ? state[key] : v); },
    subscribe: function (fn) { return App.on('store', fn); }
  };

  App.track = function (id) { return M.track(id); };
  App.isLiked = function (id) { return !!state.liked[id]; };
  App.isFollowed = function (id) { return !!state.followed[id]; };
  App.isSaved = function (id) {
    var s = state.saves[id];
    if (s) return s.status === 'saved';
    var t = M.track(id); return !!(t && t.saved);
  };
  /* likes and follows change at once and are sent to the server; a refusal puts them back and says why */
  App.toggleLike = function (id, force) {
    var on = force === undefined ? !state.liked[id] : !!force;
    setLiked([id], on);
    App.toast(on ? 'Added to Liked songs' : 'Removed from Liked songs', {
      icon: on ? 'heart-fill' : 'heart', action: 'Undo', onAction: function () { App.toggleLike(id, !on); }, quiet: true
    });
  };
  /* setLiked(ids, on): many at once (album and selection actions) */
  function setLiked(ids, on) {
    App.store.update('liked', function (l) { ids.forEach(function (id) { if (on) l[id] = true; else delete l[id]; }); });
    ids.forEach(function (id) { App.emit('like', { id: id, liked: on, track: M.track(id) }); });
    Promise.all(ids.map(function (id) {
      return M.api('/api/likes/' + encodeURIComponent(id), { method: on ? 'PUT' : 'DELETE' });
    })).catch(function (err) {
      App.store.update('liked', function (l) { ids.forEach(function (id) { if (on) delete l[id]; else l[id] = true; }); });
      ids.forEach(function (id) { App.emit('like', { id: id, liked: !on, track: M.track(id) }); });
      App.toast('Couldn\'t ' + (on ? 'like' : 'unlike') + ': ' + err.message, { kind: 'error' });
    });
  }
  App.setLiked = setLiked;
  App.toggleFollow = function (id, force) {
    var on = force === undefined ? !state.followed[id] : !!force, kind = /^ar-/.test(id) ? 'artist' : 'tag';
    var a = M.artist(id), name = a ? a.name : id;
    App.store.update('followed', function (f) { if (on) f[id] = true; else delete f[id]; });
    App.emit('follow', { id: id, followed: on });
    App.toast(on ? 'Following ' + name : 'Unfollowed ' + name, { icon: 'follow', action: 'Undo', onAction: function () { App.toggleFollow(id, !on); } });
    M.api('/api/follows/' + kind + '/' + encodeURIComponent(id), { method: on ? 'PUT' : 'DELETE' }).catch(function (err) {
      App.store.update('followed', function (f) { if (on) delete f[id]; else f[id] = true; });
      App.emit('follow', { id: id, followed: !on });
      App.toast('Couldn\'t ' + (on ? 'follow ' : 'unfollow ') + name + ': ' + err.message, { kind: 'error' });
    });
  };

  /* =====================================================================
     3. Preferences: theme (day / night / system), motion, text size, design. Persisted, try/catch.
     ===================================================================== */
  var root = document.documentElement;
  var prefs = storage('prefs') || {};
  function applyPrefs(p) {
    Designs.applyTheme(p.theme);                     // 'system' (null) -> explicit day / night, follows the OS
    if (p.motion === 'reduce' || p.motion === 'full') root.setAttribute('data-motion', p.motion); else root.removeAttribute('data-motion');
    root.style.fontSize = p.textSize && +p.textSize !== 100 ? +p.textSize + '%' : '';
    var meta = $('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', getComputedStyle(root).getPropertyValue('--sky').trim() || '#f7f4ff');
    App.emit('prefs', p);
  }
  App.prefs = function () {
    return { theme: prefs.theme || 'system', motion: prefs.motion || 'system', textSize: +prefs.textSize || 100, design: Designs.current };
  };
  /* keys: theme ('day'|'night'|null), motion ('reduce'|'full'|'system'), textSize (90|100|112|125), design (name) */
  App.setPref = function (key, value, temporary) {
    if (key === 'design') { App.setDesign(value, temporary); return; }
    prefs[key] = value; applyPrefs(prefs);
    if (!temporary) storage('prefs', prefs);
  };
  /* the theme actually showing: 'day' | 'night' (always explicit on :root[data-theme], see Designs.applyTheme) */
  App.theme = function () { return root.getAttribute('data-theme') === 'night' ? 'night' : 'day'; };
  App.toggleTheme = function () { App.setPref('theme', App.theme() === 'night' ? 'day' : 'night'); };
  App.reducedMotion = function () {
    var m = root.getAttribute('data-motion');
    if (m) return m === 'reduce';
    return !!window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches;
  };
  /* global flags in the hash (not persisted) win over stored prefs, from the first apply on, so there is no flash */
  function applyFlags(q) {
    if (q.theme) prefs.theme = q.theme === 'system' ? null : q.theme;
    if (q.motion) prefs.motion = q.motion;
  }
  applyFlags(parseHash().query);
  applyPrefs(prefs);

  /* Designs (designs/designs.js): App.designs = { name: { label, description, swatches, shell, preview } },
     App.designOrder, App.design() = showing design, App.setDesign(name, temporary) swaps the stylesheet,
     re-renders the shell and the current route without a reload. App.slot(name, ctx, fallback): markup a
     design.js may replace (see Designs.define). The pre-paint script in index.html already applied it. */
  App.designs = Designs.list; App.designOrder = Designs.order;
  App.design = function () { return Designs.current; };
  App.slot = Designs.slot;
  App.product = Designs.product;              // the product name ('lull') for titles and copy
  App.setDesign = function (name, temporary) {
    if (!Designs.valid(name)) { console.warn('Unknown design', name); return false; }
    var was = prefs.design;
    if (!temporary) { prefs.design = name; storage('prefs', prefs); }
    return Designs.set(name, function () {
      if (!booted) return;
      renderShell(); renderRoute(true);
      applyPrefs(prefs);                       // theme-color + 'prefs' listeners (settings, switcher) repaint
      App.emit('design', name);
    }, function () {                           // stylesheet failed: the current design stays
      if (!temporary) { prefs.design = was; storage('prefs', prefs); }
      if (!booted) return;
      applyPrefs(prefs);
      App.toast('Couldn\'t load the ' + Designs.list[name].label + ' design. Still using ' + Designs.list[Designs.current].label + '.', { kind: 'error' });
    });
  };

  /* =====================================================================
     4. Router
     ===================================================================== */
  var routes = [];
  App.current = { path: '/home', params: {}, query: {}, route: null };
  var cleanups = [];

  /* App.route('/album/:id', render, opts) or App.route(pattern, { render, mount, title, nav, view }).
     render(ctx) returns markup (Safe or string); mount(ctx) runs after it is in the DOM.
     ctx = { params, query, el (the view), onCleanup(fn) }. ':name?' marks an optional segment. */
  App.route = function (pattern, render, opts) {
    var def = typeof render === 'function' ? Object.assign({ render: render }, opts || {}) : render;
    var keys = [];
    var re = new RegExp('^' + pattern.replace(/\/:(\w+)(\?)?/g, function (_, k, opt) {
      keys.push(k); return opt ? '(?:/([^/]+))?' : '/([^/]+)';
    }) + '/?$');
    def.pattern = pattern; def.re = re; def.keys = keys;
    routes = routes.filter(function (r) { return r.pattern !== pattern; });
    routes.push(def);
  };
  function parseHash() {
    var h = location.hash.replace(/^#/, '') || '/home';
    var qi = h.indexOf('?'), path = qi >= 0 ? h.slice(0, qi) : h, query = {};
    var qs = (qi >= 0 ? h.slice(qi + 1) : '') + (location.search ? '&' + location.search.slice(1) : '');
    qs.split('&').forEach(function (kv) {
      if (!kv) return; var p = kv.split('='); query[decodeURIComponent(p[0])] = decodeURIComponent((p[1] || '').replace(/\+/g, ' '));
    });
    if (path === '/' || path === '') path = '/home';
    return { path: path, query: query };
  }
  App.go = function (path, opts) {
    if (opts && opts.replace) history.replaceState(null, '', '#' + path); else location.hash = path;
    if (opts && opts.replace) renderRoute();
  };
  App.back = function () { if (history.length > 1) history.back(); else App.go('/home'); };
  /* re-render the current route (screens call this after their own state changes) */
  App.refresh = function () { renderRoute(true); };

  var lastPath = null;
  function renderRoute(isRefresh) {
    var loc = parseHash(), route = null, params = {};
    for (var i = 0; i < routes.length; i++) {
      var m = routes[i].re.exec(loc.path);
      if (m) { route = routes[i]; route.keys.forEach(function (k, j) { if (m[j + 1] !== undefined) params[k] = decodeURIComponent(m[j + 1]); }); break; }
    }
    // global flags (not persisted): theme, motion, design
    if (loc.query.theme || loc.query.motion) { applyFlags(loc.query); applyPrefs(prefs); }
    if (!isRefresh && loc.query.design && loc.query.design !== Designs.current && Designs.valid(loc.query.design)) App.setDesign(loc.query.design, true);
    if (!isRefresh) M.nextRoute();                    // reads made by the previous route stop re-rendering

    cleanups.forEach(function (fn) { try { fn(); } catch (e) { console.error(e); } }); cleanups = [];
    var view = $('#view');
    var ctx = { params: params, query: loc.query, el: view, route: route,
      onCleanup: function (fn) { cleanups.push(fn); } };
    App.current = { path: loc.path, params: params, query: loc.query, route: route };

    var markup;
    if (!route) markup = notFound(loc.path);
    else { try { markup = route.render(ctx); } catch (e) { console.error(e); markup = renderError(e); } }
    view.innerHTML = String(markup || '');
    if (route && route.mount) { try { route.mount(ctx); } catch (e) { console.error(e); } }

    $('#app').setAttribute('data-view', (route && route.view) || 'page');
    $('#app').setAttribute('data-route', (route && route.nav) || '');     // design hook: .app[data-route="home"]
    var title = route && route.title ? (typeof route.title === 'function' ? route.title(ctx) : route.title) : '';
    document.title = (title ? title + ' - ' : '') + App.product;
    /* title-bar hooks: .app[data-screen] = first path segment (album, playlist, now ...), .main[data-title] = the
       page title, .main[data-meta] = the route's live meta line (route.meta(ctx), e.g. "9 songs, 37 min"; App.setMeta) */
    $('#app').setAttribute('data-screen', loc.path.split('/')[1] || 'home');
    $('.main').setAttribute('data-title', String(title).toLowerCase());
    App.setMeta(route && route.meta ? safeCall(route.meta, ctx) : '');
    var nav = route && route.nav;
    $$('[data-nav]').forEach(function (a) {
      if (a.getAttribute('data-nav') === nav) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current');
    });
    $$('.rail__pl').forEach(function (a) {
      if (a.getAttribute('href') === '#' + loc.path) a.setAttribute('aria-current', 'page'); else a.removeAttribute('aria-current');
    });
    if (!isRefresh && lastPath !== loc.path) {
      view.scrollTop = 0;
      if (lastPath !== null && !(route && route.keepFocus)) view.focus({ preventScroll: true });
    }
    lastPath = loc.path;
    App.syncRows();
    App.emit('route', App.current);
  }
  function safeCall(fn, arg) { try { return fn(arg) || ''; } catch (e) { console.error(e); return ''; } }
  /* the content window's meta line (.main[data-meta]); screens may update it after a change */
  App.setMeta = function (text) { var m = $('.main'); if (m) m.setAttribute('data-meta', text || ''); };
  function notFound(path) {
    return html`<div class="card" style="max-width:640px">${App.ui.emptyState({ art: 'star', title: 'Nothing lives here',
      text: 'The address ' + path + ' does not match a screen. Head home and pick something to play.',
      action: { label: 'Go home', href: '#/home' } })}</div>`;
  }
  /* the honest error card for a read that failed (M.get(...).error): the server's own sentence + retry */
  App.errorCard = function (err, o) {
    o = o || {};
    return html`<div class="card" ${o.label ? raw('aria-label="' + esc(o.label) + '"') : ''}>${App.ui.emptyState({ art: o.art || 'star',
      title: o.title || 'Couldn\'t load this', text: err && err.message || String(err), action: { label: 'Try again', act: 'reload-read', id: o.path || '' } })}</div>`;
  };
  function renderError(e) {
    return html`<div class="card" style="max-width:640px">${App.ui.emptyState({ art: 'star', title: 'This screen hit a snag',
      text: String(e && e.message || e), action: { label: 'Reload', act: 'reload' } })}</div>`;
  }

  /* =====================================================================
     5. Render helpers (App.ui). All return Safe markup.
     ===================================================================== */
  var ui = App.ui = {};
  ui.icon = icon;

  /* cover recipe .c1-.c8 by a stable hash of the album (or artist / playlist) id */
  App.coverClass = function (key) { return 'c' + (hash(String(key)) % 8 + 1); };
  ui.cover = function (item, size, opts) {
    opts = opts || {}; size = size == null ? 48 : size;
    var key = typeof item === 'string' ? item : (item.albumId || item.id);
    var px = typeof size === 'number';
    var cls = 'cover ' + App.coverClass(key) + (px && size < 40 ? ' cover--xs' : px && size <= 64 ? ' cover--thumb' : '') +
      (opts.round ? ' cover--round' : '') + (opts.cls ? ' ' + opts.cls : '');
    var a11y = opts.label ? ' role="img" aria-label="' + esc(opts.label) + '"' : ' aria-hidden="true"';
    var c = { key: String(key), recipe: App.coverClass(key), size: px ? size + 'px' : size, cls: cls, attrs: a11y,
      round: !!opts.round, label: opts.label || '' };
    return raw(String(App.slot('coverArt', c, coverMarkup)));
  };
  /* default coverArt slot: one span; the .c1-.c8 recipe draws the art in CSS until (and unless) the real cover
     loads: <img class="cover__img"> from /api/art, and .has-art on the span once it has (see the load listener) */
  function coverMarkup(c) {
    return '<span class="' + c.cls + '" style="--cover-size-base:' + c.size + '"' + c.attrs + '>' + artImg(c.key, c.size) + '</span>';
  }
  var artMissing = {};          // albums whose art 404'd this session: not asked for again
  function artImg(key, size) {
    if (!/^al-/.test(key) || artMissing[key]) return '';
    var px = parseInt(size, 10), want = /%$/.test(size) || !px ? 256 : px <= 96 ? 96 : px <= 256 ? 256 : 512;
    return '<img class="cover__img" loading="lazy" decoding="async" alt="" data-art="' + esc(key) + '" src="' + esc(App.artUrl(key, want)) + '">';
  }
  App.artUrl = function (albumId, size) { return M.url('/api/art/' + encodeURIComponent(albumId) + '?size=' + size); };
  document.addEventListener('load', function (e) {
    var img = e.target; if (img.tagName === 'IMG' && img.classList.contains('cover__img')) img.parentNode.classList.add('has-art');
  }, true);
  document.addEventListener('error', function (e) {
    var img = e.target; if (img.tagName !== 'IMG' || !img.classList.contains('cover__img')) return;
    artMissing[img.getAttribute('data-art')] = true; img.remove();
  }, true);
  /* 2x2 mosaic of the first four distinct album covers (playlists) */
  ui.mosaic = function (trackIds, size, label) {
    var seen = {}, keys = [];
    trackIds.forEach(function (id) { var t = M.track(id); if (t && !seen[t.albumId] && keys.length < 4) { seen[t.albumId] = 1; keys.push(t.albumId); } });
    while (keys.length < 4) keys.push('pad-' + keys.length);
    var px = typeof size === 'number' ? size + 'px' : size;
    return raw('<span class="mosaic" style="--cover-size-base:' + px + '"' + (label ? ' role="img" aria-label="' + esc(label) + '"' : ' aria-hidden="true"') + '>' +
      keys.map(function (k) {
        var c = { key: k, recipe: App.coverClass(k), size: '100%', cls: 'cover ' + App.coverClass(k), attrs: '', round: false, label: '', mosaic: true };
        return String(App.slot('coverArt', c, function (x) { return '<span class="' + x.cls + '">' + artImg(x.key, '100%') + '</span>'; }));
      }).join('') + '</span>');
  };

  ui.savedBadge = function () { return html`<span class="badge badge-saved">${icon('saved')}Saved</span>`; };
  ui.soulseekBadge = function () { return html`<span class="badge badge-slsk">${icon('soulseek')}Soulseek</span>`; };
  /* Saved / Soulseek badge for a track; flips in place when a save completes (App.syncRows) */
  ui.sourceBadge = function (trackOrId) {
    var id = typeof trackOrId === 'string' ? trackOrId : trackOrId.id;
    return html`<span class="src-badge" data-src-badge="${id}">${App.isSaved(id) ? ui.savedBadge() : ui.soulseekBadge()}</span>`;
  };
  /* quality chip: outlined by default; { solid: true } for player bar / device; { button: true } opens source details */
  ui.qualityChip = function (fmt, opts) {
    opts = opts || {};
    var f = M.formats[fmt] || { label: fmt, lossless: false };
    var cls = 'q-chip ' + (f.lossless ? 'q-chip--ll' : 'q-chip--ly') + (opts.solid ? ' q-chip--solid' : '');
    var label = (f.lossless ? 'Lossless, ' : '') + f.label;
    if (opts.button) return html`<button class="${cls}" data-act="source-details" ${opts.id ? raw('data-id="' + esc(opts.id) + '"') : ''} aria-label="${label}. Show source details" data-tip="Source details">${f.label}</button>`;
    return html`<span class="${cls}" title="${label}">${f.label}</span>`;
  };
  ui.newBadge = function () { return html`<span class="badge-new">new</span>`; };
  ui.countBadge = function (n, label) { return html`<span class="badge-count" ${label ? raw('aria-label="' + esc(label) + '"') : ''}>${n}</span>`; };
  ui.keycap = function (k) { return html`<kbd class="kbd">${k}</kbd>`; };
  /* ok | warn | down | off (not set up: drawn as down's shape in the quiet colour) */
  ui.statusGlyph = function (st, label) {
    var name = st === 'ok' ? 'status-ok' : st === 'warn' ? 'status-warn' : 'status-down';
    return raw(window.icon(name, label, 'sg sg--' + (st === 'ok' || st === 'warn' || st === 'off' ? st : 'down')));
  };
  ui.dupNote = function () { return html`<span class="dup-note">${icon('link')}matches a saved song</span>`; };
  ui.dupBetter = function () { return html`<span class="dup-better">${icon('arrow-up')}better than saved</span>`; };
  /* queue resolve state: ready | resolving | none | resolved (renders nothing) */
  ui.resolve = function (st) {
    if (st === 'resolving') return html`<span class="resolve"><span class="spin-ring" aria-hidden="true"></span>Finding a source</span>`;
    if (st === 'none') return html`<span class="resolve">${icon('no-source')}No source</span>`;
    if (st === 'ready') return html`<span class="resolve">${icon('check', null, 'ic-16')}ready</span>`;
    return '';
  };
  /* scrobble status: done | pending | short */
  ui.scrobble = function (st) {
    var map = { done: ['check', 'Scrobbled'], pending: ['clock', 'Scrobble pending'], short: ['dash', 'Not scrobbled: too short'] }[st] || ['dash', st];
    return html`<span class="scrobble scrobble--${st}" tabindex="0" aria-label="${map[1]}" data-tip="${map[1]}">${icon(map[0])}</span>`;
  };

  ui.tagChip = function (tagId, opts) {
    opts = opts || {}; var tag = M.tag(tagId) || { id: tagId, name: tagId };
    var on = opts.pressed != null ? opts.pressed : false;
    if (opts.toggle) {
      return html`<button class="chip" data-act="follow-tag" data-id="${tag.id}" aria-pressed="${on ? 'true' : 'false'}">${icon('check', null, 'chip__check')}${tag.name}</button>`;
    }
    return html`<a class="chip" href="#/tag/${encodeURIComponent(tag.id)}">${tag.name}</a>`;
  };
  ui.chip = function (text, opts) {
    opts = opts || {};
    if (opts.href) return html`<a class="chip" href="${opts.href}">${opts.icon ? icon(opts.icon) : ''}${text}</a>`;
    if (opts.act) return html`<button class="chip" data-act="${opts.act}" ${opts.id ? raw('data-id="' + esc(opts.id) + '"') : ''} ${opts.pressed != null ? raw('aria-pressed="' + !!opts.pressed + '"') : ''}>${opts.icon ? icon(opts.icon) : ''}${text}</button>`;
    return html`<span class="chip">${opts.icon ? icon(opts.icon) : ''}${text}</span>`;
  };
  ui.filterPill = function (text, opts) {
    opts = opts || {};
    return html`<button class="pill-filter" data-act="${opts.act || 'noop'}" ${opts.value != null ? raw('data-value="' + esc(opts.value) + '"') : ''} aria-pressed="${opts.pressed ? 'true' : 'false'}">${opts.icon ? icon(opts.icon) : ''}${text}</button>`;
  };

  /* reason chip: { kind: like|tag|listeners|fresh, name, ref }, opts { more: n, via, itemId } */
  var REASON = {
    like: ['heart-fill', 'Because you like '], tag: ['sparkle', 'Tag: '],
    listeners: ['peer', 'Popular with listeners of '], fresh: ['bell', 'Fresh from ']
  };
  ui.reasonChip = function (reason, opts) {
    opts = opts || {}; var r = REASON[reason.kind] || REASON.tag;
    var tail = reason.kind === 'fresh' ? ', an artist you follow' : '';
    var chip = html`<button class="chip-reason" data-act="reason" data-kind="${reason.kind}" data-ref="${reason.ref || ''}" data-name="${reason.name}" data-via="${opts.via || ''}" data-item="${opts.itemId || ''}" aria-haspopup="dialog">${icon(r[0])}<span>${r[1]}<span class="chip-reason__name">${reason.name}</span>${tail}</span></button>`;
    if (opts.more) return html`<span class="cluster">${chip}<button class="chip-more" data-act="reason" data-kind="${reason.kind}" data-ref="${reason.ref || ''}" data-name="${reason.name}" data-via="${opts.via || ''}" data-item="${opts.itemId || ''}" aria-label="${opts.more} more reason${opts.more > 1 ? 's' : ''}">+${opts.more}</button></span>`;
    return chip;
  };

  /* section label: lowercase text + hairline rule + optional quiet action { label, href | act } */
  ui.labelRule = function (text, opts) {
    opts = opts || {}; var tag = opts.level || 'h2';
    var action = opts.action ? (opts.action.href
      ? html`<a class="btn btn--quiet btn--sm" href="${opts.action.href}">${opts.action.label}</a>`
      : html`<button class="btn btn--quiet btn--sm" data-act="${opts.action.act}">${opts.action.label}</button>`) : '';
    return html`<div class="label-rule${opts.sm ? ' label-rule--sm' : ''}">${raw('<' + tag + (opts.id ? ' id="' + esc(opts.id) + '"' : '') + '>')}${text}${raw('</' + tag + '>')}${action}</div>`;
  };
  ui.iconButton = function (name, label, attrs) {
    attrs = attrs || {};
    var extra = Object.keys(attrs).filter(function (k) { return k !== 'cls'; })
      .map(function (k) { return ' ' + k + '="' + esc(attrs[k]) + '"'; }).join('');
    return raw('<button class="btn-icon' + (attrs.cls ? ' ' + attrs.cls : '') + '" aria-label="' + esc(label) + '" data-tip="' + esc(label) + '"' + extra + '>' + window.icon(name) + '</button>');
  };
  ui.likeButton = function (id, cls) {
    var on = App.isLiked(id), t = M.track(id);
    return html`<button class="btn-icon btn-icon--bare ${on ? 'is-liked' : ''} ${cls || ''}" data-act="like" data-id="${id}" data-like="${id}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Like ${t ? t.title : ''}" data-tip="${on ? 'Liked' : 'Like'}">${icon(on ? 'heart-fill' : 'heart')}</button>`;
  };

  /* Track row. opts: { index, dense, added (ISO), draggable (default true), showSources, dup ('saved'|'better'),
     queueIndex, actions (default true), unavailable, badges (markup placed before the source badge) } */
  ui.trackRow = function (track, opts) {
    opts = opts || {}; if (typeof track === 'string') track = M.track(track);
    if (!track) return '';
    var saved = App.isSaved(track.id);
    var playing = App.player && App.player.currentId() === track.id;
    var cls = 'trow' + (opts.dense ? ' trow--dense' : '') + (playing ? ' is-playing' : '') + (opts.unavailable ? ' is-unavailable' : '');
    var drag = opts.draggable === false ? '' : ' draggable="true"';
    var n = opts.index != null ? opts.index : track.no;
    /* file facts as data (data-khz / data-kbps / data-bits / data-size in MB) + a hidden .trow__fmt line a design may show */
    var facts = raw(' data-khz="' + (track.khz || '') + '" data-kbps="' + (track.kbps || '') + '" data-bits="' + (track.bits || '') + '" data-size="' + (track.sizeMB || '') + '"');
    return html`<li class="${cls}" data-track-id="${track.id}" ${opts.queueIndex != null ? raw('data-queue-index="' + opts.queueIndex + '"') : ''}${facts} tabindex="0"${raw(drag)} aria-label="${track.title} by ${track.artist}">
      <div class="trow__num"><span class="trow__n">${n}</span><span class="trow__grip" aria-hidden="true">${icon('grip', null, 'ic-16')}</span><span class="eq" aria-hidden="true"><i></i><i></i><i></i></span></div>
      <div class="trow__art">${ui.cover(track, opts.dense ? 40 : 48)}</div>
      <div class="trow__main">
        <button class="trow__title" data-act="play-track" data-id="${track.id}" tabindex="-1">${track.title}</button>
        <div class="trow__artist"><span class="trow__mini" data-src-mini="${track.id}">${icon(saved ? 'saved' : 'soulseek', saved ? 'Saved' : 'Soulseek')}</span><a href="#/artist/${track.artistId}" tabindex="-1">${track.artist}</a></div>
        <div class="trow__fmt" aria-hidden="true">${formatLine(track)}</div>
      </div>
      <div class="trow__album"><a href="#/album/${track.albumId}" tabindex="-1">${track.album}</a></div>
      ${opts.added ? html`<div class="trow__added">${fmtDate(opts.added, true)}</div>` : ''}
      <div class="trow__badges">${opts.badges || ''}${opts.dup === 'better' ? ui.dupBetter() : opts.dup ? ui.dupNote() : ''}${ui.sourceBadge(track)}${track.format ? ui.qualityChip(track.format) : ''}${opts.showSources && sourceCount(track) ? html`<button class="trow__sources" data-act="show-sources" data-id="${track.id}" tabindex="-1" aria-label="${sourceCount(track)} sources for ${track.title}. Pick a source" data-tip="Pick a source">${sourceCount(track)} sources</button>` : ''}</div>
      <div class="trow__like">${ui.likeButton(track.id)}</div>
      <div class="trow__dur">${fmtTime(track.duration)}</div>
      <div class="trow__more">${opts.actions === false ? '' : html`<button class="btn-icon btn-icon--bare" data-act="track-menu" data-id="${track.id}" aria-haspopup="menu" aria-label="More actions for ${track.title}" data-tip="More">${icon('more')}</button>`}</div>
    </li>`;
  };
  /* how many peers offer a track: a search result's count, or the length of a fetched source list */
  function sourceCount(t) { return typeof t.sources === 'number' ? t.sources : t.sources ? t.sources.length : 0; }
  /* list wrapper: data-ids gives play-track its context (the rest of the list becomes the queue) */
  ui.trackList = function (tracks, opts) {
    opts = opts || {};
    tracks = tracks.map(function (t) { return typeof t === 'string' ? M.track(t) : t; }).filter(Boolean);
    return html`<ol class="tlist${opts.dense ? ' tlist--dense' : ''}${opts.dated ? ' tlist--dated' : ''}" data-ids="${tracks.map(function (t) { return t.id; }).join(',')}" ${opts.label ? raw('aria-label="' + esc(opts.label) + '"') : ''}>${tracks.map(function (t, i) {
      return ui.trackRow(t, Object.assign({}, opts, { index: opts.numbered === false ? t.no : i + 1, added: opts.added ? opts.added[i] : null }));
    })}</ol>`;
  };

  /* release tile (shelves): art + flat round play + title + artist + new chip + pixel-face "album, Oct 29" */
  ui.releaseTile = function (album, opts) {
    opts = opts || {}; if (typeof album === 'string') album = M.album(album);
    return html`<div class="tile" data-album-id="${album.id}">
      <div class="tile__art"><a href="#/album/${album.id}" aria-label="${album.title} by ${album.artist}">${ui.cover(album, '100%')}</a>
        <button class="tile__play" data-act="play-album" data-id="${album.id}" aria-label="Play ${album.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/album/${album.id}">${album.title}</a></div>
      <div class="tile__by"><a href="#/artist/${album.artistId}">${album.artist}</a></div>
      <div class="tile__meta">${album.isNew && opts.showNew !== false ? ui.newBadge() : ''}<span>${album.type.toLowerCase()}${album.released ? ', ' + fmtDate(album.released) : ''}</span></div>
    </div>`;
  };

  /* seek bar bound to the player: every [data-seek="player"] updates live (pointer, keys, aria) */
  ui.seekBar = function (opts) {
    opts = opts || {};
    return html`<div class="seek ${opts.cls || ''}" data-seek="${opts.bind || 'player'}" role="slider" tabindex="0" aria-label="${opts.label || 'Seek'}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0" aria-valuetext="">
      <div class="seek__track"><div class="seek__xfer"></div><div class="seek__bufs"></div><div class="seek__played"></div></div>
      <div class="seek__rail"><div class="seek__knob"></div></div>
      <div class="seek__tip" aria-hidden="true"></div>
    </div>`;
  };
  ui.volume = function () {
    return html`<div class="vol">${ui.iconButton('volume-2', 'Mute', { 'data-act': 'mute', 'data-bind': 'mute', cls: 'btn-icon--bare' })}${ui.seekBar({ bind: 'volume', label: 'Volume' })}</div>`;
  };
  /* waveform strip for the device screen; bound to the player like the seek bar */
  ui.wave = function (trackId, bars) {
    bars = bars || 56; var h = hash(trackId || 'x'), out = [];
    for (var i = 0; i < bars; i++) {
      h = (h * 1103515245 + 12345) & 0x7fffffff;
      var env = Math.sin(Math.PI * (i + .5) / bars) * .55 + .35;
      out.push('<i class="wave__bar" style="--h:' + Math.max(.12, Math.min(1, env * (.55 + (h % 100) / 180))).toFixed(2) + '"></i>');
    }
    return html`<div class="wave" data-seek="player" role="slider" tabindex="0" aria-label="Seek" aria-valuemin="0" aria-valuemax="100">${raw(out.join(''))}<div class="wave__rail"><i class="wave__head"></i></div></div>`;
  };
  ui.waveTimes = function () {
    return html`<div class="wave-times"><b data-bind="pos">0:00</b><span>buffered to <span data-bind="buffered-to">0:00</span></span><span data-bind="dur">0:00</span></div>`;
  };

  /* save button: state from the save pipeline; re-rendered in place by App.syncRows */
  ui.saveButton = function (trackId, opts) {
    opts = opts || {};
    var s = state.saves[trackId], st = App.isSaved(trackId) ? (s ? 'saved' : 'library') : s ? s.status : 'idle';
    var label, ic, attrs = '';
    if (st === 'saving') { label = html`Saving <span class="px">${s.step}/5</span>`; ic = 'save'; attrs = ' style="--p:' + (s.step / 5).toFixed(2) + '" aria-live="polite"'; }
    else if (st === 'saved') { label = 'Saved'; ic = 'saved'; }
    else if (st === 'library') { label = 'In library'; ic = 'saved'; }
    else if (st === 'failed') { label = 'Retry'; ic = 'warning'; }
    else if (st === 'pending') { label = 'Saved, waiting for Jellyfin'; ic = 'clock'; }
    else { label = opts.label || 'Save'; ic = 'save'; }
    var act = st === 'library' ? 'go-album' : st === 'saved' ? 'noop-saved' : 'save';
    return html`<button class="btn btn-save ${opts.cls || ''}" data-state="${st}" data-save-btn="${trackId}" data-act="${act}" data-id="${trackId}"${raw(attrs)}>${icon(ic)}<span>${label}</span></button>`;
  };
  /* 5-step pipeline. step = current step (1-5); status saving|failed|saved */
  ui.pipeline = function (step, status, error) {
    var segs = [];
    for (var i = 1; i <= 5; i++) {
      var c = status === 'saved' || i < step ? 'is-done' : i === step ? (status === 'failed' ? 'is-failed' : 'is-current') : '';
      segs.push(html`<span class="pipeline__seg ${c}" aria-hidden="true">${c === 'is-done' ? icon('check') : c === 'is-failed' ? icon('close') : ''}</span>`);
    }
    var text = status === 'saved' ? 'Saved to your library' : status === 'failed' ? (error || 'Step failed: ' + M.saveSteps[step - 1]) : M.saveSteps[step - 1];
    return html`<div class="pipeline-wrap" role="group" aria-label="Save progress"><div class="pipeline">${segs}</div>
      <div class="pipeline__text" aria-live="polite"><span class="pipeline__step">${status === 'saved' ? 'done' : 'step ' + step + ' of 5'}</span><span class="pipeline__label">${text}</span></div></div>`;
  };

  var EMPTY_ART = {
    moon: '<svg viewBox="0 0 112 88"><path d="M70 14a30 30 0 1 0 22 44 26 26 0 1 1-22-44z"/><path d="M18 70h20M24 78h10" /></svg>',
    slot: '<svg viewBox="0 0 112 88"><rect x="20" y="16" width="72" height="56" rx="10"/><path d="M36 16v14h40V16M40 52h32M40 60h20"/></svg>',
    star: '<svg viewBox="0 0 112 88"><path d="M56 10l9 20 22 3-16 15 4 22-19-10-19 10 4-22-16-15 22-3z"/><path d="M56 30l-4 12 6 4-5 12"/></svg>',
    wave: '<svg viewBox="0 0 112 88"><path d="M16 44h8M30 34v20M40 26v36M50 38v12M60 22v44M70 32v24M80 40v8M90 44h6"/></svg>'
  };
  /* default emptyArt slot: line drawing + one sparkle */
  function emptyArtMarkup(c) { return (EMPTY_ART[c.art] || EMPTY_ART.moon) + '<span class="sparkle"></span>'; }
  /* empty / error state: { art: moon|slot|star|wave, title, text, action: { label, href | act }, secondary } */
  ui.emptyState = function (o) {
    function btn(a, primary) {
      if (!a) return '';
      return a.href ? html`<a class="btn ${primary ? 'btn--primary' : ''}" href="${a.href}">${a.label}</a>`
        : html`<button class="btn ${primary ? 'btn--primary' : ''}" data-act="${a.act}" ${a.id ? raw('data-id="' + esc(a.id) + '"') : ''}>${a.label}</button>`;
    }
    return html`<div class="empty">
      <div class="empty__art" aria-hidden="true">${raw(String(App.slot('emptyArt', { art: o.art || 'moon', title: o.title }, emptyArtMarkup)))}</div>
      <h3 class="empty__title">${o.title}</h3>
      ${o.text ? html`<p class="empty__text">${o.text}</p>` : ''}
      ${o.action || o.secondary ? html`<div class="empty__actions">${btn(o.action, true)}${btn(o.secondary)}</div>` : ''}
    </div>`;
  };
  /* skeleton(kind, n): kind = row | tile | chip | text | card */
  ui.skeleton = function (kind, n) {
    n = n || 1; var out = [];
    for (var i = 0; i < n; i++) {
      if (kind === 'row') out.push('<div class="skel-row"><div class="skel skel--cover"></div><div><div class="skel skel--title" style="width:' + (50 + (i * 17) % 35) + '%"></div><div class="skel skel--text" style="width:' + (30 + (i * 11) % 25) + '%"></div></div><div class="skel skel--text"></div></div>');
      else if (kind === 'tile') out.push('<div class="skel-tile"><div class="skel skel--cover"></div><div class="skel skel--title" style="width:80%"></div><div class="skel skel--text" style="width:55%"></div></div>');
      else if (kind === 'chip') out.push('<span class="skel skel--chip" style="display:inline-block;width:' + (70 + (i * 23) % 50) + 'px"></span>');
      else if (kind === 'card') out.push('<div class="card" aria-hidden="true"><div class="skel skel--title" style="width:30%;margin-bottom:16px"></div>' + ui.skeleton('row', 3) + '</div>');
      else out.push('<div class="skel skel--text" style="width:' + (60 + (i * 13) % 35) + '%;margin:8px 0"></div>');
    }
    return raw('<div class="skel-group" aria-busy="true" aria-label="Loading">' + out.join('') + '</div>');
  };
  ui.sparkle = function (style) { return raw('<span class="sparkle" aria-hidden="true" style="' + esc(style || '') + '"></span>'); };
  /* the plain wordmark (device bezel); with `where`, the design's brand slot (token gate: 'desktop').
     Named "lull" for assistive tech: role=img + label around the decorative markup. */
  ui.wordmark = function (where) {
    var inner = where ? String(App.slot('brand', { where: where }, Designs.brandMarkup)) : Designs.brandMarkup();
    return raw('<span class="brand-mark" role="img" aria-label="' + esc(App.product) + '">' + inner + '</span>');
  };
  /* brand slot: the wordmark in the rail head and the phone / topnav / dock header ({ where: 'rail'|'header' }).
     Default (Designs.brandMarkup): .wordmark.brand > .brand__name "lull" + .brand__orn sparkle; the link has the name. */
  ui.brand = function (where) {
    return raw(String(App.slot('brand', { where: where }, Designs.brandMarkup)));
  };

  /* Card heads: every .card gets a .card-head whose .card-head__title names it, so a design can draw a
     title bar. A leading .label-rule becomes the head; a card without one gets a hidden head
     (.card-head--hidden, aria-hidden) titled from its aria-label / aria-labelledby / data-title.
     A MutationObserver (boot) runs this on the view, the context panel and overlays, so screens that
     repaint a card's insides keep their head without doing anything. */
  /* a card's data-meta="..." becomes .card-head__meta (aria-hidden, hidden in the base look) inside its head */
  function cardMeta(card, head) {
    var text = card.getAttribute('data-meta'), el = null;
    for (var c = head.firstElementChild; c; c = c.nextElementSibling) if (c.classList.contains('card-head__meta')) el = c;
    if (!text) { if (el) el.remove(); return; }
    if (!el) {
      el = document.createElement('span'); el.className = 'card-head__meta'; el.setAttribute('aria-hidden', 'true');
      var title = head.querySelector('.card-head__title');
      head.insertBefore(el, title && title.parentNode === head ? title.nextSibling : head.firstChild);
    }
    if (el.textContent !== text) el.textContent = text;
  }
  ui.cardHeads = function (scope) {
    $$('.card', scope).forEach(function (card) {
      var first = card.firstElementChild;
      if (first && first.classList.contains('card-head')) { cardMeta(card, first); return; }
      if (first && first.classList.contains('label-rule')) {
        first.classList.add('card-head');
        var h = first.querySelector('h1, h2, h3, .label-rule__text'); if (h) h.classList.add('card-head__title');
        card.setAttribute('data-head', 'rule');
        cardMeta(card, first);
        return;
      }
      var by = card.getAttribute('aria-labelledby'), lab = by && document.getElementById(by);
      var title = card.getAttribute('aria-label') || (lab ? lab.textContent : '') || card.getAttribute('data-title') || '';
      if (!title) return;
      var head = document.createElement('div');
      head.className = 'card-head card-head--hidden'; head.setAttribute('aria-hidden', 'true');
      head.innerHTML = '<span class="card-head__title">' + esc(title.trim().toLowerCase()) + '</span>';
      card.insertBefore(head, first);
      card.setAttribute('data-head', 'hidden');
      cardMeta(card, head);
    });
  };

  /* =====================================================================
     6. Overlays: sheet, dialog (focus trap, Esc, return focus), popover, menu, tooltip, toast, banner
     ===================================================================== */
  var layers = [];   // open sheets/dialogs, topmost last
  /* run fn after the next paint; the timeout covers environments where rAF never fires (headless screenshots) */
  function afterPaint(fn) {
    var done = false; function go() { if (!done) { done = true; fn(); } }
    requestAnimationFrame(function () { requestAnimationFrame(go); }); setTimeout(go, 80);
  }
  var FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select, textarea, [tabindex]:not([tabindex="-1"])';
  function focusables(el) { return $$(FOCUSABLE, el).filter(function (n) { return n.offsetParent !== null || n === document.activeElement; }); }

  /* App.openSheet({ title, body, foot, wide, className, label, onMount(el, close), onClose })
     App.openDialog(same) -> { el, close }. Dialogs never stack: a new dialog replaces the open one. */
  function openLayer(kind, o) {
    o = o || {};
    if (kind === 'dialog') layers.filter(function (l) { return l.kind === 'dialog'; }).forEach(function (l) { l.close(true); });
    var opener = document.activeElement;
    var host = $('#overlay-host');
    var scrim = document.createElement('div'); scrim.className = 'scrim';
    var el = document.createElement('section');
    var titleId = nextId('lt');
    el.className = (kind === 'sheet' ? 'sheet' : 'dialog') + ' s-card' + (o.wide ? (kind === 'sheet' ? ' sheet--wide' : ' dialog--wide') : '') + (o.className ? ' ' + o.className : '');
    el.setAttribute('role', o.role || 'dialog'); el.setAttribute('aria-modal', 'true');
    if (o.title) el.setAttribute('aria-labelledby', titleId); else if (o.label) el.setAttribute('aria-label', o.label);
    el.innerHTML = String(html`${kind === 'sheet' ? raw('<div class="sheet__handle" aria-hidden="true"></div>') : ''}
      ${o.title || o.closable !== false ? html`<header class="sheet__head"><h2 class="sheet__title" id="${titleId}">${o.title || ''}</h2>${o.closable === false ? '' : html`<button class="btn-icon btn-icon--bare" data-close aria-label="Close" data-tip="Close (Esc)">${icon('close')}</button>`}</header>` : ''}
      <div class="sheet__body">${o.body || ''}</div>
      ${o.foot ? html`<footer class="sheet__foot">${o.foot}</footer>` : ''}`);
    host.appendChild(scrim); host.appendChild(el);
    var layer = { kind: kind, el: el, scrim: scrim, opener: opener, onClose: o.onClose, dismissible: o.dismissible !== false };
    layer.close = function (silent) {
      if (layer.closed) return; layer.closed = true;
      layers = layers.filter(function (l) { return l !== layer; });
      el.classList.remove('is-open'); scrim.classList.remove('is-open');
      setTimeout(function () { el.remove(); scrim.remove(); }, App.reducedMotion() ? 130 : 260);
      if (!silent && opener && opener.focus && document.contains(opener)) opener.focus();
      if (layer.onClose) layer.onClose();
    };
    layers.push(layer);
    scrim.addEventListener('click', function () { if (o.dismissible !== false) layer.close(); });
    el.addEventListener('click', function (e) { if (e.target.closest('[data-close]')) layer.close(); });
    if (kind === 'sheet') enableDragDismiss(el, layer);
    afterPaint(function () { if (!layer.closed) { scrim.classList.add('is-open'); el.classList.add('is-open'); } });
    if (o.onMount) o.onMount(el, layer.close);
    var first = $('[autofocus]', el) || focusables($('.sheet__body', el))[0] || $('[data-close]', el) || el;
    if (first === el) el.setAttribute('tabindex', '-1');
    setTimeout(function () { first.focus(); }, 30);
    return { el: el, close: function () { layer.close(); } };
  }
  App.openSheet = function (o) { return openLayer('sheet', o); };
  App.openDialog = function (o) { return openLayer('dialog', o); };
  App.closeTop = function () { var l = layers[layers.length - 1]; if (l) { l.close(); return true; } return false; };
  /* Esc: the top layer unless it must stay (the token gate); true when Esc was used up */
  function escapeTop() { var l = layers[layers.length - 1]; if (!l) return false; if (l.dismissible) l.close(); return true; }
  /* confirm dialog: App.confirm({ title, text, confirm: 'Delete', danger: true }).then-style callback */
  App.confirm = function (o, onYes) {
    var d = App.openDialog({ title: o.title, body: html`<p class="prose">${o.text}</p>`,
      foot: html`<button class="btn" data-close>Cancel</button><button class="btn ${o.danger ? 'btn--danger' : 'btn--primary'}" data-yes>${o.confirm || 'OK'}</button>` });
    $('[data-yes]', d.el).addEventListener('click', function () { d.close(); if (onYes) onYes(); });
    return d;
  };
  function enableDragDismiss(el, layer) {
    var startY = null, dy = 0;
    el.addEventListener('pointerdown', function (e) {
      if (!e.target.closest('.sheet__handle, .sheet__head') || e.target.closest('button')) return;
      startY = e.clientY; dy = 0; el.setPointerCapture(e.pointerId); el.style.transition = 'none';
    });
    el.addEventListener('pointermove', function (e) {
      if (startY === null) return; dy = Math.max(0, e.clientY - startY);
      el.style.transform = 'translate(-50%, ' + dy + 'px)';
    });
    function end() {
      if (startY === null) return; startY = null; el.style.transition = ''; el.style.transform = '';
      if (dy > 110) layer.close();
    }
    el.addEventListener('pointerup', end); el.addEventListener('pointercancel', end);
  }
  // focus trap for the topmost layer
  document.addEventListener('keydown', function (e) {
    if (e.key !== 'Tab' || !layers.length) return;
    var el = layers[layers.length - 1].el, f = focusables(el);
    if (!f.length) { e.preventDefault(); return; }
    if (e.shiftKey && document.activeElement === f[0]) { e.preventDefault(); f[f.length - 1].focus(); }
    else if (!e.shiftKey && document.activeElement === f[f.length - 1]) { e.preventDefault(); f[0].focus(); }
    else if (!el.contains(document.activeElement)) { e.preventDefault(); f[0].focus(); }
  });

  /* popover anchored to an element: App.popover(anchor, markup, { label, className, onMount(el, close), role }) */
  var openPop = null;
  function placeNear(el, anchor) {
    var r = anchor.getBoundingClientRect(), w = el.offsetWidth, h = el.offsetHeight;
    var left = clamp(r.left + r.width / 2 - w / 2, 12, window.innerWidth - w - 12);
    var top = r.bottom + 10;
    if (top + h > window.innerHeight - 12) top = Math.max(12, r.top - h - 14);
    el.style.left = left + 'px'; el.style.top = top + 'px';
  }
  App.closePopover = function (returnFocus) {
    if (!openPop) return false;
    var p = openPop; openPop = null; p.el.remove();
    if (p.anchor) p.anchor.setAttribute('aria-expanded', 'false');
    if (returnFocus && p.anchor && document.contains(p.anchor)) p.anchor.focus();
    if (p.onClose) p.onClose();
    return true;
  };
  App.popover = function (anchor, markup, o) {
    o = o || {};
    if (openPop && openPop.anchor === anchor) { App.closePopover(true); return null; }
    App.closePopover();
    var el = document.createElement('div');
    el.className = (o.menu ? 'menu' : 'popover') + ' s-card' + (o.className ? ' ' + o.className : '');
    el.setAttribute('role', o.role || (o.menu ? 'menu' : 'dialog'));
    if (o.label) el.setAttribute('aria-label', o.label);
    el.innerHTML = String(markup);
    document.body.appendChild(el);
    placeNear(el, anchor);
    anchor.setAttribute('aria-expanded', 'true');
    openPop = { el: el, anchor: anchor, onClose: o.onClose };
    afterPaint(function () { el.classList.add('is-open'); });
    var close = function () { App.closePopover(true); };
    if (o.onMount) o.onMount(el, close);
    var f = focusables(el)[0]; if (f) f.focus();
    return { el: el, close: close };
  };
  /* menu: App.menu(anchor, [{ label, icon, run, danger } | 'sep'], { label }) with arrow-key navigation */
  App.menu = function (anchor, items, o) {
    o = o || {};
    var markup = items.map(function (it, i) {
      if (it === 'sep') return '<div class="menu__sep" role="separator"></div>';
      return String(html`<button class="menu__item ${it.danger ? 'menu__item--danger' : ''}" role="menuitem" data-i="${i}">${it.icon ? icon(it.icon) : ''}<span>${it.label}</span></button>`);
    }).join('');
    return App.popover(anchor, markup, { menu: true, label: o.label, onMount: function (el, close) {
      el.addEventListener('click', function (e) {
        var b = e.target.closest('[data-i]'); if (!b) return;
        var it = items[+b.getAttribute('data-i')]; App.closePopover(true); if (it.run) it.run();
      });
      el.addEventListener('keydown', function (e) {
        var f = $$('.menu__item', el), i = f.indexOf(document.activeElement);
        if (e.key === 'ArrowDown') { e.preventDefault(); f[(i + 1) % f.length].focus(); }
        else if (e.key === 'ArrowUp') { e.preventDefault(); f[(i - 1 + f.length) % f.length].focus(); }
        else if (e.key === 'Home') { e.preventDefault(); f[0].focus(); }
        else if (e.key === 'End') { e.preventDefault(); f[f.length - 1].focus(); }
        else if (e.key === 'Tab') { App.closePopover(); }
      });
    } });
  };
  document.addEventListener('pointerdown', function (e) {
    if (openPop && !openPop.el.contains(e.target) && !openPop.anchor.contains(e.target)) App.closePopover();
  });
  var lastW = window.innerWidth;              // close on real width changes only (not the phone keyboard)
  window.addEventListener('resize', function () { if (window.innerWidth !== lastW) { lastW = window.innerWidth; App.closePopover(); } });

  /* tooltip: any element with data-tip (icon-only buttons always have one); dark cream-on-ink */
  var tipEl = null, tipTimer = null, tipFor = null;
  function showTip(target) {
    var text = target.getAttribute('data-tip'); if (!text) return;
    var lbl = target.querySelector('.nav-item__label'); if (lbl && lbl.offsetParent) return;   // rail labels visible: no tooltip
    if (!tipEl) { tipEl = document.createElement('div'); tipEl.className = 'tooltip'; tipEl.setAttribute('role', 'tooltip'); tipEl.id = 'tooltip'; document.body.appendChild(tipEl); }
    tipEl.textContent = text; tipFor = target;
    var r = target.getBoundingClientRect();
    tipEl.style.left = '0px'; tipEl.style.top = '0px'; tipEl.classList.add('is-on');
    var w = tipEl.offsetWidth, h = tipEl.offsetHeight;
    var top = r.top - h - 8; if (top < 8) top = r.bottom + 8;
    tipEl.style.left = clamp(r.left + r.width / 2 - w / 2, 8, window.innerWidth - w - 8) + 'px';
    tipEl.style.top = top + 'px';
  }
  function hideTip() { clearTimeout(tipTimer); tipFor = null; if (tipEl) tipEl.classList.remove('is-on'); }
  document.addEventListener('mouseover', function (e) {
    var t = e.target.closest && e.target.closest('[data-tip]');
    if (t === tipFor) return; hideTip();
    if (t) tipTimer = setTimeout(function () { showTip(t); }, 450);
  });
  document.addEventListener('focusin', function (e) {
    var t = e.target.closest && e.target.closest('[data-tip]'); hideTip();
    if (t && t.matches(':focus-visible')) showTip(t);
  });
  document.addEventListener('focusout', hideTip);
  document.addEventListener('pointerdown', hideTip);
  window.addEventListener('scroll', hideTip, true);

  /* toast(message, { icon, action, onAction, kind: info|ok|error, sticky }) -> { close }.
     Auto-dismiss 6 s (paused on hover/focus); errors with an action stay until dismissed. */
  App.toast = function (msg, o) {
    o = o || {};
    var region = $('#toasts');
    var el = document.createElement('div');
    el.className = 'toast s-card' + (o.kind ? ' toast--' + o.kind : '');
    if (o.kind === 'error') el.setAttribute('role', 'alert');
    el.innerHTML = String(html`${icon(o.icon || (o.kind === 'error' ? 'warning' : o.kind === 'ok' ? 'check' : 'info'))}<span class="toast__msg">${msg}</span>
      ${o.action ? html`<button class="btn btn--quiet btn--sm" data-toast-action>${o.action}</button>` : ''}
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-toast-close aria-label="Dismiss">${icon('close', null, 'ic-16')}</button>`);
    region.appendChild(el);
    while (region.children.length > 3) region.firstChild.remove();
    requestAnimationFrame(function () { el.classList.add('is-open'); });
    var timer = null, paused = false;
    function close() { clearTimeout(timer); el.classList.remove('is-open'); setTimeout(function () { el.remove(); }, 180); }
    function arm() { clearTimeout(timer); if (!paused && !(o.sticky || (o.kind === 'error' && o.action))) timer = setTimeout(close, 6000); }
    el.addEventListener('mouseenter', function () { paused = true; clearTimeout(timer); });
    el.addEventListener('mouseleave', function () { paused = false; arm(); });
    el.addEventListener('focusin', function () { paused = true; clearTimeout(timer); });
    el.addEventListener('focusout', function () { paused = false; arm(); });
    el.addEventListener('click', function (e) {
      if (e.target.closest('[data-toast-action]')) { close(); if (o.onAction) o.onAction(); }
      else if (e.target.closest('[data-toast-close]')) close();
    });
    arm();
    return { el: el, close: close };
  };

  /* banners (degraded / offline): App.banner.show({ id, kind: warn|down|info|ok, icon, text, action: { label, run }, dismissible })
     App.banner.hide(id). Never modal. "server unreachable" passes dismissible: false. */
  var banners = [];
  App.banner = {
    show: function (b) { banners = banners.filter(function (x) { return x.id !== b.id; }); banners.push(b); renderBanners(); },
    hide: function (id) { banners = banners.filter(function (x) { return x.id !== id; }); renderBanners(); },
    clear: function () { banners = []; renderBanners(); },
    list: function () { return banners.slice(); }
  };
  /* phones: two or more banners collapse to the most severe one (.is-lead) plus a toggle row (components.css) */
  var SEVERITY = { down: 0, warn: 1, info: 2, ok: 3 }, bannersOpen = false;
  function severity(b) { return b.kind in SEVERITY ? SEVERITY[b.kind] : 2; }
  function renderBanners() {
    var slot = $('#banners'); if (!slot) return;
    var lead = banners.reduce(function (best, b) { return !best || severity(b) < severity(best) ? b : best; }, null);
    if (banners.length < 2) bannersOpen = false;
    slot.classList.toggle('is-open', bannersOpen);
    slot.innerHTML = banners.map(function (b) {
      var ic = b.icon || (b.kind === 'down' ? 'status-down' : b.kind === 'warn' ? 'warning' : b.kind === 'ok' ? 'check' : 'info');
      return String(html`<div class="banner s-card banner--${b.kind || 'info'}${b === lead ? ' is-lead' : ''}" role="${b.kind === 'down' ? 'alert' : 'status'}" data-banner="${b.id}">
        ${icon(ic)}<span class="banner__msg">${b.text}</span>
        ${b.action ? html`<button class="btn btn--sm" data-banner-action="${b.id}">${b.action.label}</button>` : ''}
        ${b.dismissible === false ? '' : html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-banner-close="${b.id}" aria-label="Dismiss" data-tip="Dismiss">${icon('close', null, 'ic-16')}</button>`}
      </div>`);
    }).join('') + (banners.length > 1 ? String(html`<button class="banner-more s-card" data-banner-toggle aria-expanded="${bannersOpen ? 'true' : 'false'}">
        ${icon('chevron-down')}<span class="banner-more__msg">${bannerToggleText()}</span></button>`) : '');
  }
  function bannerToggleText() { return bannersOpen ? 'Show fewer notices' : (banners.length - 1) + ' more ' + (banners.length > 2 ? 'notices' : 'notice'); }
  document.addEventListener('click', function (e) {
    var t = e.target.closest('[data-banner-toggle]');
    if (t) {                                   // in place, so live regions are not re-announced and focus stays
      bannersOpen = !bannersOpen;
      $('#banners').classList.toggle('is-open', bannersOpen);
      t.setAttribute('aria-expanded', bannersOpen ? 'true' : 'false');
      $('.banner-more__msg', t).textContent = bannerToggleText();
    }
    var a = e.target.closest('[data-banner-action]'), c = e.target.closest('[data-banner-close]');
    if (a) { var b = banners.filter(function (x) { return x.id === a.getAttribute('data-banner-action'); })[0]; if (b && b.action.run) b.action.run(); }
    if (c) App.banner.hide(c.getAttribute('data-banner-close'));
  });

  /* =====================================================================
     7. Save pipeline: App.saves.start(ids), retry(id), get(id), set(id, record)
        POST /api/saves starts one job per track; `save` events carry each step (docs/context/api.md).
     ===================================================================== */
  App.hooks = {
    /* screens may replace any hook; defaults keep every control working */
    beforeSave: function (ids, proceed) { proceed(ids); },
    sourcePicker: function (trackId) { App.go('/now?tab=details&track=' + encodeURIComponent(trackId)); },
    sourceDetails: function () { App.go('/now?tab=details'); },
    reason: function (anchor, r) { defaultReasonPopover(anchor, r); },
    addToPlaylist: function (ids, anchor) { defaultAddToPlaylist(ids, anchor); },
    shortcuts: function () { defaultShortcutsDialog(); },
    palette: function () { var s = $('#header-search'); if (s && s.offsetParent) s.focus(); else App.go('/search'); },
    importFile: function (file) { App.toast('Only M3U playlists can be dropped here, not ' + file.name, { kind: 'error' }); },
    moreSheet: function () { defaultMoreSheet(); },
    /* media keys / lock screen target: null = App.player; Classic mode sets { play, pause, prev, next, seek, seekBy } */
    media: null
  };
  var SAVE_STATUS = { running: 'saving', done: 'saved', failed: 'failed', pending: 'pending' };
  App.saves = {
    get: function (id) { return state.saves[id]; },
    /* opts: { mode: 'separate'|'replace' (duplicate dialog: a saved song may be saved again), source: { trackId: { peer, path } } } */
    start: function (ids, opts) {
      opts = opts || {};
      ids = [].concat(ids).filter(function (id) { return (opts.mode || !App.isSaved(id)) && !(state.saves[id] && state.saves[id].status === 'saving'); });
      if (!ids.length) { App.toast('Already in your library', { icon: 'saved' }); return; }
      App.hooks.beforeSave(ids, function (ok, o) { startSaves(ok || ids, Object.assign({}, opts, o)); });
    },
    retry: function (id) {
      var s = state.saves[id];
      if (!s || !s.jobId) { App.saves.start([id]); return; }
      setSave(id, Object.assign({}, s, { status: 'saving', error: null }));
      M.api('/api/saves/' + encodeURIComponent(s.jobId) + '/retry', { method: 'POST' }).catch(function (err) { saveFailed([id], err); });
    },
    set: function (id, rec) { setSave(id, rec); }
  };
  function startSaves(ids, opts) {
    if (!ids.length) return;
    ids.forEach(function (id) { setSave(id, { step: 1, status: 'saving', pct: 0 }); });
    if (ids.length > 1) App.toast('Saving ' + ids.length + ' songs to your library', { icon: 'save' });
    /* the playing song is saved from the peer it streams from, so the transfer is not started twice */
    var source = Object.assign({}, opts.source);
    if (ps.source && ids.indexOf(P.currentId()) >= 0 && !source[P.currentId()]) source[P.currentId()] = { peer: ps.source.peer, path: ps.source.path };
    var body = { ids: ids };
    if (Object.keys(source).length) body.source = source;
    if (opts.mode) body.mode = opts.mode;
    M.api('/api/saves', { method: 'POST', body: body }).then(function (res) {
      (res.jobs || []).forEach(function (jobId, i) {
        var id = ids[i]; if (id && state.saves[id]) setSave(id, Object.assign({}, state.saves[id], { jobId: jobId }));
      });
    }, function (err) { saveFailed(ids, err); });
  }
  function saveFailed(ids, err) {
    ids.forEach(function (id) { setSave(id, Object.assign({}, state.saves[id], { status: 'failed', error: err.message })); });
    App.toast('Couldn\'t save: ' + err.message, { kind: 'error' });
  }
  function setSave(id, rec) {
    App.store.update('saves', function (s) { if (rec) s[id] = rec; else delete s[id]; });
    App.emit('save', { id: id, save: rec });
    App.syncRows(); updateActivityBadge();
  }
  /* `save` event: { jobId, trackId, step, status: running|done|failed|pending, pct, error, kind, nocover } */
  function onSaveEvent(e) {
    var was = state.saves[e.trackId], status = SAVE_STATUS[e.status] || 'saving';
    setSave(e.trackId, { jobId: e.jobId, step: e.step, status: status, pct: e.pct || 0, error: e.error || null, kind: e.kind || null, nocover: !!e.nocover });
    if (status !== 'saved' || (was && was.status === 'saved')) return;
    M.patchTrack(e.trackId, { saved: true });
    var t = M.track(e.trackId);
    App.toast('Saved to library: ' + (t ? t.title : 'one song'), { kind: 'ok', icon: 'saved', action: t ? 'Show' : null,
      onAction: function () { App.go('/album/' + t.albumId); } });
    App.announce('Saved ' + (t ? t.title : 'the song') + ' to your library');
  }

  /* =====================================================================
     8. Player engine. Two <audio> elements: the playing one and the next, preloaded for gapless and
        crossfade. Saved songs come from /api/library/stream/{id}; Soulseek songs from /api/stream/{id},
        which serves bytes while the transfer runs (`stream` events report finding, queued, stalled,
        fallback). Loudness normalisation and crossfade go through Web Audio gains when switched on.
     status: idle | finding | queued | buffering | streaming | stalled | failed | library | ended | jellyfin-error
     `playing` is separate: paused = playing false while streaming or library.
     Fractions (0-1) of the track: received (bytes transferred), buffered ranges [[from, to], ...].
     ===================================================================== */
  var P = App.player = {};
  var saved0 = storage('player') || {};
  var ps = P.state = {
    queue: [], index: 0, playing: false, pos: 0, status: 'idle',
    received: 0, buffered: [], durationKnown: false,
    peer: null, peersFound: 0, queuePos: 0, fallback: null, failedPeers: [], source: null,
    crossfade: false, shuffle: !!saved0.shuffle, repeat: saved0.repeat || 'off',
    volume: saved0.volume == null ? .7 : saved0.volume, muted: !!saved0.muted, radio: null, error: null
  };
  var quSeq = 0, duration = 0;
  P.current = function () { var q = ps.queue[ps.index]; return q ? M.track(q.trackId) : null; };
  P.currentId = function () { var q = ps.queue[ps.index]; return q ? q.trackId : null; };
  P.duration = function () { var t = P.current(); return duration || (t ? t.duration : 0); };
  P.upNext = function (n) { return ps.queue.slice(ps.index + 1, ps.index + 1 + (n || 2)).map(function (q) { return M.track(q.trackId); }).filter(Boolean); };
  function changed(kind) { App.emit('player', kind || 'state'); persistSoon(); }

  /* playback settings (Settings > Playback; GET /api/settings), defaults until the server answers */
  var playback = { gapless: true, crossfade: 0, normalisation: 'off', radioWhenQueueEnds: true, volumeStep: 5 };
  P.volumeStep = function () { return (playback.volumeStep || 5) / 100; };
  App.setPlayback = function (patch) {
    Object.assign(playback, patch);
    App.store.set('crossfadeSec', playback.crossfade || 0);
    applyLevels();
  };

  /* ---------- the two decks ---------- */
  var decks = [makeDeck(), makeDeck()], cur = decks[0], nxt = decks[1];
  function makeDeck() {
    var d = { el: new Audio(), entry: null, url: null, soulseek: false, fade: 1, norm: 1, gain: null, analyser: null, retried: false };
    d.el.preload = 'auto';
    ['playing', 'waiting', 'timeupdate', 'durationchange', 'progress', 'ended', 'error'].forEach(function (type) {
      d.el.addEventListener(type, function () { onMedia(d, type); });
    });
    return d;
  }
  function streamUrl(t, source, forceSoulseek) {
    if (App.isSaved(t.id) && !forceSoulseek) return M.url('/api/library/stream/' + encodeURIComponent(t.id));
    var q = source ? '?peer=' + encodeURIComponent(source.peer) + '&path=' + encodeURIComponent(source.path) : '';
    return M.url('/api/stream/' + encodeURIComponent(t.id) + q);
  }
  /* point a deck at a queue entry (no-op when it already holds it) */
  function arm(d, entry, forceSoulseek) {
    var t = M.track(entry.trackId), url = streamUrl(t, entry.source, forceSoulseek);
    if (d.entry === entry && d.url === url) return;
    release(d);
    d.entry = entry; d.url = url; d.soulseek = url.indexOf('api/stream/') >= 0; d.retried = false; d.fade = 1; d.norm = 1; d.ms = 0; d.msN = 0;
    d.el.src = url;
    d.el.load();
  }
  /* empty a deck; a Soulseek transfer nobody will listen to is cancelled (DELETE /api/stream/{id}) */
  function release(d) {
    if (d.entry && d.soulseek && !d.complete) {
      var id = d.entry.trackId, other = d === cur ? nxt : cur;
      if (!(other.entry && other.entry.trackId === id)) M.api('/api/stream/' + encodeURIComponent(id), { method: 'DELETE' }).catch(function () {});
    }
    d.entry = null; d.url = null; d.complete = false;
    d.el.pause(); d.el.removeAttribute('src'); d.el.load();
  }

  function onMedia(d, type) {
    if (d !== cur) { if (type === 'error' && d.entry) d.entry.resolve = 'none'; return; }
    var t = P.current(); if (!t) return;
    switch (type) {
      case 'playing':
        ps.status = d.soulseek ? 'streaming' : 'library'; ps.error = null;
        beginPlay(t);
        break;
      case 'waiting':
        if (ps.playing && (ps.status === 'streaming' || ps.status === 'library')) ps.status = 'buffering';
        break;
      case 'timeupdate': trackListening(d); ps.pos = d.el.currentTime; return;
      case 'durationchange':
        if (isFinite(d.el.duration) && d.el.duration > 0) { duration = d.el.duration; ps.durationKnown = !d.soulseek || ps.received >= 1; }
        break;
      case 'progress': updateBuffered(); return;
      case 'ended': onEnded(); return;
      case 'error': onError(d); return;
      default: return;
    }
    changed('status');
  }

  /* ---------- load, play, pause ---------- */
  function resetTrackState() {
    ps.pos = 0; duration = 0; ps.fallback = null; ps.failedPeers = []; ps.error = null; ps.crossfade = false;
    ps.peer = null; ps.peersFound = 0; ps.queuePos = 0; ps.durationKnown = false;
  }
  /* begin playing queue[index]: opts { autoplay (default true), at (seconds), reason (how the last song ended), forceSoulseek } */
  function load(index, opts) {
    opts = opts || {};
    endPlay(opts.reason || 'skipped');
    ps.index = clamp(index, 0, Math.max(0, ps.queue.length - 1));
    resetTrackState();
    var entry = ps.queue[ps.index], t = P.current();
    if (!t) { release(cur); release(nxt); ps.status = 'idle'; ps.playing = false; ps.source = null; changed('track'); return; }
    if (nxt.entry === entry && !opts.forceSoulseek) { var s = cur; cur = nxt; nxt = s; }   // already preloaded
    release(nxt);                                       // the other deck's song is not what plays now
    arm(cur, entry, opts.forceSoulseek);
    ps.source = entry.source || null;
    if (cur.soulseek) { ps.status = 'finding'; ps.received = 0; ps.buffered = []; }
    else { ps.status = 'library'; ps.received = 1; ps.buffered = []; ps.durationKnown = true; }
    ps.playing = opts.autoplay !== false;
    if (opts.at) cur.el.currentTime = ps.pos = opts.at;
    if (ps.playing) start(cur);
    applyLevels();
    changed('track');
    topUpRadio();
  }
  function start(d) {
    ensureGraph();
    var p = d.el.play();
    if (p && p.catch) p.catch(function (err) {
      if (err.name === 'NotAllowedError') { ps.playing = false; changed('play'); }    // autoplay blocked until a tap
    });
  }
  P.play = function () {
    if (!ps.queue.length) { App.toast('Nothing queued. Search or pick something from Home.', { icon: 'info' }); return; }
    if (ps.status === 'ended') { load(0); return; }
    if (ps.status === 'failed' || ps.status === 'jellyfin-error') { P.retry(); return; }
    if (!cur.entry) { load(ps.index, { at: App.isSaved(P.currentId()) ? ps.pos : 0 }); return; }   // restored after a reload
    ps.playing = true; start(cur); changed('play');
  };
  P.pause = function () { ps.playing = false; cur.el.pause(); if (ps.crossfade) finishCrossfade(); changed('play'); };
  P.toggle = function () { if (ps.playing) P.pause(); else P.play(); };

  /* the index the queue moves to next: repeat one (only when a song ends by itself), shuffle, wrap, or -1 */
  var shuffleNext = null;
  function nextIndex(auto) {
    if (auto && ps.repeat === 'one') return ps.index;
    if (ps.shuffle && ps.queue.length > 2) {
      if (shuffleNext === null || shuffleNext === ps.index || shuffleNext >= ps.queue.length) {
        do { shuffleNext = Math.floor(Math.random() * ps.queue.length); } while (shuffleNext === ps.index);
      }
      return shuffleNext;
    }
    if (ps.index + 1 < ps.queue.length) return ps.index + 1;
    return ps.repeat === 'all' ? 0 : -1;
  }
  P.next = function (auto) {
    if (!ps.queue.length) return;
    var i = nextIndex(auto); shuffleNext = null;
    if (i === ps.index && auto) { cur.el.currentTime = 0; endPlay('completed'); start(cur); return; }
    if (i < 0) {                                        // the end: radio may still add songs
      topUpRadio().then(function (added) { if (added) load(ps.index + 1, { reason: auto ? 'completed' : 'skipped' }); else endQueue(auto); });
      return;
    }
    load(i, { reason: auto ? 'completed' : 'skipped' });
  };
  function endQueue(auto) {
    endPlay(auto ? 'completed' : 'skipped');
    ps.status = 'ended'; ps.playing = false; ps.pos = P.duration(); cur.el.pause();
    changed('ended');
  }
  P.prev = function () {
    if (ps.pos > 3 || ps.index === 0) { P.seek(0); return; }
    load(ps.index - 1);
  };

  /* ---------- seeking: only inside what has arrived ---------- */
  /* how far a seek may go (0-1): everything for saved songs, what the transfer delivered for Soulseek ones */
  function seekLimit() {
    if (!cur.soulseek) return 1;
    var bufEnd = ps.buffered.reduce(function (m, r) { return Math.max(m, r[1]); }, 0);
    return Math.max(bufEnd, ps.received >= 1 ? 1 : ps.received - .01);
  }
  P.seekable = function (f) { return P.current() ? f <= seekLimit() : false; };
  /* outside what arrived: snap to the furthest point that did, shake (seek bar) and return false */
  P.seekFraction = function (f) {
    f = clamp(f, 0, 1);
    var dur = P.duration(); if (!dur || !cur.entry) return false;
    var ok = P.seekable(f), target = ok ? f : Math.max(0, seekLimit() - .004);
    cur.el.currentTime = ps.pos = target * dur;
    changed('seek');
    if (!ok) App.emit('seek-denied', { fraction: f, received: ps.received });
    return ok;
  };
  P.seek = function (sec) { return P.seekFraction(sec / (P.duration() || 1)); };
  P.seekBy = function (d) { return P.seek(ps.pos + d); };

  P.setShuffle = function (on) { ps.shuffle = on == null ? !ps.shuffle : !!on; shuffleNext = null; releaseNext(); changed('mode'); };
  P.cycleRepeat = function () { ps.repeat = { off: 'all', all: 'one', one: 'off' }[ps.repeat]; releaseNext(); changed('mode'); };
  P.setVolume = function (v) { ps.volume = clamp(v, 0, 1); ps.muted = false; applyLevels(); changed('volume'); };
  P.toggleMute = function () { ps.muted = !ps.muted; applyLevels(); changed('volume'); };

  /* ---------- queue ---------- */
  function entry(id, group, source) {
    quSeq += 1;
    return { uid: 'q' + quSeq, trackId: id, group: group || 'context', resolve: App.isSaved(id) ? 'ready' : 'resolved', source: source || null };
  }
  function queueChanged() { shuffleNext = null; releaseNext(); changed('queue'); }
  /* replace the queue and start: playList(ids, startIndex, { radio, sources: { trackId: source }, at, autoplay }) */
  P.playList = function (ids, start, opts) {
    opts = opts || {};
    var group = opts.radio ? 'radio' : 'context', sources = opts.sources || {};
    endPlay('skipped'); release(nxt);
    ps.queue = ids.map(function (id) { return entry(id, group, sources[id]); });
    ps.radio = opts.radio || null;
    load(start || 0, { reason: 'skipped', at: opts.at, autoplay: opts.autoplay });
    queueChanged();
  };
  /* move to queue[index] without touching the queue: jumpTo(index, { at, autoplay }) */
  P.jumpTo = function (index, opts) { load(index, Object.assign({ reason: 'skipped' }, opts)); };
  P.streamUrl = function (trackId, source) { var t = M.track(trackId); return t ? streamUrl(t, source) : null; };
  P.playTrack = function (id, contextIds) {
    var ids = contextIds && contextIds.indexOf(id) >= 0 ? contextIds : [id];
    P.playList(ids, ids.indexOf(id));
  };
  function titleOf(id) { var t = M.track(id); return t ? t.title : 'one song'; }
  P.playNext = function (ids) {
    ids = [].concat(ids); var at = ps.index + 1;
    while (ps.queue[at] && ps.queue[at].group === 'next') at++;
    ps.queue.splice.apply(ps.queue, [at, 0].concat(ids.map(function (id) { return entry(id, 'next'); })));
    if (ps.status === 'idle') load(0);
    queueChanged();
    App.toast(ids.length > 1 ? ids.length + ' songs will play next' : 'Playing next: ' + titleOf(ids[0]), { icon: 'queue' });
  };
  P.addToQueue = function (ids) {
    ids = [].concat(ids);
    var at = ps.queue.length;
    while (at > ps.index + 1 && ps.queue[at - 1].group === 'radio') at--;   // before the radio tail
    ps.queue.splice.apply(ps.queue, [at, 0].concat(ids.map(function (id) { return entry(id, 'context'); })));
    if (ps.status === 'idle') load(0);
    queueChanged();
    App.toast(ids.length > 1 ? 'Added ' + ids.length + ' songs to the queue' : 'Added to queue: ' + titleOf(ids[0]), { icon: 'queue' });
  };
  P.removeAt = function (i) {
    if (i === ps.index) { App.toast('That song is playing. Skip it instead.', { icon: 'info' }); return; }
    var removed = ps.queue.splice(i, 1)[0]; if (i < ps.index) ps.index--;
    queueChanged();
    App.toast('Removed from queue', { icon: 'trash', action: 'Undo', onAction: function () {
      ps.queue.splice(i, 0, removed); if (i <= ps.index) ps.index++; queueChanged();
    } });
  };
  P.move = function (from, to) {
    if (from === to || from < 0 || to < 0 || from >= ps.queue.length || to >= ps.queue.length) return;
    var playingEntry = ps.queue[ps.index], item = ps.queue.splice(from, 1)[0];
    ps.queue.splice(to, 0, item); ps.index = ps.queue.indexOf(playingEntry); queueChanged();
  };
  P.clear = function () {
    var before = ps.queue.slice(), beforeIndex = ps.index, playingEntry = ps.queue[ps.index];
    ps.queue = playingEntry ? [playingEntry] : []; ps.index = 0; queueChanged();
    App.toast('Queue cleared', { icon: 'trash', action: 'Undo', onAction: function () { ps.queue = before; ps.index = beforeIndex; queueChanged(); } });
  };
  /* manual source choice (source picker): useSource(trackId, { peer, path, format, sizeBytes, speed }) */
  P.useSource = function (trackId, source) {
    var i = ps.queue.findIndex(function (q) { return q.trackId === trackId; });
    if (i < 0) { i = ps.queue.length ? ps.index + 1 : 0; ps.queue.splice(i, 0, entry(trackId, 'next', source)); }
    else ps.queue[i].source = source;
    load(i, { forceSoulseek: true });
    queueChanged();
  };
  P.retry = function () { load(ps.index, { at: ps.status === 'failed' ? 0 : ps.pos }); };
  /* a saved song Jellyfin can't serve: stream it from Soulseek instead */
  P.trySoulseek = function () { if (P.current()) load(ps.index, { forceSoulseek: true }); };

  /* ---------- radio: GET /api/radio tops the queue up while "keeps adding" is on ---------- */
  function radioActive() { return !!(ps.radio && ps.radio.keepsAdding); }
  function radioName(seed) {
    var e = seed.kind === 'artist' ? M.artist(seed.id) : seed.kind === 'track' ? M.track(seed.id) : null;
    return e ? e.name || e.title : seed.id;
  }
  function fetchRadio(seed, steer, exclude) {
    var q = '?kind=' + seed.kind + '&id=' + encodeURIComponent(seed.id) + '&steer=' + (steer == null ? .4 : steer).toFixed(2) +
      (exclude && exclude.length ? '&exclude=' + exclude.map(encodeURIComponent).join(',') : '');
    return M.api('/api/radio' + q).then(function (res) { return res.items || []; });
  }
  /* albums seed from their first song; the radio endpoint takes track, artist or tag */
  function radioSeed(seed) {
    if (seed.kind !== 'album') return seed;
    var a = M.album(seed.id);
    return a && a.trackIds.length ? { kind: 'track', id: a.trackIds[0] } : { kind: 'artist', id: a ? a.artistId : seed.id };
  }
  P.startRadio = function (seed) {
    seed = radioSeed(seed);
    var name = radioName(seed);
    return fetchRadio(seed, .4, []).then(function (ids) {
      if (seed.kind === 'track') ids = [seed.id].concat(ids.filter(function (id) { return id !== seed.id; }));
      if (!ids.length) { App.toast('No radio for ' + name + ' yet: nothing similar was found', { icon: 'radio' }); return false; }
      P.playList(ids, 0, { radio: { name: name, seed: seed, keepsAdding: true, steer: .4 } });
      App.toast('Radio: ' + name + ', keeps adding songs', { icon: 'radio' });
      return true;
    }, function (err) {
      App.toast('Couldn\'t start radio: ' + err.message, { kind: 'error' });
      return false;
    });
  };
  /* steering or switching "keeps adding" re-fills the radio songs that have not played yet */
  P.setRadio = function (patch) {
    var was = ps.radio || {}, steerChanged = patch && ps.radio && patch.steer != null && patch.steer !== was.steer;
    ps.radio = patch === null ? null : Object.assign({}, was, patch);
    if (steerChanged) ps.queue = ps.queue.filter(function (q, i) { return i <= ps.index || q.group !== 'radio'; });
    /* a new steer, "keeps adding" switched on, or a retry after an error (error cleared) tops up now */
    if (patch && (steerChanged || patch.keepsAdding && !was.keepsAdding || 'error' in patch && !patch.error)) topUpRadio();
    queueChanged();
  };
  var topping = null;
  /* keep at least three songs after the playing one; resolves to how many were added.
     With "radio when the queue ends" (Settings > Playback), the last song seeds a radio as it starts. */
  function topUpRadio() {
    if (topping) return topping;
    var left = ps.queue.length - ps.index - 1;
    if (!ps.radio && playback.radioWhenQueueEnds && ps.repeat === 'off' && ps.queue.length && left === 0) {
      var last = ps.queue[ps.index];
      ps.radio = { name: radioName({ kind: 'track', id: last.trackId }), seed: { kind: 'track', id: last.trackId }, keepsAdding: true, steer: .4 };
    }
    if (!radioActive() || left >= 3) return Promise.resolve(0);
    var have = ps.queue.map(function (q) { return q.trackId; });
    topping = fetchRadio(ps.radio.seed, ps.radio.steer, have).then(function (ids) {
      topping = null;
      if (ps.radio && ps.radio.error) ps.radio = Object.assign({}, ps.radio, { error: null });
      ids = ids.filter(function (id) { return have.indexOf(id) < 0; });
      ids.forEach(function (id) { ps.queue.push(entry(id, 'radio')); });
      if (ids.length) queueChanged();
      return ids.length;
    }, function (err) {
      topping = null;
      ps.radio = Object.assign({}, ps.radio, { error: err.message });
      changed('queue');
      return 0;
    });
    return topping;
  }

  /* ---------- server `stream` events: the Soulseek side of the playing song ---------- */
  function onStreamEvent(e) {
    var q = ps.queue[ps.index], d = q && q.trackId === e.trackId ? cur : nxt.entry && nxt.entry.trackId === e.trackId ? nxt : null;
    if (!d || !d.soulseek) return;
    if (e.size && e.received >= e.size) d.complete = true;
    if (d === nxt) { d.entry.resolve = e.status === 'failed' ? 'none' : e.status === 'streaming' ? 'ready' : 'resolving'; changed('queue'); return; }
    var prevPeer = ps.peer && ps.peer.name, t = P.current();
    ps.received = e.size ? clamp(e.received / e.size, 0, 1) : ps.received;
    ps.peersFound = e.peersFound || ps.peersFound;
    ps.queuePos = e.queuePos || 0;
    ps.failedPeers = (e.failedPeers || []).map(function (f) { return typeof f === 'string' ? { peer: f, at: null } : f; });
    if (e.peer) {
      ps.peer = { name: e.peer, rank: ps.failedPeers.length + 1, of: ps.peersFound || null, speed: e.speed ? e.speed / 1e6 : (ps.peer && ps.peer.speed) || 0,
        format: (ps.source && ps.source.format) || t.format, sizeMB: e.size ? e.size / 1e6 : t.sizeMB };
    }
    var elPlaying = !cur.el.paused && cur.el.readyState >= 3;
    switch (e.status) {
      case 'finding': case 'queued': case 'stalled':
        if (!elPlaying || e.status === 'stalled') ps.status = e.status; break;
      case 'buffering': if (!elPlaying) ps.status = 'buffering'; break;
      case 'streaming': ps.status = elPlaying || !ps.playing ? 'streaming' : 'buffering'; ps.fallback = null; break;
      case 'fallback':
        ps.fallback = { from: prevPeer, to: e.peer, n: ps.failedPeers.length + 1, of: ps.peersFound || null };
        ps.status = 'buffering';
        if (e.peer && e.peer !== prevPeer) App.toast('Now from ' + e.peer, { icon: 'soulseek', kind: 'ok' });
        break;
      case 'failed':
        ps.status = 'failed'; ps.playing = false; cur.el.pause();
        ps.error = e.error || 'Couldn\'t play: no peer responded';
        if (q) q.resolve = 'none';
        break;
    }
    if (ps.received >= 1 && isFinite(cur.el.duration)) ps.durationKnown = true;
    updateBuffered();
    changed(e.peer !== prevPeer ? 'source' : 'status');
  }

  /* buffered = what the element holds, cut to what the transfer delivered (both 0-1 of the song) */
  function updateBuffered() {
    var el = cur.el, dur = P.duration(), out = [];
    if (dur) for (var i = 0; i < el.buffered.length; i++) {
      var a = el.buffered.start(i) / dur, b = Math.min(el.buffered.end(i) / dur, cur.soulseek ? ps.received || 1 : 1);
      if (b > a) out.push([clamp(a, 0, 1), clamp(b, 0, 1)]);
    }
    ps.buffered = out;
    if (cur.soulseek && !ps.received && out.length) ps.received = out[out.length - 1][1];   // until the first event
  }

  /* the element failed: ask the server why (the body says it in a sentence), then show it */
  function onError(d) {
    var t = P.current();
    if (d.soulseek && !d.retried && ps.status !== 'failed') {        // a peer fallback can break the byte stream once
      d.retried = true; var at = ps.pos;
      d.el.src = d.url; d.el.load(); d.el.currentTime = at; if (ps.playing) start(d);
      return;
    }
    fetch(d.url, { headers: { Range: 'bytes=0-0' }, credentials: 'same-origin' }).then(function (res) {
      if (res.ok) return 'This file can\'t be played in this browser';
      return res.json().then(function (b) { return b.message; }, function () { return 'The server answered ' + res.status; });
    }, function () { return 'Can\'t reach the lull server'; }).then(function (msg) {
      if (d !== cur || P.current() !== t) return;
      ps.playing = false; ps.error = msg;
      ps.status = d.soulseek ? 'failed' : 'jellyfin-error';
      changed('status');
    });
  }

  /* ---------- gapless hand-over, crossfade, preload ---------- */
  function releaseNext() { if (nxt.entry) release(nxt); }
  /* preload the next song once the playing one is close to its end (or its transfer is done) */
  function preloadNext() {
    var i = nextIndex(true);
    if (i < 0 || i === ps.index || !playback.gapless && !playback.crossfade) return;
    /* a Soulseek song still transferring gets the bandwidth to itself until its last half minute */
    var e = ps.queue[i], dur = P.duration(), lead = Math.max(30, (playback.crossfade || 0) + 15) + (cur.soulseek && ps.received < 1 ? 0 : 60);
    if (!dur || dur - ps.pos > lead) return;
    if (nxt.entry !== e) { arm(nxt, e); if (e.resolve === 'resolved') e.resolve = 'resolving'; changed('queue'); }
  }
  function onEnded() {
    if (ps.crossfade) { finishCrossfade(); return; }
    var i = nextIndex(true);
    if (i >= 0 && i !== ps.index && nxt.entry === ps.queue[i]) {        // gapless: the next song is already loaded
      endPlay('completed'); shuffleNext = null;
      var s = cur; cur = nxt; nxt = s; release(nxt);
      ps.index = i; resetTrackState();
      ps.status = cur.soulseek ? 'buffering' : 'library'; ps.received = cur.soulseek && !cur.complete ? 0 : 1; ps.source = cur.entry.source || null;
      start(cur); applyLevels(); changed('track'); topUpRadio();
      return;
    }
    P.next(true);
  }
  function maybeCrossfade() {
    var xf = playback.crossfade || 0, dur = P.duration(), i = nextIndex(true);
    if (!xf || ps.crossfade || !ps.playing || i < 0 || i === ps.index || dur - ps.pos > xf || ps.repeat === 'one') return;
    if (nxt.entry !== ps.queue[i] || nxt.el.readyState < 3) return;
    ps.crossfade = true; xfStart = Date.now(); xfLen = Math.max(1, (dur - ps.pos) * 1000);
    nxt.fade = 0; start(nxt);
    changed('status');
  }
  var xfStart = 0, xfLen = 0;
  function stepCrossfade() {
    if (!ps.crossfade) return;
    var k = clamp((Date.now() - xfStart) / xfLen, 0, 1);
    cur.fade = Math.cos(k * Math.PI / 2); nxt.fade = Math.sin(k * Math.PI / 2);   // equal-power
    applyLevels();
    if (k >= 1) finishCrossfade();
  }
  function finishCrossfade() {
    if (!ps.crossfade) return;
    ps.crossfade = false;
    if (!ps.playing) { nxt.fade = 1; nxt.el.pause(); nxt.el.currentTime = 0; applyLevels(); changed('status'); return; }
    endPlay('completed');
    var i = nextIndex(true); shuffleNext = null;
    var s = cur; cur = nxt; nxt = s; cur.fade = 1; release(nxt);
    ps.index = i; resetTrackState(); ps.pos = cur.el.currentTime;
    ps.status = cur.soulseek ? 'streaming' : 'library'; ps.received = cur.soulseek ? ps.received : 1; ps.source = cur.entry.source || null;
    applyLevels(); changed('track'); topUpRadio();
  }

  /* ---------- levels: volume x crossfade x loudness normalisation ---------- */
  var audioCtx = null, master = null;
  function wantGraph() { return playback.normalisation && playback.normalisation !== 'off' || playback.crossfade > 0; }
  /* Web Audio only when a feature needs it; built inside a user gesture (play), once */
  function ensureGraph() {
    if (audioCtx || !wantGraph() || !window.AudioContext) { if (audioCtx && audioCtx.state === 'suspended') audioCtx.resume(); return; }
    audioCtx = new AudioContext();
    master = audioCtx.createGain(); master.connect(audioCtx.destination);
    decks.forEach(function (d) {
      var src = audioCtx.createMediaElementSource(d.el);
      d.gain = audioCtx.createGain(); d.analyser = audioCtx.createAnalyser(); d.analyser.fftSize = 2048;
      src.connect(d.gain); d.gain.connect(master); src.connect(d.analyser);
    });
    applyLevels();
  }
  function applyLevels() {
    var vol = ps.muted ? 0 : ps.volume * ps.volume;                     // perceptual curve
    decks.forEach(function (d) {
      var level = d.fade * (playback.normalisation === 'off' ? 1 : d.norm);
      if (d.gain) { d.gain.gain.setTargetAtTime(level, audioCtx.currentTime, .05); d.el.volume = 1; }
      else d.el.volume = clamp(vol * d.fade, 0, 1);
    });
    if (master) master.gain.setTargetAtTime(vol, audioCtx.currentTime, .05);
  }
  /* ReplayGain from the track when the library supplies it (replayGain: { track, album } in dB), otherwise
     measured: mean loudness of what has played, levelled toward -18 dBFS RMS, within -12 to +6 dB */
  var TARGET_RMS = .126;
  function normalise(d) {
    if (!d.gain || playback.normalisation === 'off' || !d.entry) return;
    var t = M.track(d.entry.trackId), rg = t && t.replayGain;
    var db = rg && (playback.normalisation === 'replaygain-album' ? rg.album : playback.normalisation.indexOf('replaygain') === 0 ? rg.track : null);
    if (db != null) { d.norm = Math.pow(10, db / 20); return; }
    if (d.el.paused) return;
    var buf = d.buf || (d.buf = new Float32Array(d.analyser.fftSize));
    d.analyser.getFloatTimeDomainData(buf);
    var sum = 0; for (var i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
    d.ms += sum / buf.length; d.msN += 1;
    if (d.msN < 12) return;                                            // ~3 s of listening before adjusting
    var rms = Math.sqrt(d.ms / d.msN); if (rms < 1e-4) return;
    d.norm = clamp(TARGET_RMS / rms, Math.pow(10, -12 / 20), Math.pow(10, 6 / 20));
  }

  /* ---------- plays: POST /api/plays when a song starts, PATCH when it ends (scrobbling, history, taste) ---------- */
  /* App.recordPlay(trackId, durationS) -> { listened (seconds heard, the caller adds to it), end(how) };
     how: completed (ended by itself) | skipped (the listener moved on) | stopped (page closed).
     Also used by Classic mode, where Webamp plays. */
  var playWarned = false;
  App.recordPlay = function (trackId, dur) {
    var rec = { listened: 0, ended: false };
    var id = M.api('/api/plays', { method: 'POST', body: { trackId: trackId, duration: Math.round(dur || 0) } })
      .then(function (r) { return r && r.playId; }, function (err) {
        if (!playWarned) { playWarned = true; App.toast('Listening history is off: ' + err.message, { kind: 'error' }); }
        return null;
      });
    rec.end = function (how) {
      if (rec.ended) return; rec.ended = true;
      id.then(function (playId) {
        if (playId) M.api('/api/plays/' + encodeURIComponent(playId), { method: 'PATCH', keepalive: true,
          body: { playedS: Math.round(rec.listened), completed: how === 'completed', skipped: how === 'skipped' } }).catch(function () {});
      });
    };
    return rec;
  };
  var play = null;            // { uid (queue entry), rec, last (element time at the last timeupdate) }
  function beginPlay(t) {
    var e = ps.queue[ps.index];
    if (play && play.uid === e.uid) return;
    endPlay('skipped');
    play = { uid: e.uid, rec: App.recordPlay(t.id, P.duration()), last: null };
  }
  function trackListening(d) {
    if (!play) return;
    var now = d.el.currentTime, dt = play.last == null ? 0 : now - play.last;
    if (dt > 0 && dt < 1.5 && ps.playing) play.rec.listened += dt;
    play.last = now;
  }
  function endPlay(how) { if (play) { play.rec.end(how); play = null; } }
  window.addEventListener('pagehide', function () { endPlay('stopped'); persist(); });

  /* ---------- the clock: position, crossfade, preload, normalisation; 'tick' for screens ---------- */
  var TICK = 250;
  setInterval(function () {
    if (!cur.entry) return;
    if (ps.playing) { normalise(cur); if (ps.crossfade) normalise(nxt); }
    stepCrossfade(); maybeCrossfade(); preloadNext();
    if (document.hidden) return;
    ps.pos = cur.el.currentTime || ps.pos;
    updateBuffered();
    App.emit('tick', ps);
  }, TICK);

  /* ---------- the queue survives a reload on this device (localStorage, a convenience) ---------- */
  var persistTimer = null;
  function persistSoon() { clearTimeout(persistTimer); persistTimer = setTimeout(persist, 600); }
  function persist() {
    var keep = {};
    ps.queue.forEach(function (q) { var t = M.track(q.trackId); if (t) keep[t.id] = t; });
    storage('player', { volume: ps.volume, muted: ps.muted, shuffle: ps.shuffle, repeat: ps.repeat, radio: ps.radio,
      index: ps.index, pos: ps.pos, queue: ps.queue.map(function (q) { return { trackId: q.trackId, group: q.group, source: q.source }; }), tracks: keep });
  }
  function restore() {
    if (!saved0.queue || !saved0.queue.length) return;
    M.absorb({ tracks: saved0.tracks });
    ps.queue = saved0.queue.filter(function (q) { return M.track(q.trackId); }).map(function (q) { return entry(q.trackId, q.group, q.source); });
    if (!ps.queue.length) return;
    ps.index = clamp(saved0.index || 0, 0, ps.queue.length - 1);
    ps.radio = saved0.radio || null;
    ps.pos = saved0.pos || 0;
    var t = P.current();
    ps.status = App.isSaved(t.id) ? 'library' : 'streaming';
    ps.received = App.isSaved(t.id) ? 1 : 0;
  }
  restore();

  /* one line describing the current status (player bar state tab, mini-player sub line, aria) */
  P.statusText = function () {
    var peer = ps.peer ? ps.peer.name : 'a peer', of = ps.peer && ps.peer.of ? ' of ' + ps.peer.of : '';
    switch (ps.status) {
      case 'idle': return 'Search or pick something from Home';
      case 'finding': return 'Finding a source' + (ps.peersFound ? ', ' + ps.peersFound + ' peers answered' : '');
      case 'queued': return 'Queued at peer ' + peer + (ps.queuePos ? ', position ' + ps.queuePos : '');
      case 'buffering':
        if (!cur.soulseek) return 'Loading from your library';
        return ps.fallback ? 'Switching to ' + ps.fallback.to + ' (' + ps.fallback.n + (ps.fallback.of ? ' of ' + ps.fallback.of : '') + ')' : 'Buffering from ' + peer;
      case 'stalled': return 'Peer stalled, switching to the next source (' + ((ps.peer ? ps.peer.rank : 1) + 1) + of + ')';
      case 'failed': return ps.error || 'Couldn\'t play: no peer responded';
      case 'jellyfin-error': return 'Library item unavailable' + (ps.error ? ': ' + ps.error : '');
      case 'ended': return 'End of queue';
      default:
        if (ps.crossfade) { var n = M.track(nxt.entry ? nxt.entry.trackId : ''); return 'Crossfading into ' + (n ? n.title : 'the next song'); }
        return null;
    }
  };

  /* =====================================================================
     9. Live bindings: seek bars, waveforms, data-bind text, rows (playing / liked / saved)
     ===================================================================== */
  function bufKey(r) { return r.map(function (x) { return x[0].toFixed(3) + '-' + x[1].toFixed(3); }).join('|'); }
  function updateSeeks() {
    var dur = P.duration() || 1, f = clamp(ps.pos / dur, 0, 1), bk = bufKey(ps.buffered);
    var bufTo = ps.buffered.length ? ps.buffered[ps.buffered.length - 1][1] * dur : 0;
    var valText = fmtTime(ps.pos) + ' of ' + (ps.durationKnown ? '' : 'about ') + fmtTime(dur) + (ps.received < 1 ? ', downloaded to ' + fmtTime(bufTo) : '');
    var transferring = ps.received < 1 && (ps.status === 'streaming' || ps.status === 'buffering');
    // player bar + mini: play state and the received percentage (an integer, usable as counter(rx) in CSS)
    ['#pbar', '#mini'].forEach(function (s) { var el = $(s); if (!el) return; el.setAttribute('data-playing', String(!!ps.playing)); el.style.setProperty('--rx', Math.round(ps.received * 100)); });
    $$('.seek[data-seek="player"]').forEach(function (el) {
      el.style.setProperty('--pos', f.toFixed(4));
      el.style.setProperty('--xfer', ps.received.toFixed(4));
      el.classList.toggle('is-transferring', transferring);
      if (el._bk !== bk) {
        el._bk = bk;
        var host = $('.seek__bufs', el);
        if (host) host.innerHTML = ps.buffered.map(function (r) { return '<i class="seek__buf" style="left:' + (r[0] * 100).toFixed(2) + '%;width:' + ((r[1] - r[0]) * 100).toFixed(2) + '%"></i>'; }).join('');
      }
      el.setAttribute('aria-valuenow', Math.round(f * 100));
      el.setAttribute('aria-valuetext', valText);
    });
    $$('.wave[data-seek="player"]').forEach(function (el) {
      var bars = el._bars || (el._bars = $$('.wave__bar', el)), n = bars.length;
      el.style.setProperty('--pos', f.toFixed(4));
      el.classList.toggle('is-playing', ps.playing && ps.status !== 'buffering');
      bars.forEach(function (b, i) {
        var x = (i + .5) / n, inBuf = ps.buffered.some(function (r) { return x >= r[0] && x <= r[1]; });
        var c = x <= f ? 'is-played' : inBuf ? 'is-buf' : x <= ps.received ? 'is-xfer' : '';
        if (b._c !== c) { if (b._c) b.classList.remove(b._c); if (c) b.classList.add(c); b._c = c; }
      });
      el.setAttribute('aria-valuenow', Math.round(f * 100));
      el.setAttribute('aria-valuetext', valText);
    });
    $$('[data-bind="pos"]').forEach(function (el) { el.textContent = fmtTime(ps.pos); });
    $$('[data-bind="dur"]').forEach(function (el) { el.textContent = fmtTime(dur, !ps.durationKnown); });
    $$('[data-bind="buffered-to"]').forEach(function (el) { el.textContent = fmtTime(bufTo); });
    $$('[data-bind="received"]').forEach(function (el) { el.textContent = Math.round(ps.received * 100) + '%'; });
    var vol = ps.muted ? 0 : ps.volume;
    $$('.seek[data-seek="volume"]').forEach(function (el) {
      el.style.setProperty('--pos', vol.toFixed(3)); el.style.setProperty('--xfer', 0);
      el.setAttribute('aria-valuenow', Math.round(vol * 100)); el.setAttribute('aria-valuetext', ps.muted ? 'Muted' : Math.round(vol * 100) + '%');
    });
  }
  App.on('tick', updateSeeks);
  App.on('player', updateSeeks);

  /* rows and buttons that show per-track state: playing row, like buttons, Saved/Soulseek badges, save buttons */
  App.syncRows = function () {
    var cur = P.currentId();
    $$('.trow[data-track-id]').forEach(function (r) {
      var on = r.getAttribute('data-track-id') === cur;
      r.classList.toggle('is-playing', on);
      r.classList.toggle('is-playing-now', on && ps.playing);
    });
    $$('[data-like]').forEach(function (b) {
      var id = b.getAttribute('data-like'), on = App.isLiked(id);
      if (b.getAttribute('aria-pressed') === String(on)) return;
      b.setAttribute('aria-pressed', String(on)); b.classList.toggle('is-liked', on);
      b.setAttribute('data-tip', on ? 'Liked' : 'Like');
      var u = $('use', b); if (u) u.setAttribute('href', '#i-' + (on ? 'heart-fill' : 'heart'));
    });
    $$('[data-src-badge]').forEach(function (el) {
      var id = el.getAttribute('data-src-badge'), want = App.isSaved(id) ? 'saved' : 'slsk';
      if (el.getAttribute('data-was') === want) return;
      el.setAttribute('data-was', want);
      el.innerHTML = String(want === 'saved' ? ui.savedBadge() : ui.soulseekBadge());
    });
    $$('[data-src-mini]').forEach(function (el) {
      var on = App.isSaved(el.getAttribute('data-src-mini'));
      el.innerHTML = window.icon(on ? 'saved' : 'soulseek', on ? 'Saved' : 'Soulseek');
    });
    $$('[data-save-btn]').forEach(function (el) {
      var keep = el.className.split(/\s+/).filter(function (c) { return c && c !== 'btn' && c !== 'btn-save'; }).join(' ');
      var id = el.getAttribute('data-save-btn'), next = ui.saveButton(id, { cls: keep });
      var focused = el === document.activeElement;
      var tmp = document.createElement('div'); tmp.innerHTML = String(next);
      var nb = tmp.firstChild; if (nb.outerHTML === el.outerHTML) return;
      el.replaceWith(nb); if (focused) nb.focus();
    });
  };
  App.on('player', function (k) { if (k === 'track' || k === 'play' || k === 'queue') App.syncRows(); });
  App.on('like', function () { App.syncRows(); });

  /* pointer + keyboard for every [data-seek] (player seek, waveform, volume) */
  var drag = null;
  function seekFrac(el, x) {
    var t = $('.seek__track', el) || el, r = t.getBoundingClientRect();
    return clamp((x - r.left) / r.width, 0, 1);
  }
  function applySeek(el, f, final) {
    if (el.getAttribute('data-seek') === 'volume') { P.setVolume(f); return; }
    var ok = P.seekFraction(f);
    if (!ok && final) {
      el.classList.remove('is-shake'); void el.offsetWidth; el.classList.add('is-shake');
      setTimeout(function () { el.classList.remove('is-shake'); }, 420);
    }
  }
  document.addEventListener('pointerdown', function (e) {
    var el = e.target.closest('[data-seek]'); if (!el || e.button !== 0) return;
    e.preventDefault(); el.focus({ preventScroll: true });
    drag = { el: el }; el.classList.add('is-dragging');
    try { el.setPointerCapture(e.pointerId); } catch (err) { /* synthetic events */ }
    applySeek(el, seekFrac(el, e.clientX), false);
  });
  document.addEventListener('pointermove', function (e) {
    if (drag) { applySeek(drag.el, seekFrac(drag.el, e.clientX), false); return; }
    var el = e.target.closest && e.target.closest('.seek[data-seek="player"]');
    $$('.seek__tip.is-on').forEach(function (t) { if (!el || !el.contains(t)) t.classList.remove('is-on'); });
    if (!el) return;
    var f = seekFrac(el, e.clientX), tip = $('.seek__tip', el);
    var inBuf = P.seekable(f);
    if (!tip) return;
    tip.textContent = inBuf ? fmtTime(f * P.duration()) : 'Not downloaded yet (' + Math.round(ps.received * 100) + '%)';
    tip.style.left = (f * 100) + '%'; tip.classList.add('is-on');
  });
  document.addEventListener('pointerup', function (e) {
    if (!drag) return;
    applySeek(drag.el, seekFrac(drag.el, e.clientX), true);
    drag.el.classList.remove('is-dragging'); drag = null;
  });
  document.addEventListener('keydown', function (e) {
    var el = e.target.closest && e.target.closest('[data-seek]'); if (!el) return;
    var vol = el.getAttribute('data-seek') === 'volume', step = vol ? .05 : 5 / (P.duration() || 1);
    var cur = vol ? (ps.muted ? 0 : ps.volume) : ps.pos / (P.duration() || 1), f = null;
    if (e.key === 'ArrowRight' || e.key === 'ArrowUp') f = cur + step * (e.shiftKey ? 6 : 1);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') f = cur - step * (e.shiftKey ? 6 : 1);
    else if (e.key === 'Home') f = 0; else if (e.key === 'End') f = 1;
    else if (e.key === 'PageUp') f = cur + step * 6; else if (e.key === 'PageDown') f = cur - step * 6;
    if (f === null) return;
    e.preventDefault(); e.stopPropagation(); applySeek(el, clamp(f, 0, 1), true);
  });
  App.on('seek-denied', function (d) {
    App.emit('announce', 'Not downloaded yet, ' + Math.round(d.received * 100) + ' percent');
  });

  /* =====================================================================
     10. Shell: rail, header, status tray, context panel, player bar, mini-player, tab bar
     ===================================================================== */
  function followedTags() {
    return html`${Object.keys(state.followed).filter(function (id) { return !/^ar-/.test(id); }).map(function (id) { return ui.tagChip(id); })}`;
  }
  function playlistLinks() {
    return html`${state.playlists.map(function (p) {
      return html`<li><a class="rail__pl" href="#/playlist/${p.id}" data-drop="playlist:${p.id}">${ui.mosaic(p.items.map(function (i) { return i.trackId; }), 36)}<span class="ellipsis"><b>${p.name}</b><small>${p.items.length} songs</small></span></a></li>`;
    })}`;
  }
  function renderRail() {
    $('#rail-tags').innerHTML = String(followedTags());
    $('#rail-playlists').innerHTML = String(playlistLinks());
  }
  function renderBrand() {
    $$('[data-brand]').forEach(function (a) {
      a.innerHTML = String(ui.brand(a.closest('.rail') ? 'rail' : 'header'));
      a.setAttribute('aria-label', App.product + ' home');     // label-in-name: the visible wordmark reads "lull"
    });
  }
  /* topnav and dock shells have no rail: following + playlists open from the header (.header__shelf) */
  function shelfPopover(anchor) {
    App.popover(anchor, html`<div class="popover__head">following</div><div class="shelf-pop__tags">${followedTags()}</div>
      <div class="popover__head">playlists</div><ul class="rail__list shelf-pop__list">${playlistLinks()}</ul>
      <div class="popover__actions"><a class="btn btn--sm" href="#/following">Everything you follow</a><a class="btn btn--sm" href="#/playlists">All playlists</a></div>`,
      { label: 'Following and playlists', className: 'shelf-pop', onMount: function (el, close) {
        el.addEventListener('click', function (e) { if (e.target.closest('a')) setTimeout(function () { App.closePopover(); }, 0); });
      } });
  }
  /* everything outside the view that a design can change: brand, rail, tray, context panel, player bar */
  function renderShell() { renderBrand(); renderRail(); renderTray(); renderCtx(); renderBar(); layoutCtx(); }
  var STATE_WORDS = { ok: 'ok', warn: 'needs attention', down: 'down', off: 'not set up' };
  function trayLabel() {
    var s = state.services, parts = [];
    Object.keys(s).forEach(function (k) {
      parts.push(s[k].label + ' ' + STATE_WORDS[s[k].state] + (k === 'cache' && s[k].capGB ? ', ' + s[k].usedGB + ' of ' + s[k].capGB + ' GB' : k === 'lastfm' && s[k].queued ? ', ' + s[k].queued + ' queued' : ''));
    });
    return 'Service status: ' + parts.join('; ') + '. Open details';
  }
  /* the compact pill's status words: the worst service (down before warn, Soulseek first), else "all good" */
  var TRAY_WARN = { soulseek: 'reconnecting', jellyfin: 'slow', cache: 'full', lastfm: 'queued' };
  function trayWorst() {
    var s = state.services, keys = ['soulseek', 'jellyfin', 'cache', 'lastfm'], i;
    for (i = 0; i < keys.length; i++) if (s[keys[i]].state === 'down') return { state: 'down', word: s[keys[i]].label + ' down' };
    for (i = 0; i < keys.length; i++) if (s[keys[i]].state === 'warn') return { state: 'warn', word: s[keys[i]].label + ' ' + TRAY_WARN[keys[i]] };
    return { state: 'ok', word: 'all good' };
  }
  /* Status tray. Design hooks: every pill has data-svc + data-state; the Last.fm count is .tray__count; the cache pill
     (.tray__pill--cache) and both tray buttons carry --cache-frac (0-1, used / cap) and data-cache-used / -cap (GB). */
  function renderTray() {
    var s = state.services, c = s.cache;
    var frac = c.capGB ? Math.max(0, Math.min(1, c.usedGB / c.capGB)) : 0;
    var cacheAttrs = raw('data-cache-used="' + (c.usedGB || 0) + '" data-cache-cap="' + (c.capGB || 0) + '" style="--cache-frac:' + frac.toFixed(3) + '"');
    var gb = c.capGB ? c.usedGB + '/' + c.capGB + 'G' : 'off';
    var full = html`${['soulseek', 'jellyfin', 'lastfm'].map(function (k) {
      var extra = k === 'lastfm' && s.lastfm.queued ? html`&nbsp;<span class="tray__count">${s.lastfm.queued}</span><span class="tray__label-long">&nbsp;queued</span>` : '';
      return html`<span class="tray__pill" data-svc="${k}" data-state="${s[k].state}">${ui.statusGlyph(s[k].state)}${s[k].label}${extra}</span>`;
    })}<span class="tray__pill tray__pill--cache" data-svc="cache" data-state="${c.state}" ${cacheAttrs}>${ui.statusGlyph(c.state)}<span class="tray__label-long">cache&nbsp;</span>${gb}</span>`;
    var worst = trayWorst();
    var compact = html`<span class="tray__pill" data-worst="${worst.state}">${['soulseek', 'jellyfin', 'lastfm'].map(function (k) { return ui.statusGlyph(s[k].state); })}<span class="tray__word">${worst.word}</span>${c.capGB ? html`<span class="tray__gb">${c.usedGB}G</span>` : ''}</span>`;
    $$('[data-tray]').forEach(function (b) {
      b.innerHTML = String(b.getAttribute('data-tray') === 'compact' ? compact : full);
      b.setAttribute('aria-label', trayLabel());
      b.style.setProperty('--cache-frac', frac.toFixed(3));
      b.setAttribute('data-cache-used', c.usedGB || 0); b.setAttribute('data-cache-cap', c.capGB || 0);
    });
  }
  function statusPopover(anchor) {
    var s = state.services;
    var actions = { soulseek: ['Settings', 'open-sources'], jellyfin: ['Rescan', 'jellyfin-rescan'], lastfm: ['Retry now', 'lastfm-retry'], cache: ['Manage', 'open-cache'] };
    var body = html`<div class="popover__head">services</div><div class="status-list">${Object.keys(s).map(function (k) {
      var st = s[k].state;
      return html`<div class="status-row">${ui.statusGlyph(st, STATE_WORDS[st])}<div><span class="status-row__name">${s[k].label}</span><span class="status-row__detail">${s[k].detail}</span></div>
        <button class="btn btn--sm" data-act="${actions[k][1]}">${actions[k][0]}</button></div>`;
    })}</div>`;
    App.popover(anchor, body, { label: 'Service status', className: 'popover--status' });
  }
  /* a service as /health or a `service` event reports it: { state, detail, extra } -> the tray's shape */
  function applyService(name, svc) {
    if (!SERVICE_LABELS[name]) return;
    var x = svc.extra || {}, patch = { state: svc.state, detail: svc.detail };
    if (name === 'cache') { patch.usedGB = Math.round((x.usedBytes || 0) / 1e8) / 10; patch.capGB = Math.round((x.capBytes || 0) / 1e8) / 10; }
    if (name === 'lastfm') patch.queued = x.queued || 0;
    App.setService(name, patch);
  }
  App.setService = function (key, patch) {
    App.store.update('services', function (s) { s[key] = Object.assign({}, s[key], patch); });
    renderTray();
  };
  App.on('store', function (keys) {
    if (keys.indexOf('services') >= 0) renderTray();
    if (keys.indexOf('playlists') >= 0 || keys.indexOf('followed') >= 0) renderRail();
  });

  /* running jobs: transfers and imports from the activity list; saves from the save records (one per track) */
  function activeJobs() {
    var n = state.activity.filter(function (j) { return j.kind !== 'save' && (j.state === 'transferring' || j.state === 'queued' || j.state === 'running'); }).length;
    return n + Object.keys(state.saves).filter(function (id) { return state.saves[id].status === 'saving'; }).length;
  }
  /* GET /api/jobs + `job` / `transfer` events keep the Activity list (rows shaped like design/prototype/mock-data.js activity, id = jobId) */
  function upsertJob(row) {
    App.store.update('activity', function (list) {
      var i = list.findIndex(function (j) { return j.id === row.id; });
      if (i < 0) list.unshift(row); else list[i] = Object.assign({}, list[i], row);
    });
  }
  function onJobEvent(e) { upsertJob(Object.assign({}, e, { id: e.jobId })); }
  function onTransferEvent(e) {
    upsertJob({ id: e.jobId, kind: 'transfer', trackId: e.trackId, peer: e.peer, state: e.state, queuePos: e.queuePos,
      pct: e.size ? Math.round(e.received / e.size * 100) : 0, speed: e.speed ? e.speed / 1e6 : 0, error: e.error || null });
  }
  function loadJobs() {
    return M.api('/api/jobs').then(function (res) {
      App.store.set({ activity: (res.items || res || []).map(function (j) { return Object.assign({ id: j.jobId }, j); }), activityError: null });
    }, function (err) { App.store.set({ activity: [], activityError: err.message }); });
  }
  App.loadJobs = loadJobs;
  function updateActivityBadge() {
    var n = activeJobs();
    $$('[data-activity-count]').forEach(function (el) { el.textContent = n; el.hidden = !n; });
    $$('[data-act="open-activity"]').forEach(function (b) { b.setAttribute('aria-label', 'Activity, ' + n + ' running'); });
  }
  App.activeJobs = activeJobs;

  /* context panel: App.panel(id, { label, render(el) -> markup, mount(el) }) registers a tab (queue, lyrics, activity) */
  var panels = {}, panelOrder = ['queue', 'lyrics', 'activity'];
  App.panel = function (id, def) { panels[id] = def; if (panelOrder.indexOf(id) < 0) panelOrder.push(id); renderCtx(); };
  App.openPanel = function (tab) {
    if (tab) state.ctxTab = tab;
    var app = $('#app');
    if (window.innerWidth >= 1360) { storage('ctxCollapsed', false); app.setAttribute('data-ctx', 'docked'); }
    else app.setAttribute('data-ctx', 'open');
    renderCtx();
    var body = $('#ctx-body'); if (body && app.getAttribute('data-ctx') === 'open') setTimeout(function () { var t = $('.seg__btn[aria-selected="true"]', $('#ctx')); if (t) t.focus(); }, 60);
  };
  App.closePanel = function () {
    var app = $('#app');
    if (app.getAttribute('data-ctx') === 'docked') storage('ctxCollapsed', true);
    app.setAttribute('data-ctx', 'closed'); updatePanelButtons();
  };
  App.togglePanel = function (tab) {
    var mode = $('#app').getAttribute('data-ctx'), open = mode === 'docked' || mode === 'open';
    if (open && (!tab || tab === state.ctxTab)) App.closePanel(); else App.openPanel(tab);
  };
  App.refreshPanel = function () { renderCtx(); };
  function renderCtx() {
    var host = $('#ctx'); if (!host) return;
    var tabs = panelOrder.filter(function (id) { return panels[id]; });
    if (tabs.indexOf(state.ctxTab) < 0) state.ctxTab = tabs[0];
    var i = tabs.indexOf(state.ctxTab);
    $('#ctx-tabs').style.setProperty('--n', tabs.length); $('#ctx-tabs').style.setProperty('--i', i);
    $('#ctx-tabs').innerHTML = '<span class="seg__ind" aria-hidden="true"></span>' + tabs.map(function (id) {
      var on = id === state.ctxTab;
      return '<button class="seg__btn" role="tab" id="ctx-tab-' + id + '" aria-controls="ctx-body" aria-selected="' + on + '" tabindex="' + (on ? 0 : -1) + '" data-ctx-tab="' + id + '">' + esc(panels[id].label) + (id === 'activity' && activeJobs() ? ' <span class="badge-count">' + activeJobs() + '</span>' : '') + '</button>';
    }).join('');
    var body = $('#ctx-body'), p = panels[state.ctxTab];
    var title = $('#ctx-title'); if (title) title.textContent = p ? p.label.toLowerCase() : '';
    updateCtxMeta();
    body.setAttribute('aria-labelledby', 'ctx-tab-' + state.ctxTab);
    body.innerHTML = p ? String(p.render(body) || '') : '';
    if (p && p.mount) p.mount(body);
    updatePanelButtons();
  }
  /* the panel's meta line: #ctx[data-meta] + .region-meta (panel def meta(), e.g. queue "14 songs, 52 min") */
  function updateCtxMeta() {
    var p = panels[state.ctxTab], m = p && p.meta ? safeCall(p.meta) : '';
    $('#ctx').setAttribute('data-meta', m);
    var el = $('#ctx-meta'); if (el) el.textContent = m;
  }
  function updatePanelButtons() {
    var mode = $('#app').getAttribute('data-ctx'), open = mode === 'docked' || mode === 'open';
    $$('[data-act="toggle-panel"]').forEach(function (b) {
      b.setAttribute('aria-pressed', String(open && b.getAttribute('data-tab') === state.ctxTab));
      b.setAttribute('aria-expanded', String(open));
    });
  }
  document.addEventListener('click', function (e) {
    var t = e.target.closest('[data-ctx-tab]'); if (!t) return;
    state.ctxTab = t.getAttribute('data-ctx-tab'); renderCtx();
    var nb = $('[data-ctx-tab="' + state.ctxTab + '"]'); if (nb) nb.focus();
  });
  App.on('player', function (k) { if (k === 'queue' || k === 'track') { if (state.ctxTab === 'queue' || state.ctxTab === 'lyrics') renderCtx(); } });
  function layoutCtx() {
    var app = $('#app'), mode = app.getAttribute('data-ctx'), w = window.innerWidth;
    if (w >= 1360 && !storage('ctxCollapsed') && mode !== 'docked') app.setAttribute('data-ctx', 'docked');
    else if (w < 1360 && mode === 'docked') app.setAttribute('data-ctx', 'closed');
    if (!app.getAttribute('data-ctx')) app.setAttribute('data-ctx', 'closed');
    updatePanelButtons();
  }
  window.addEventListener('resize', layoutCtx);

  /* ---------- player bar + mini-player ---------- */
  function renderBar() {
    var t = P.current(), bar = $('#pbar'), mini = $('#mini');
    var st = ps.status, idle = !t || st === 'idle';
    bar.setAttribute('data-status', idle ? 'idle' : st);
    mini.setAttribute('data-status', idle ? 'idle' : st);
    var now = $('#pbar-now');
    if (idle) {
      now.innerHTML = String(html`<span class="bezel-thumb bezel-thumb--empty" aria-hidden="true"><span></span></span><div class="pbar__idle">${icon('sparkle')}<span>Search or pick something from Home</span></div>`);
    } else {
      now.innerHTML = String(html`<a class="bezel-thumb" href="#/now" aria-label="Open Now Playing: ${t.title}" data-tip="Now Playing">${ui.cover(t, 52)}</a>
        <div class="pbar__text">
          <div class="pbar__title"><a href="#/album/${t.albumId}">${t.title}</a></div>
          <div class="pbar__artist"><a href="#/artist/${t.artistId}">${t.artist}</a></div>
          <div class="pbar__badges">${ui.sourceBadge(t)}${playingFormat(t) ? ui.qualityChip(playingFormat(t), { solid: true, button: true, id: t.id }) : ''}</div>
        </div>`);
    }
    // state tab above the seek bar
    var line = P.statusText(), stEl = $('#pbar-state');
    if (line && st !== 'idle') {
      var err = st === 'failed' || st === 'jellyfin-error';
      var acts = st === 'failed' ? html`<button class="btn btn--quiet" data-act="retry-search">Retry search</button><button class="btn btn--quiet" data-act="pick-source">Pick another source</button><button class="btn btn--quiet" data-act="next">Skip</button>`
        : st === 'jellyfin-error' ? html`<button class="btn btn--quiet" data-act="try-soulseek">Try Soulseek instead</button>`
        : st === 'stalled' || (st === 'buffering' && ps.fallback) ? html`<button class="btn btn--quiet" data-act="pick-source">Choose manually</button>`
        : st === 'ended' ? html`<button class="btn btn--quiet" data-act="start-radio-current">Start radio</button>` : '';
      var ic = err ? 'warning' : st === 'finding' || st === 'queued' || st === 'buffering' ? 'soulseek' : st === 'stalled' ? 'refresh' : ps.crossfade ? 'shuffle' : 'info';
      stEl.innerHTML = String(html`${icon(ic)}<span>${line}</span>${acts}`);
      stEl.classList.toggle('is-error', err); stEl.hidden = false;
    } else { stEl.hidden = true; stEl.innerHTML = ''; }
    // transport
    var busy = st === 'finding' || st === 'queued' || st === 'buffering' || st === 'stalled';
    $$('[data-bind="play"]').forEach(function (b) {
      var playingNow = ps.playing && !idle && st !== 'ended' && st !== 'failed';
      b.setAttribute('aria-label', playingNow ? 'Pause' : 'Play'); b.setAttribute('data-tip', playingNow ? 'Pause (Space)' : 'Play (Space)');
      $('use', b).setAttribute('href', '#i-' + (playingNow ? 'pause' : 'play'));
      $('svg', b).classList.toggle('ic-play', !playingNow);
      b.classList.toggle('is-buffering', busy && ps.playing);
    });
    $$('[data-bind="shuffle"]').forEach(function (b) { b.setAttribute('aria-pressed', String(ps.shuffle)); });
    $$('[data-bind="repeat"]').forEach(function (b) {
      b.setAttribute('aria-pressed', String(ps.repeat !== 'off'));
      var label = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat];
      b.setAttribute('aria-label', label); b.setAttribute('data-tip', label + ' (R)');
      $('use', b).setAttribute('href', '#i-' + (ps.repeat === 'one' ? 'repeat-one' : 'repeat'));
    });
    $$('[data-bind="like"]').forEach(function (b) {
      if (!t) { b.disabled = true; b.removeAttribute('data-like'); b.setAttribute('aria-pressed', 'false'); b.classList.remove('is-liked'); $('use', b).setAttribute('href', '#i-heart'); return; } b.disabled = false;
      b.setAttribute('data-like', t.id); b.setAttribute('data-id', t.id); b.removeAttribute('aria-pressed');
    });
    $$('[data-bind="save"]').forEach(function (b) {
      if (!t) { b.disabled = true; return; } b.disabled = false;
      var sv = state.saves[t.id], saved = App.isSaved(t.id), stp = $('.pbar__save-step', b);
      var label = saved ? 'Saved to library' : sv && sv.status === 'saving' ? 'Saving, step ' + sv.step + ' of 5' : sv && sv.status === 'failed' ? 'Save failed, retry' : 'Save to library (Shift+S)';
      b.setAttribute('aria-label', label); b.setAttribute('data-tip', label); b.setAttribute('data-id', t.id);
      $('use', b).setAttribute('href', '#i-' + (saved ? 'saved' : sv && sv.status === 'failed' ? 'warning' : 'save'));
      b.setAttribute('aria-pressed', String(saved));
      if (sv && sv.status === 'saving') { if (!stp) { stp = document.createElement('span'); stp.className = 'pbar__save-step'; stp.setAttribute('aria-hidden', 'true'); b.appendChild(stp); } stp.textContent = sv.step + '/5'; }
      else if (stp) stp.remove();
    });
    $$('[data-bind="mute"]').forEach(function (b) {
      var v = ps.muted ? 0 : ps.volume;
      $('use', b).setAttribute('href', '#i-' + (ps.muted ? 'mute' : v === 0 ? 'volume-0' : v < .5 ? 'volume-1' : 'volume-2'));
      b.setAttribute('aria-label', ps.muted ? 'Unmute (M)' : 'Mute (M)'); b.setAttribute('data-tip', ps.muted ? 'Unmute (M)' : 'Mute (M)');
      b.setAttribute('aria-pressed', String(ps.muted));
    });
    $$('[data-bind="xfade"]').forEach(function (el) { el.hidden = !ps.crossfade; });
    // mini-player text
    $('#mini-title').textContent = idle ? 'Nothing playing' : t.title;
    $('#mini-sub').textContent = idle ? 'Search or pick something from Home' : (line && st !== 'streaming' && st !== 'library' ? line : t.artist);
    var mt = $('#mini-thumb'); mt.innerHTML = idle ? '<span></span>' : String(ui.cover(t, 40)); mt.classList.toggle('bezel-thumb--empty', idle);
    App.syncRows();
    updateSeeks();
    updateMediaSession();
  }
  /* the format actually playing: the picked peer's file for Soulseek, the library file otherwise */
  function playingFormat(t) { return ps.peer && ps.status !== 'library' ? ps.peer.format || t.format : t.format; }
  App.on('player', renderBar);
  App.on('save', function (d) { if (d.id === P.currentId()) renderBar(); });
  App.on('like', function (d) { if (d.id === P.currentId()) App.syncRows(); });

  /* phone mini-player: swipe left/right = next/prev (buttons are the alternative) */
  (function () {
    var x0 = null;
    document.addEventListener('pointerdown', function (e) { var m = e.target.closest('#mini'); if (m && !e.target.closest('button')) x0 = e.clientX; });
    document.addEventListener('pointerup', function (e) {
      if (x0 === null) return; var dx = e.clientX - x0; x0 = null;
      if (Math.abs(dx) > 60) { if (dx < 0) P.next(); else P.prev(); }
    });
  })();

  /* default More sheet (phone 4th tab) */
  function defaultMoreSheet() {
    var links = [['#/playlists', 'playlists', 'Playlists'], ['#/liked', 'heart', 'Liked songs'], ['#/history', 'clock', 'History'],
      ['#/following', 'follow', 'Following'], ['#/stats', 'stats', 'Stats'], ['#/settings', 'settings', 'Settings'], ['#/classic', 'classic', 'Classic mode']];
    var d = App.openSheet({ title: 'more', body: html`<nav class="menu-list" aria-label="More">${links.map(function (l) {
      return html`<a class="menu__item" href="${l[0]}" data-close>${icon(l[1])}<span>${l[2]}</span></a>`;
    })}<button class="menu__item" data-act="open-activity" data-close>${icon('activity')}<span>Activity</span></button>
      <button class="menu__item" data-act="shortcuts" data-close>${icon('keyboard')}<span>Keyboard shortcuts</span></button></nav>` });
    return d;
  }

  /* default reason popover (discover screen may replace App.hooks.reason) */
  function defaultReasonPopover(anchor, r) {
    var expl = { like: 'You like songs by ' + r.name + '.',
      tag: 'This sits in ' + r.name + ', a tag you follow or play.',
      listeners: 'People who play ' + r.name + ' also play this.',
      fresh: r.name + ' released this recently, and you follow them.' }[r.kind] || '';
    App.popover(anchor, html`<div class="popover__head">why this?</div><div class="popover__body"><p>${expl}</p><p class="t-micro fg-2" style="margin-top:6px">via ${r.via || 'ListenBrainz'}</p></div>
      <div class="popover__actions"><button class="btn btn--sm" data-act="more-like" data-id="${r.item || ''}">More like this</button><button class="btn btn--sm" data-act="less-like" data-id="${r.item || ''}">Less like this</button>${r.kind === 'tag' ? '' : html`<button class="btn btn--sm" data-act="hide-artist" data-id="${r.ref}">Hide this artist</button>`}</div>`,
      { label: 'Why this was suggested' });
  }
  /* default add-to-playlist picker */
  function defaultAddToPlaylist(ids, anchor) {
    var items = state.playlists.map(function (p) {
      return { label: p.name, icon: 'playlists', run: function () { addToPlaylist(p.id, ids); } };
    });
    items.push('sep'); items.push({ label: 'New playlist', icon: 'plus', run: function () {
      App.createPlaylist({ name: 'new playlist', desc: '' }).then(function (p) { if (p) addToPlaylist(p.id, ids); });
    } });
    if (anchor) App.menu(anchor, items, { label: 'Add to playlist' });
    else App.openSheet({ title: 'add to playlist', body: html`${items.filter(function (i) { return i !== 'sep'; }).map(function (it, i) { return html`<button class="menu__item" data-pi="${i}">${icon(it.icon)}<span>${it.label}</span></button>`; })}`,
      onMount: function (el, close) { el.addEventListener('click', function (e) { var b = e.target.closest('[data-pi]'); if (b) { close(); items.filter(function (i) { return i !== 'sep'; })[+b.getAttribute('data-pi')].run(); } }); } });
  }
  /* playlists live on the server (GET/POST /api/playlists, PUT .../tracks with the full order); the store holds
     the last answer. App.savePlaylistOrder(id, trackIds) sends a new order and takes the server's playlist back. */
  function loadPlaylists() {
    return M.api('/api/playlists').then(function (res) {
      App.store.set({ playlists: Array.isArray(res) ? res : res.items || [], playlistsError: null });
    }, function (err) { App.store.set({ playlists: [], playlistsError: err.message }); });
  }
  function putPlaylist(p) {
    App.store.update('playlists', function (l) { var i = l.findIndex(function (x) { return x.id === p.id; }); if (i < 0) l.push(p); else l[i] = p; });
    return p;
  }
  App.loadPlaylists = loadPlaylists;
  App.createPlaylist = function (fields) {
    return M.api('/api/playlists', { method: 'POST', body: fields }).then(putPlaylist, function (err) {
      App.toast('Couldn\'t create the playlist: ' + err.message, { kind: 'error' }); return null;
    });
  };
  App.savePlaylistOrder = function (plId, trackIds) {
    return M.api('/api/playlists/' + encodeURIComponent(plId) + '/tracks', { method: 'PUT', body: trackIds }).then(function (p) {
      if (p && p.id) return putPlaylist(p);
      return loadPlaylists();
    });
  };
  function addToPlaylist(plId, ids) {
    ids = [].concat(ids);
    var p = M.playlist(plId); if (!p) return;
    var before = p.items.map(function (i) { return i.trackId; });
    App.savePlaylistOrder(plId, ids.concat(before)).then(function () {
      App.toast('Added to ' + p.name, { icon: 'playlists', action: 'Undo', onAction: function () { App.savePlaylistOrder(plId, before); } });
    }, function (err) { App.toast('Couldn\'t add to ' + p.name + ': ' + err.message, { kind: 'error' }); });
  }
  App.addToPlaylist = addToPlaylist;
  function defaultShortcutsDialog() {
    var groups = {};
    App.shortcuts.forEach(function (s) { (groups[s.group] = groups[s.group] || []).push(s); });
    App.openDialog({ title: 'keyboard shortcuts', wide: true, body: html`${Object.keys(groups).map(function (g) {
      return html`${ui.labelRule(g.toLowerCase(), { level: 'h3', sm: true })}<dl class="shortcut-list">${groups[g].map(function (s) {
        return html`<div class="shortcut-row"><dt>${s.keys.map(function (k, i) { return html`${i ? html`<span class="fg-2"> or </span>` : ''}${k.split('+').map(function (p) { return ui.keycap(p); })}`; })}</dt><dd>${s.label}</dd></div>`;
      })}</dl>`;
    })}` });
  }

  /* =====================================================================
     11. Actions: delegated clicks on [data-act]. App.action(name, fn(el, event)) adds or replaces one.
     ===================================================================== */
  var actions = {};
  App.action = function (name, fn) { actions[name] = fn; };
  function ids(el) { return (el.getAttribute('data-ids') || el.getAttribute('data-id') || '').split(',').filter(Boolean); }
  function listIds(el) { var l = el.closest('[data-ids]'); return l ? l.getAttribute('data-ids').split(',') : null; }
  /* start radio, then show where it landed: the Queue panel (phone: Now Playing, Queue tab) */
  function startRadio(seed) {
    P.startRadio(seed).then(function (ok) {
      if (!ok) return;
      if (window.innerWidth < 640) App.go('/now?tab=queue'); else App.openPanel('queue');
    });
  }
  /* an album's songs, fetched when only the album head is known yet (shelf tiles) */
  function albumTracks(id) {
    var a = M.album(id);
    if (a && a.trackIds.length) return Promise.resolve(a.trackIds);
    return M.api('/api/albums/' + encodeURIComponent(id)).then(function () { var x = M.album(id); return x ? x.trackIds : []; });
  }
  App.albumTracks = albumTracks;
  function withAlbum(id, fn) {
    albumTracks(id).then(function (list) {
      if (list.length) fn(list); else App.toast('This album has no songs to play yet', { icon: 'info' });
    }, function (err) { App.toast('Couldn\'t open the album: ' + err.message, { kind: 'error' }); });
  }
  function feedback(kind, id, done) {
    App.closePopover(true);
    M.api('/api/feedback', { method: 'POST', body: { kind: kind, id: id } }).then(done, function (err) {
      App.toast('Couldn\'t send that to your feed: ' + err.message, { kind: 'error' });
    });
  }
  function serverAction(path, ok) {
    App.closePopover(true);
    M.api(path, { method: 'POST' }).then(function () { App.toast(ok, { kind: 'ok' }); }, function (err) { App.toast(err.message, { kind: 'error' }); });
  }
  Object.assign(actions, {
    'noop': function () {},
    'noop-saved': function () { App.toast('Already saved to your library', { icon: 'saved' }); },
    'reload': function () { location.reload(); },
    'play-track': function (el) { var id = el.getAttribute('data-id'); P.playTrack(id, listIds(el)); },
    'play-album': function (el) {
      var id = el.getAttribute('data-id');
      withAlbum(id, function (list) { P.playList(list, 0); App.toast('Playing ' + M.album(id).title, { icon: 'play' }); });
    },
    'shuffle-album': function (el) {
      withAlbum(el.getAttribute('data-id'), function (list) { P.setShuffle(true); P.playList(list, Math.floor(Math.random() * list.length)); });
    },
    'play-playlist': function (el) {
      var p = state.playlists.filter(function (x) { return x.id === el.getAttribute('data-id'); })[0];
      if (!p.items.length) { App.toast('This playlist is empty. Drag songs here or use Add to playlist.', { icon: 'info' }); return; }
      P.playList(p.items.map(function (i) { return i.trackId; }), 0);
    },
    'play-ids': function (el) { P.playList(ids(el), 0); },
    'queue-next': function (el) { P.playNext(ids(el)); },
    'queue-add': function (el) { P.addToQueue(ids(el)); },
    'like': function (el) { App.toggleLike(el.getAttribute('data-id')); },
    'save': function (el) { var id = el.getAttribute('data-id'), s = state.saves[id]; if (s && s.status === 'failed') App.saves.retry(id); else App.saves.start(ids(el)); },
    'save-album': function (el) { withAlbum(el.getAttribute('data-id'), App.saves.start); },
    'go-album': function (el) { var t = M.track(el.getAttribute('data-id')); App.go('/album/' + (t ? t.albumId : el.getAttribute('data-id'))); },
    'follow': function (el) { App.toggleFollow(el.getAttribute('data-id')); },
    'follow-tag': function (el) {
      var id = el.getAttribute('data-id'); App.toggleFollow(id);
      el.setAttribute('aria-pressed', String(App.isFollowed(id)));
    },
    'start-radio': function (el) { startRadio({ kind: el.getAttribute('data-kind') || 'track', id: el.getAttribute('data-id') }); },
    'start-radio-current': function () { var t = P.current(); if (t) startRadio({ kind: 'track', id: t.id }); },
    'add-to-playlist': function (el) { App.hooks.addToPlaylist(ids(el), el); },
    'show-sources': function (el) { App.hooks.sourcePicker(el.getAttribute('data-id')); },
    'pick-source': function () { var t = P.current(); if (t) App.hooks.sourcePicker(t.id); },
    'source-details': function () { App.hooks.sourceDetails(); },
    'reason': function (el) {
      App.hooks.reason(el, { kind: el.getAttribute('data-kind'), ref: el.getAttribute('data-ref'), name: el.getAttribute('data-name'),
        via: el.getAttribute('data-via'), item: el.getAttribute('data-item') });
    },
    'more-like': function (el) { feedback('more-like', el.getAttribute('data-id'), function () { App.toast('Got it: more like this in your feed', { icon: 'sparkle' }); }); },
    'less-like': function (el) { feedback('less-like', el.getAttribute('data-id'), function () { App.toast('Got it: less like this', { icon: 'info' }); }); },
    'hide-artist': function (el) {
      var id = el.getAttribute('data-id'), a = M.artist(id);
      feedback('hide-artist', id, function () {
        App.store.update('hiddenArtists', function (h) { h[id] = true; });
        App.toast('Hidden ' + (a ? a.name : 'the artist') + ' from your feed', { icon: 'info' });
      });
    },
    'track-menu': function (el) {
      var id = el.getAttribute('data-id'), t = M.track(id); if (!t) return;
      App.menu(el, [
        { label: 'Play next', icon: 'queue', run: function () { P.playNext([id]); } },
        { label: 'Add to queue', icon: 'plus', run: function () { P.addToQueue([id]); } },
        { label: App.isSaved(id) ? 'In your library' : 'Save to library', icon: App.isSaved(id) ? 'saved' : 'save', run: function () { App.saves.start([id]); } },
        { label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist([id]); } },
        { label: 'Start radio', icon: 'radio', run: function () { startRadio({ kind: 'track', id: id }); } },
        'sep',
        { label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } },
        { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + t.artistId); } },
        { label: 'Show sources', icon: 'soulseek', run: function () { App.hooks.sourcePicker(id); } }
      ], { label: 'Actions for ' + (t ? t.title : 'this song') });
    },
    'player-more': function (el) {
      var t = P.current();
      if (!t) { App.toast('Nothing playing yet', { icon: 'info' }); return; }
      App.menu(el, [
        { label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist([t.id]); } },
        { label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } },
        { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + t.artistId); } },
        { label: 'Start radio', icon: 'radio', run: function () { startRadio({ kind: 'track', id: t.id }); } },
        { label: 'Show source', icon: 'soulseek', run: function () { App.hooks.sourceDetails(); } },
        { label: 'Copy link', icon: 'link', run: function () { copyText(location.href.split('#')[0] + '#/album/' + t.albumId); } },
        'sep',
        { label: 'Lyrics', icon: 'lyrics', run: function () { App.togglePanel('lyrics'); } },
        { label: 'Classic mode', icon: 'classic', run: function () { App.go('/classic'); } }
      ], { label: 'More player actions' });
    },
    'play': function () { P.toggle(); }, 'next': function () { P.next(); }, 'prev': function () { P.prev(); },
    'shuffle': function () { P.setShuffle(); }, 'repeat': function () { P.cycleRepeat(); }, 'mute': function () { P.toggleMute(); },
    'retry-search': function () { P.retry(); App.toast('Searching Soulseek again', { icon: 'refresh' }); },
    'try-soulseek': function () { P.trySoulseek(); },
    'reload-read': function (el) { var path = el.getAttribute('data-id'); if (path) M.reload(path); else { M.invalidate(); App.refresh(); } },
    'toggle-panel': function (el) { App.togglePanel(el.getAttribute('data-tab')); },
    'close-panel': function () { App.closePanel(); },
    'open-activity': function () { App.openPanel('activity'); },
    'open-now': function () { App.go('/now'); },
    'theme': function () { App.toggleTheme(); },
    'status': function (el) { statusPopover(el); },
    'shelf': function (el) { shelfPopover(el); },
    'more-sheet': function () { App.hooks.moreSheet(); },
    'shortcuts': function () { App.hooks.shortcuts(); },
    'palette': function () { App.hooks.palette(); },
    'classic': function () { App.go('/classic'); },
    'open-sources': function () { App.closePopover(); App.go('/settings/sources'); },
    'jellyfin-rescan': function () { serverAction('/api/library/rescan', 'Jellyfin rescan started'); },
    'lastfm-retry': function () { serverAction('/api/lastfm/retry', 'Sending queued scrobbles to Last.fm'); },
    'open-cache': function () { App.closePopover(); App.go('/settings/cache'); },
    'clear-search': function () { var i = $('#header-search'); i.value = ''; i.focus(); toggleClear(); },
    'back': function () { App.back(); },
    'skip': function () { $('#view').focus(); }
  });
  function copyText(text) {
    var done = function () { App.toast('Link copied', { icon: 'link' }); };
    try { navigator.clipboard.writeText(text).then(done, done); } catch (e) { done(); }
  }
  document.addEventListener('click', function (e) {
    var el = e.target.closest('[data-act]'); if (!el || el.disabled) return;
    var fn = actions[el.getAttribute('data-act')];
    if (!fn) { console.warn('No action registered for', el.getAttribute('data-act')); return; }
    if (el.tagName === 'A') e.preventDefault();
    fn(el, e);
    if (el.hasAttribute('data-close')) App.closeTop();
  });
  // a bare <a data-close> inside a sheet closes it after navigating
  document.addEventListener('click', function (e) {
    var a = e.target.closest('a[data-close]'); if (a && !a.hasAttribute('data-act')) setTimeout(App.closeTop, 0);
  });
  // double-click a row plays it
  document.addEventListener('dblclick', function (e) {
    var r = e.target.closest('.trow[data-track-id]'); if (!r || e.target.closest('button, a')) return;
    P.playTrack(r.getAttribute('data-track-id'), listIds(r));
  });

  /* =====================================================================
     12. Keyboard shortcuts (ignored while typing; disabled inside the Webamp stage)
     ===================================================================== */
  App.shortcuts = [
    { group: 'Playback', keys: ['Space'], label: 'Play or pause' },
    { group: 'Playback', keys: ['Left', 'Right'], label: 'Seek 5\u00a0s (Shift: 30\u00a0s)' },
    { group: 'Playback', keys: ['N', 'Ctrl+Right'], label: 'Next song' },
    { group: 'Playback', keys: ['P', 'Ctrl+Left'], label: 'Previous song' },
    { group: 'Playback', keys: ['Up', 'Down'], label: 'Volume up or down (outside lists)' },
    { group: 'Playback', keys: ['M'], label: 'Mute' },
    { group: 'Playback', keys: ['S'], label: 'Shuffle' },
    { group: 'Playback', keys: ['R'], label: 'Repeat: off, all, one' },
    { group: 'Song', keys: ['L'], label: 'Like the playing song' },
    { group: 'Song', keys: ['Shift+S'], label: 'Save the playing song' },
    { group: 'Panels', keys: ['Q'], label: 'Queue panel' },
    { group: 'Panels', keys: ['Y'], label: 'Lyrics panel' },
    { group: 'Panels', keys: ['W'], label: 'Classic mode' },
    { group: 'Panels', keys: ['T'], label: 'Day or night theme' },
    { group: 'Panels', keys: ['Esc'], label: 'Close panel, sheet or menu' },
    { group: 'Go to', keys: ['/'], label: 'Search' },
    { group: 'Go to', keys: ['Ctrl+K'], label: 'Command palette' },
    { group: 'Go to', keys: ['G+H', 'G+S', 'G+L', 'G+P'], label: 'Home, Search, Library, Playlists' },
    { group: 'Go to', keys: ['?'], label: 'This list' },
    { group: 'Lists', keys: ['Up', 'Down'], label: 'Move between songs' },
    { group: 'Lists', keys: ['Enter'], label: 'Play the song' },
    { group: 'Lists', keys: ['Shift+Enter'], label: 'Play next' },
    { group: 'Lists', keys: ['Alt+Up', 'Alt+Down'], label: 'Reorder (queue, playlists)' },
    { group: 'Lists', keys: ['Delete'], label: 'Remove (queue, playlists)' }
  ];
  var gPending = 0;
  function typing(el) {
    return el && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName)) && !(el.type === 'range' || el.type === 'checkbox' || el.type === 'radio');
  }
  document.addEventListener('keydown', function (e) {
    var a = document.activeElement, k = e.key, mod = e.ctrlKey || e.metaKey;
    if (k === 'Escape') {
      if (App.closePopover(true)) return;
      if (escapeTop()) return;
      var mode = $('#app').getAttribute('data-ctx');
      if (mode === 'open') { App.closePanel(); return; }
      if (App.current.route && App.current.route.view === 'now' && window.innerWidth < 640) { App.back(); return; }
      return;
    }
    if (mod && (k === 'k' || k === 'K')) { e.preventDefault(); App.hooks.palette(); return; }
    if (typing(a) || e.defaultPrevented) return;
    if (a && a.closest && a.closest('.webamp, [data-webamp]')) return;
    if (layers.length && k !== '?') return;            // sheets/dialogs own the keyboard
    var row = a && a.closest && a.closest('.trow');
    if (row) {
      var list = row.parentElement, rows = $$('.trow', list), i = rows.indexOf(row);
      if ((k === 'ArrowDown' || k === 'ArrowUp') && e.altKey) { e.preventDefault(); App.emit('row:move', { row: row, list: list, index: i, dir: k === 'ArrowDown' ? 1 : -1 }); return; }
      if (k === 'ArrowDown' || k === 'ArrowUp') {
        e.preventDefault(); var n = rows[i + (k === 'ArrowDown' ? 1 : -1)];
        if (n) { rows.forEach(function (r) { r.classList.remove('is-selected'); }); n.classList.add('is-selected'); n.focus(); }
        return;
      }
      if (k === 'Enter' && a === row) { e.preventDefault(); if (e.shiftKey) P.playNext([row.getAttribute('data-track-id')]); else P.playTrack(row.getAttribute('data-track-id'), listIds(row)); return; }
      if ((k === 'Delete' || k === 'Backspace') && a === row) { e.preventDefault(); App.emit('row:delete', { row: row, list: list, index: i }); return; }
    }
    var onControl = a && a.closest && a.closest('[role="slider"], input[type="range"], [role="tablist"], [role="menu"], [role="radiogroup"], select');
    if (gPending && Date.now() - gPending < 1200) {
      gPending = 0;
      var dest = { h: '/home', s: '/search', l: '/library', p: '/playlists' }[k.toLowerCase()];
      if (dest) { e.preventDefault(); App.go(dest); return; }
    }
    if (mod && k === 'ArrowRight') { e.preventDefault(); P.next(); return; }
    if (mod && k === 'ArrowLeft') { e.preventDefault(); P.prev(); return; }
    if (mod || e.altKey) return;
    switch (k) {
      case ' ': if (a && (a.tagName === 'BUTTON' || a.tagName === 'A')) return; e.preventDefault(); P.toggle(); break;
      case 'ArrowRight': if (onControl) return; e.preventDefault(); P.seekBy(e.shiftKey ? 30 : 5); break;
      case 'ArrowLeft': if (onControl) return; e.preventDefault(); P.seekBy(e.shiftKey ? -30 : -5); break;
      case 'ArrowUp': if (onControl) return; e.preventDefault(); P.setVolume(ps.volume + P.volumeStep()); break;
      case 'ArrowDown': if (onControl) return; e.preventDefault(); P.setVolume(ps.volume - P.volumeStep()); break;
      case 'n': case 'N': P.next(); break;
      case 'p': case 'P': P.prev(); break;
      case 'm': case 'M': P.toggleMute(); break;
      case 's': P.setShuffle(); App.toast(ps.shuffle ? 'Shuffle on' : 'Shuffle off', { icon: 'shuffle' }); break;
      case 'S': if (P.currentId()) App.saves.start([P.currentId()]); break;
      case 'r': case 'R': P.cycleRepeat(); App.toast({ off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat], { icon: ps.repeat === 'one' ? 'repeat-one' : 'repeat' }); break;
      case 'l': case 'L': if (P.currentId()) App.toggleLike(P.currentId()); break;
      case 'q': case 'Q': App.togglePanel('queue'); break;
      case 'y': case 'Y': App.togglePanel('lyrics'); break;
      case 'w': case 'W': if (App.current.path === '/classic') App.back(); else App.go('/classic'); break;
      case 't': case 'T': App.toggleTheme(); break;
      case '/': e.preventDefault(); var s = $('#header-search'); if (s && s.offsetParent) s.focus(); else App.go('/search'); break;
      case '?': e.preventDefault(); App.hooks.shortcuts(); break;
      case 'g': case 'G': gPending = Date.now(); break;
      default: return;
    }
  });

  /* =====================================================================
     13. Media Session (lock screen, OS overlays, media keys)
     ===================================================================== */
  var msKey = null;
  function updateMediaSession() {
    if (!('mediaSession' in navigator)) return;
    var t = P.current();
    try {
      if (t && msKey !== t.id) {
        msKey = t.id;
        setMetadata(t, artwork(function (s) { return 'assets/fallback-art-' + s + '.png'; }));
        /* the real cover once it is known to exist; the brand art stays when it doesn't */
        var probe = new Image(), id = t.id;
        probe.onload = function () { if (msKey === id) setMetadata(t, artwork(function (s) { return App.artUrl(t.albumId, s); })); };
        probe.src = App.artUrl(t.albumId, 96);
      }
      if (!t) { msKey = null; navigator.mediaSession.metadata = null; }
      navigator.mediaSession.playbackState = !t ? 'none' : ps.playing ? 'playing' : 'paused';
      var dur = P.duration();
      if (t && dur && navigator.mediaSession.setPositionState) navigator.mediaSession.setPositionState({ duration: dur, playbackRate: 1, position: Math.min(ps.pos, dur) });
    } catch (e) { console.warn('Media Session rejected an update', e); }
  }
  function artwork(src) {
    return [96, 256, 512].map(function (s) { return { src: new URL(src(s), location.href).href, sizes: s + 'x' + s }; });
  }
  function setMetadata(t, art) { navigator.mediaSession.metadata = new MediaMetadata({ title: t.title, artist: t.artist, album: t.album, artwork: art }); }
  App.on('player', function (k) { if (k === 'seek' || k === 'status' || k === 'play') updateMediaSession(); });
  function wireMediaSession() {
    if (!('mediaSession' in navigator)) return;
    /* the player the keys drive: the app engine, or whatever App.hooks.media is set to (Classic mode: Webamp) */
    function T() { return App.hooks.media || P; }
    var h = {
      play: function () { T().play(); }, pause: function () { T().pause(); }, stop: function () { T().pause(); T().seek(0); },
      previoustrack: function () { T().prev(); }, nexttrack: function () { T().next(); },
      seekto: function (d) { T().seek(d.seekTime); },                     // the engine clamps to what arrived, like the seek bar
      seekbackward: function (d) { T().seekBy(-(d.seekOffset || 10)); }, seekforward: function (d) { T().seekBy(d.seekOffset || 10); }
    };
    Object.keys(h).forEach(function (k) { try { navigator.mediaSession.setActionHandler(k, h[k]); } catch (e) { /* unsupported action */ } });
  }

  /* =====================================================================
     14. Drag and drop: rows (draggable) onto [data-drop] = queue | play | playlist:<id>; files onto the window
     ===================================================================== */
  var dragIds = null, chip = null;
  document.addEventListener('dragstart', function (e) {
    var r = e.target.closest && e.target.closest('[draggable="true"][data-track-id], [draggable="true"][data-album-id]');
    if (!r) return;
    var sel = $$('.trow.is-selected', r.parentElement);
    var album = r.hasAttribute('data-album-id') && M.album(r.getAttribute('data-album-id'));
    if (r.hasAttribute('data-album-id') && !(album && album.trackIds.length)) { e.preventDefault(); return; }   // songs not known yet
    dragIds = album ? album.trackIds
      : sel.length > 1 && r.classList.contains('is-selected') ? sel.map(function (x) { return x.getAttribute('data-track-id'); }) : [r.getAttribute('data-track-id')];
    e.dataTransfer.effectAllowed = 'copyMove';
    e.dataTransfer.setData('text/plain', dragIds.join(','));
    if (!chip) { chip = document.createElement('div'); chip.className = 'drag-chip'; document.body.appendChild(chip); }
    chip.innerHTML = window.icon('queue', null, 'ic-16') + (dragIds.length === 1 ? '1 song' : dragIds.length + ' songs');
    try { e.dataTransfer.setDragImage(chip, 16, 16); } catch (err) { /* old browsers */ }
    r.classList.add('is-dragging');
  });
  document.addEventListener('dragend', function () {
    dragIds = null; $$('.is-dragging, .is-drop-target').forEach(function (n) { n.classList.remove('is-dragging', 'is-drop-target'); });
  });
  var fileDepth = 0;
  function isFileDrag(e) { return e.dataTransfer && Array.prototype.indexOf.call(e.dataTransfer.types || [], 'Files') >= 0; }
  document.addEventListener('dragenter', function (e) {
    if (isFileDrag(e)) {
      fileDepth++; var o = $('#drop-overlay');
      $('.drop-overlay__label', o).textContent = App.current.path === '/classic' ? 'Drop .wsz to apply skin' : 'Drop M3U to import';
      o.classList.add('is-on'); o.hidden = false;
    }
  });
  document.addEventListener('dragleave', function (e) { if (isFileDrag(e) && --fileDepth <= 0) { fileDepth = 0; $('#drop-overlay').classList.remove('is-on'); } });
  document.addEventListener('dragover', function (e) {
    if (isFileDrag(e)) { e.preventDefault(); return; }
    var t = e.target.closest && e.target.closest('[data-drop]');
    $$('.is-drop-target').forEach(function (n) { if (n !== t) n.classList.remove('is-drop-target'); });
    if (t && dragIds) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; t.classList.add('is-drop-target'); }
  });
  document.addEventListener('drop', function (e) {
    if (isFileDrag(e)) {
      e.preventDefault(); fileDepth = 0; $('#drop-overlay').classList.remove('is-on');
      var f = e.dataTransfer.files[0]; if (f) App.hooks.importFile(f);
      return;
    }
    var t = e.target.closest && e.target.closest('[data-drop]');
    if (!t || !dragIds) return;
    e.preventDefault(); t.classList.remove('is-drop-target');
    var dest = t.getAttribute('data-drop');
    if (dest === 'queue') P.addToQueue(dragIds);
    else if (dest === 'play') { P.playList(dragIds, 0); }
    else if (dest.indexOf('playlist:') === 0) addToPlaylist(dest.slice(9), dragIds);
    else App.emit('drop', { target: t, dest: dest, ids: dragIds });
    dragIds = null;
  });

  /* =====================================================================
     15. Server events and boot: token gate (GET /api/ping), /health, the event stream, collections,
         service worker. Each source of truth is fetched once here and kept current by events.
     ===================================================================== */
  /* `service`, `stream`, `save`, `job`, `transfer` drive the engines here; every topic is also re-emitted as
     App.emit('server:<topic>', payload) for screens (search.results -> discover.js). */
  var serverHandlers = {
    service: function (e) { applyService(e.name, e.service); },
    stream: onStreamEvent,
    save: onSaveEvent,
    job: onJobEvent,
    transfer: onTransferEvent,
    resync: function () { refetchAll(); }
  };
  function onServerEvent(topic, payload) {
    if (payload && payload.entities) M.absorb(payload.entities);
    if (serverHandlers[topic]) serverHandlers[topic](payload);
    App.emit('server:' + topic, payload);
  }
  /* App.server: { reachable, version, authorized } -- read by the banners, the gate and Settings > App */
  App.server = { reachable: true, version: null, authorized: true };
  function setServer(patch) { Object.assign(App.server, patch); App.emit('server', App.server); }
  M.onUnauthorized = function () { if (App.server.authorized) setServer({ authorized: false }); };
  M.onSettled = function () { if (booted) renderRoute(true); };

  /* GET /health: services, version, reachability; also brings a closed event stream back once the server answers */
  function loadHealth() {
    return fetch(M.url('/health'), { credentials: 'same-origin' }).then(function (r) { return r.json(); }).then(function (h) {
      var changedVersion = App.server.version && App.server.version !== h.version;
      setServer({ reachable: true, version: h.version });
      if (events && events.readyState === EventSource.CLOSED && App.server.authorized) connectEvents();
      Object.keys(h.services || {}).forEach(function (k) { applyService(k, h.services[k]); });
      if (changedVersion) App.emit('pwa', { kind: 'update' });
    }, function () { setServer({ reachable: false }); });
  }
  function loadCollections() {
    return Promise.all([
      M.api('/api/likes').then(function (r) {
        var l = {}; (r.items || []).forEach(function (id) { l[id] = true; }); App.store.set('liked', l);
      }, function (err) { console.warn('Likes unavailable:', err.message); }),
      M.api('/api/follows').then(function (r) {
        var f = {}; (r.artists || []).concat(r.tags || []).forEach(function (id) { f[id] = true; }); App.store.set('followed', f);
      }, function (err) { console.warn('Follows unavailable:', err.message); }),
      loadPlaylists(), loadJobs(), loadSettings()
    ]).then(function () { App.syncRows(); });
  }
  /* Settings > Playback values the engine needs; a value may come back as { value, source: 'config' } */
  function loadSettings() {
    return M.api('/api/settings').then(function (s) {
      var pb = (s && s.playback) || {}, v = function (x) { return x && typeof x === 'object' && 'value' in x ? x.value : x; };
      App.setPlayback({ gapless: v(pb.gapless) !== false, crossfade: +v(pb.crossfade) || 0,
        normalisation: v(pb.normalisation) || 'off', radioWhenQueueEnds: v(pb.radioWhenQueueEnds) !== false, volumeStep: +v(pb.volumeStep) || 5 });
    }, function (err) { console.warn('Settings unavailable, playback uses defaults:', err.message); });
  }
  /* after a reconnect or a resync: everything the events would have told us */
  function refetchAll() {
    M.invalidate();
    loadHealth(); loadCollections();
    if (booted) renderRoute(true);
  }
  var events = null, connectedOnce = false;
  function connectEvents() {
    if (events) events.close();
    events = M.connect(onServerEvent, function () {
      if (connectedOnce) refetchAll();
      connectedOnce = true;
      setServer({ reachable: true });
    }, function () { loadHealth(); });
  }
  /* the token gate (system.js) calls this once POST /api/session has set the cookie */
  App.checkHealth = loadHealth;
  App.start = function () {
    var wasLocked = !App.server.authorized;
    setServer({ authorized: true });
    if (wasLocked) { M.invalidate(); renderRoute(true); }       // reads made behind the gate failed with 401
    connectEvents(); loadCollections();
  };
  function startServer() {
    loadHealth().then(registerWorker);
    M.api('/api/ping').then(App.start, function (err) {
      if (err.status !== 401) { setServer({ reachable: false }); App.start(); }   // offline start: the shell still works
    });
  }

  /* service worker (sw.js): the app shell for offline start; a new server version prompts a reload */
  function registerWorker() {
    if (!('serviceWorker' in navigator) || location.protocol === 'file:') return;
    navigator.serviceWorker.register('sw.js?v=' + encodeURIComponent(App.server.version || '0')).then(function (reg) {
      worker = reg;
      navigator.serviceWorker.ready.then(function (r) {          // keep what this page loaded (shell, design, fonts)
        var urls = performance.getEntriesByType('resource').map(function (x) { return x.name; }).concat([location.href.split('#')[0]]);
        if (r.active) r.active.postMessage({ type: 'cache', urls: urls });
      });
      reg.addEventListener('updatefound', function () {
        var w = reg.installing;
        w.addEventListener('statechange', function () {
          if (w.state === 'installed' && navigator.serviceWorker.controller) App.emit('pwa', { kind: 'update' });
        });
      });
    }, function (err) { console.warn('Service worker not registered:', err.message); });
  }
  var worker = null;
  navigator.serviceWorker && navigator.serviceWorker.addEventListener('controllerchange', function () { if (updating) location.reload(); });
  var updating = false;
  var installPrompt = null;
  window.addEventListener('beforeinstallprompt', function (e) { e.preventDefault(); installPrompt = e; App.emit('pwa', { kind: 'installable' }); });
  window.addEventListener('appinstalled', function () { installPrompt = null; App.emit('pwa', { kind: 'installed' }); });
  /* App.pwa: install() shows the browser's prompt (resolves true when accepted); installable(); installed();
     update() switches to the waiting version (or just reloads when only the server changed) */
  App.pwa = {
    update: function () {
      if (worker && worker.waiting) { updating = true; worker.waiting.postMessage({ type: 'skip-waiting' }); }
      else location.reload();
    },
    installable: function () { return !!installPrompt; },
    installed: function () { return matchMedia('(display-mode: standalone)').matches || navigator.standalone === true; },
    install: function () {
      if (!installPrompt) return Promise.resolve(false);
      var p = installPrompt; installPrompt = null; p.prompt();
      return p.userChoice.then(function (c) { return c.outcome === 'accepted'; });
    }
  };

  function toggleClear() { var i = $('#header-search'), c = $('#header-search-clear'); if (c) c.hidden = !i.value; }
  function wireHeader() {
    var input = $('#header-search'); if (!input) return;
    var t = null;
    input.addEventListener('input', function () {
      toggleClear(); clearTimeout(t);
      t = setTimeout(function () {
        var q = input.value.trim(), target = '/search' + (q ? '?q=' + encodeURIComponent(q) : '');
        if (App.current.path === '/search') { history.replaceState(null, '', '#' + target); renderRoute(true); } else App.go(target);
      }, 220);
    });
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); clearTimeout(t); var q = input.value.trim(); App.go('/search' + (q ? '?q=' + encodeURIComponent(q) : '')); }
      if (e.key === 'Escape') { input.value = ''; toggleClear(); input.blur(); }
    });
    App.on('route', function (r) { if (r.path === '/search' && document.activeElement !== input) { input.value = r.query.q || ''; toggleClear(); } });
  }
  function wireThemeButtons() {
    function upd() {
      var night = App.theme() === 'night';
      $$('[data-act="theme"]').forEach(function (b) {
        var u = $('use', b); if (!u) return;                    // a design's own text button on this action keeps its label
        u.setAttribute('href', '#i-' + (night ? 'moon' : 'sun'));
        b.setAttribute('aria-label', night ? 'Night theme on. Switch to day (T)' : 'Day theme on. Switch to night (T)');
        b.setAttribute('data-tip', night ? 'Switch to day (T)' : 'Switch to night (T)');
      });
    }
    App.on('prefs', upd); upd();
    if (window.matchMedia) { try { matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function () { applyPrefs(prefs); }); } catch (e) { /* old Safari */ } }
  }
  function announce(msg) { var a = $('#announcer'); if (a) { a.textContent = ''; setTimeout(function () { a.textContent = msg; }, 30); } }
  App.announce = announce;
  App.on('announce', announce);
  document.addEventListener('visibilitychange', function () { root.classList.toggle('is-hidden', document.hidden); });

  var booted = false;
  function boot() {
    renderBrand(); renderRail(); renderTray(); wireHeader(); wireThemeButtons(); layoutCtx(); renderCtx(); wireMediaSession();
    updateActivityBadge(); renderBar();
    window.addEventListener('online', refetchAll);
    var headsQueued = false;                                   // card heads survive any repaint (see ui.cardHeads)
    var mo = new MutationObserver(function () {
      if (headsQueued) return; headsQueued = true;
      Promise.resolve().then(function () { headsQueued = false; ['#view', '#ctx-body', '#overlay-host'].forEach(function (s) { ui.cardHeads($(s)); }); });
    });
    ['#view', '#ctx-body', '#overlay-host'].forEach(function (s) { mo.observe($(s), { childList: true, subtree: true, attributes: true, attributeFilter: ['data-meta'] }); });
    $('#ctx-scrim').addEventListener('click', function () { App.closePanel(); });
    document.addEventListener('pointerdown', function (e) {    // 1100-1359 overlay closes on outside click
      var app = $('#app'); if (app.getAttribute('data-ctx') !== 'open' || window.innerWidth < 1100) return;
      if (!e.target.closest('#ctx, [data-act="toggle-panel"], [data-act="open-activity"], .popover, .menu, .sheet, .dialog, .toast')) App.closePanel();
    });
    window.addEventListener('hashchange', function () { renderRoute(); });
    App.on('store', function (k) { if (k.indexOf('saves') >= 0 || k.indexOf('activity') >= 0) { updateActivityBadge(); if (state.ctxTab === 'activity') renderCtx(); } });
    booted = true;
    renderRoute();
    startServer();
    App.emit('ready');
  }
  /* boot once the DOM is parsed and the design's design.js (if any) has registered its slots */
  document.addEventListener('DOMContentLoaded', function () { Designs.ready(boot); });
})();
