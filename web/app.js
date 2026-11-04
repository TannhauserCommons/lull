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
  /* $$ over the whole page for what the clock and the player bar update several times a second: querySelectorAll walks
     every element (~1.4 ms each on an 834-song playlist's 34k), so each answer is kept until the page's structure or
     an attribute a selector could match on changes. takeRecords() makes that exact within the same task. */
  var kept = {}, keptWatch = new MutationObserver(function () { kept = {}; });
  keptWatch.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'id', 'data-bind', 'data-seek'] });
  function $$kept(sel) {
    if (keptWatch.takeRecords().length) kept = {};
    return kept[sel] || (kept[sel] = $$(sel));
  }
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

  /* App.patch(el, markup): make el's contents match markup by changing only what differs, instead of rebuilding them.
     Every re-render goes through it, so nothing that stayed the same is replaced: an <img> whose src is unchanged keeps
     its decoded picture (a new one would reload and fade in again), rows keep their place, focus, hover and scroll,
     and text changes in place. Children are matched by key (id, data-key, data-track-id, data-id), the rest by position
     and tag. Returns el. */
  function patch(el, markup) {
    var tpl = document.createElement('template');
    tpl.innerHTML = String(markup == null ? '' : markup);
    patchChildren(el, decorated(el, tpl.content));
    pruneFills();
    return el;
  }
  /* where the page decorates what screens render (card heads, see ui.cardHeads), the new markup gets the same
     decoration first, so the comparison is like for like and the decoration isn't stripped and redrawn each time.
     All of it stays in the template's own document, which loads nothing: in the page's, every picture of the new
     markup would be fetched, even those the patch then drops for the one on show (a cover the hub hasn't yet: asked
     again on every render) */
  function decorated(el, content) {
    if (!el.closest('#view, #ctx-body, #overlay-host')) return content;
    var inert = content.ownerDocument;
    var host = inert.importNode(el, false), box = inert.createDocumentFragment();   // a copy of el: its own head counts too
    host.appendChild(content); box.appendChild(host);
    ui.cardHeads(box);
    return host;
  }
  function nodeKey(n) {
    if (n.nodeType !== 1) return null;
    var k = n.getAttribute('id') || n.getAttribute('data-key') || n.getAttribute('data-track-id') || n.getAttribute('data-id');
    return k ? n.nodeName + '#' + k : null;
  }
  /* The cursor passes old nodes already placed and keyed ones nothing wants any more (they go at the end): stopping at
     one put every later node in front of it, so a song leaving the top of a list (the queue moving on, a row removed)
     moved all the rows after it, 5,000 removals and insertions, each restyled and laid out again */
  function patchChildren(parent, next) {
    var keyed = {}, wants = {}, used = new Set(), cur = parent.firstChild, n, k;
    for (n = parent.firstChild; n; n = n.nextSibling) { k = nodeKey(n); if (k) (keyed[k] = keyed[k] || []).push(n); }
    for (n = next.firstChild; n; n = n.nextSibling) { k = nodeKey(n); if (k) wants[k] = (wants[k] || 0) + 1; }
    for (var want = next.firstChild; want; want = n) {
      n = want.nextSibling;
      while (cur && (used.has(cur) || ((k = nodeKey(cur)) && !wants[k]))) cur = cur.nextSibling;
      k = nodeKey(want);
      if (k) wants[k]--;
      var match = k ? (keyed[k] || []).shift() : unkeyedMatch(cur, want, used);
      if (match) {
        used.add(match);
        if (match === cur) cur = cur.nextSibling; else parent.insertBefore(match, cur);
        patchNode(match, want);
      } else {
        used.add(want);
        parent.insertBefore(want, cur);         // moves it out of the template
      }
    }
    for (n = parent.firstChild; n; n = k) { k = n.nextSibling; if (!used.has(n)) parent.removeChild(n); }
  }
  /* the next unclaimed, unkeyed old node from the cursor, if it is the same kind of node. A loading block (aria-busy:
     every skeleton) is never the content that replaces it: morphing one into the other made the content look as if it
     had moved from where the skeleton's bars were (layout shift) */
  function unkeyedMatch(cur, want, used) {
    for (var n = cur; n; n = n.nextSibling) {
      if (used.has(n) || nodeKey(n)) continue;
      return n.nodeName === want.nodeName && busy(n) === busy(want) ? n : null;
    }
    return null;
  }
  function busy(n) { return n.nodeType === 1 && n.getAttribute('aria-busy') === 'true'; }
  function patchNode(old, want) {
    if (old.nodeType !== 1) { if (old.nodeValue !== want.nodeValue) old.nodeValue = want.nodeValue; return; }
    if (old.nodeName === 'IMG' && old.getAttribute('src') === want.getAttribute('src')) return;   // same picture: leave it be
    if (old.nodeName === 'IMG' && old.hasAttribute('data-pic') && old.getAttribute('data-pic') === want.getAttribute('data-pic')) return;   // a cover's size is the fitter's
    patchAttrs(old, want);
    if (want.hasAttribute('data-fill')) { watchRow(old); if (old.firstChild) fillRow(old); else holdPlace(old); return; }   // a lazy row: see ui.row
    if (old.nodeName === 'TEXTAREA') { if (old !== document.activeElement) old.value = want.value; return; }
    patchChildren(old, want);
    if (old.nodeName === 'SELECT') old.value = want.value || (want.querySelector('option[selected]') || {}).value || old.value;
  }
  /* by name, never through el.attributes: reading that makes an Attr node for each attribute, which the element then
     keeps for as long as it lives (an empty row of a 5,000-song list held eleven nodes and ~15 KB that way) */
  function patchAttrs(old, want) {
    var i, v, names = want.getAttributeNames();
    for (i = 0; i < names.length; i++) { v = want.getAttribute(names[i]); if (old.getAttribute(names[i]) !== v) old.setAttribute(names[i], v); }
    names = old.getAttributeNames();
    /* data-fit-out is the row fitter's (layout.js), not the markup's: a re-render that kept the words would show a part
       that gave way again, with nothing to fit the row anew */
    for (i = names.length - 1; i >= 0; i--) if (!want.hasAttribute(names[i]) && names[i] !== 'data-fit-out') old.removeAttribute(names[i]);
    if (old.nodeName === 'INPUT') {                // properties the user may have changed since the last render
      if (old.checked !== want.checked) old.checked = want.checked;
      if (old.type !== 'file' && old !== document.activeElement && old.value !== want.value) old.value = want.value;
    }
  }

  App.html = html; App.raw = raw; App.esc = esc; App.icon = icon; App.$ = $; App.$$ = $$; App.patch = patch;
  function fmtSize(mb) {                    // "30.2 MB" / "1.4 GB"
    mb = +mb || 0;
    return mb >= 1000 ? (Math.round(mb / 100) / 10) + ' GB' : (mb >= 100 ? Math.round(mb) : Math.round(mb * 10) / 10) + ' MB';
  }
  function fmtRate(mbs) {                   // "1.4 MB/s" / "48 KB/s": a slow peer is never "0 MB/s"
    mbs = +mbs || 0;
    return mbs > 0 && mbs < 0.1 ? Math.round(mbs * 1000) + ' KB/s' : Math.round(mbs * 10) / 10 + ' MB/s';
  }
  function fmtClock(sec) {                  // "00:36:12" (album heads, media-player LCDs)
    sec = Math.max(0, Math.round(sec || 0));
    return [Math.floor(sec / 3600), Math.floor(sec / 60) % 60, sec % 60].map(function (n) { return String(n).padStart(2, '0'); }).join(':');
  }
  /* one line of file facts for a track (or a source { format, sizeMB }):
     formatLine(t) -> "FLAC :: 44.1 kHz, 1040 kbps, 30.2 MB"; { bits: true } adds "16-bit, " before the rate.
     Only what is known: "" for a song whose file isn't known yet, never a bare separator */
  /* the hub's sample rate is a float (44.099998...): one decimal at most */
  function kHz(v) { return v ? Math.round(v * 10) / 10 : v; }
  function formatLine(t, opts) {
    if (!t) return '';
    var f = M.formats[t.format] || {}, o = opts || {}, parts = [];
    var khz = kHz(t.khz || f.khz), kbps = t.kbps || f.kbps, bits = t.bits !== undefined ? t.bits : f.bits;
    if (khz) parts.push((o.bits && bits ? bits + '-bit, ' : '') + khz + ' kHz');
    if (kbps) parts.push(kbps + ' kbps');
    if (t.sizeMB) parts.push(fmtSize(t.sizeMB));
    return [f.codec || String(t.format || '').split(' ')[0], parts.join(', ')].filter(Boolean).join(' :: ');
  }
  /* album facts: "Tracks: 8  Duration: 00:36:12  Size: 250 MB" */
  function albumLine(a) {
    return a ? 'Tracks: ' + a.trackIds.length + '  Duration: ' + fmtClock(a.duration) + (a.sizeMB ? '  Size: ' + fmtSize(a.sizeMB) : '') : '';
  }
  App.fmt = { time: fmtTime, date: fmtDate, length: fmtLength, size: fmtSize, rate: fmtRate, clock: fmtClock, formatLine: formatLine, albumLine: albumLine };
  App.storage = storage; App.data = M; App.clamp = clamp; App.uid = nextId;

  /* =====================================================================
     2. Events + store
     ===================================================================== */
  var listeners = {};
  App.on = function (name, fn) {
    if (name.indexOf('server:') === 0) M.listen(name.slice(7));     // the event stream carries what someone listens for
    (listeners[name] = listeners[name] || []).push(fn);
    return function off() { listeners[name] = (listeners[name] || []).filter(function (f) { return f !== fn; }); };
  };
  App.emit = function (name, detail) {
    (listeners[name] || []).slice().forEach(function (fn) {
      try { fn(detail); } catch (e) { console.error('[' + name + ' listener]', e); }
    });
  };

  /* the four services the tray shows; /health and `service` events fill them in (state off = not reported) */
  var SERVICE_LABELS = { soulseek: 'soulseek', library: 'library', catalog: 'catalog', lastfm: 'last.fm', cache: 'cache' };
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
    ctxWant: null,                 // the panel tab the listener chose, shown again whenever it is available
    crossfadeSec: 0,
    privateListening: false        // this device's switch on the hub (PATCH /api/devices/me): its plays are recorded nowhere
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
  /* likes and follows change at once and are sent to the server (through the outbox while it can't be reached,
     offline.js); a refusal puts them back and says why */
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
      return App.outbox.call('/api/likes/' + encodeURIComponent(id), { method: on ? 'PUT' : 'DELETE' }, { op: 'like', trackId: id, liked: on });
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
    /* following an artist sends its name: the server derives the id from it and refuses a mismatch */
    var body = on && kind === 'artist' ? { name: name } : undefined;
    App.outbox.call('/api/follows/' + kind + '/' + encodeURIComponent(id), { method: on ? 'PUT' : 'DELETE', body: body },
      { op: 'follow', kind: kind, id: id, name: kind === 'artist' ? name : undefined, followed: on }).catch(function (err) {
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
    return { theme: prefs.theme || 'system', motion: prefs.motion || 'system', textSize: +prefs.textSize || 100, design: Designs.current, pauseInactive: !!prefs.pauseInactive };
  };
  /* keys: theme ('day'|'night'|null), motion ('reduce'|'full'|'system'), textSize (90|100|112|125), design (name),
     pauseInactive (true: decorative loops hold still while lull isn't the active window) */
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
     ctx = { params, query, el (the view), onCleanup(fn) }. ':name?' marks an optional segment.
     need(params, query): code the route's screen loads on first use (docs/context/web.md, Code loads where it is used):
     a promise while some is still to load (App.needScript, App.needHub, App.needClassic), nothing once it is here. */
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
  /* replace keeps the history entry (and so where its page was, below) */
  App.go = function (path, opts) {
    if (opts && opts.replace) history.replaceState(history.state, '', '#' + path); else location.hash = path;
    if (opts && opts.replace) renderRoute();
  };
  App.back = function () { if (history.length > 1) history.back(); else App.go('/home'); };
  /* re-render the current route (screens call this after their own state changes) */
  App.refresh = function () { renderRoute(true); };

  /* code most visits never run loads on first use: index.html ships Classic mode (webamp-session.js, skins.js,
     classic.js and classic.css) at start only in the desktop app, whose crash recovery needs classic.js from the start.
     Elsewhere App.needClassic() loads them, once, the scripts in order; it resolves when they have run (they register
     their route, App.classicSettings and the .wsz import, as they do when loaded at start) and the styles are in */
  var CLASSIC = ['desktop/webamp-session.js', 'screens/skins.js', 'screens/classic.js'], classicLoad = null;
  App.needClassic = function () {
    if (App.classicSettings && window.LullWebamp && !classicRoute().need) return Promise.resolve();
    return classicLoad || (classicLoad = Promise.all([loadStyle('screens/classic.css', 'Classic mode'),
      CLASSIC.reduce(function (p, src) { return p.then(function () { return loadScript(src, 'Classic mode'); }); }, Promise.resolve())])
      .then(null, function (err) { classicLoad = null; throw err; }));
  };
  function classicRoute() { return routes.filter(function (r) { return r.pattern === '/classic'; })[0] || {}; }
  function unloaded(what) { return new Error('Couldn\'t load ' + what + '. Check your connection and try again.'); }
  /* in the screens layer, as index.html's own screen styles are */
  function loadStyle(href, what) {
    return new Promise(function (ok, fail) {
      var el = document.createElement('style'); el.textContent = '@import url(' + href + ') layer(screens);';
      el.onload = ok; el.onerror = function () { el.remove(); fail(unloaded(what)); };
      document.head.appendChild(el);
    });
  }
  function loadScript(src, what) {
    return new Promise(function (ok, fail) {
      var s = document.createElement('script'); s.src = src;
      s.onload = ok; s.onerror = function () { s.remove(); fail(unloaded(what)); };
      document.head.appendChild(s);
    });
  }
  /* a screen of its own file (a route's `need`), with its stylesheet if it has one: loaded once, and resolved when both
     are in, so it never shows unstyled; tried again after a failed fetch */
  var scriptLoads = {};
  App.needScript = function (src, what, style) {
    return scriptLoads[src] || (scriptLoads[src] = Promise.all([style && loadStyle(style, what), loadScript(src, what)])
      .then(null, function (err) { delete scriptLoads[src]; throw err; }));
  };
  /* the equaliser's face (screens/eq.js: Settings > Playback, the player's "…" > Equaliser) loads the same way; its
     settings and the sound (eq.js) are with the player from the start */
  var eqLoad = null;
  App.needEq = function () {
    if (App.eq) return Promise.resolve();
    return eqLoad || (eqLoad = loadScript('screens/eq.js', 'the equaliser').then(null, function (err) { eqLoad = null; throw err; }));
  };
  /* the hub's own settings (screens/hub.js, hub.css): the library source, the Soulseek account, Settings > Devices, and
     the parts the pairing gate and first-run setup are built from (App.hubKit). The routes that show them ask for it
     (their `need`), and the gate as it opens */
  App.needHub = function () {
    return App.hubKit ? Promise.resolve() : App.needScript('screens/hub.js', 'the hub\'s settings', 'screens/hub.css');
  };
  /* opening a screen shouldn't wait for its code (Classic's first open took ~150 ms more with the CPU slowed 6x, plus
     the fetches): it loads as the pointer or focus reaches a way in (or, for Classic, the menu that lists it opens,
     playerMenu). [where the pointer goes, what it loads]; each fires once */
  var SOON = [
    ['[data-act="classic"]', function () { return App.needClassic(); }],
    ['a[href^="#/settings/library"], a[href^="#/settings/devices"], a[href^="#/settings/sources"]', function () { return App.needHub(); }],
    ['a[href^="#/stats"]', function () { return App.needStats(); }]
  ];
  function soon(e) {
    if (!e.target.closest) return;
    SOON = SOON.filter(function (s) {
      if (!e.target.closest(s[0])) return true;
      s[1]().then(null, function () {});    // a failed fetch is tried again, and reported, when the screen opens
      return false;
    });
    if (!SOON.length) { document.removeEventListener('pointerover', soon, true); document.removeEventListener('focusin', soon, true); }
  }
  document.addEventListener('pointerover', soon, true); document.addEventListener('focusin', soon, true);
  /* and for the W key, which gives no warning: once the page has settled and the browser is idle, well after the start's
     own reads and covers, so it never competes with them */
  App.on('ready', function () {
    setTimeout(function () {
      var go = function () { App.needClassic().then(null, function () {}); };
      if (window.requestIdleCallback) requestIdleCallback(go, { timeout: 10000 }); else go();
    }, 15000);
  });
  /* until classic.js replaces it: the router loads Classic mode, then renders the real route */
  App.route('/classic', { title: 'Classic mode', need: App.needClassic, render: function () { return ''; } });
  /* a file opened by drop or by an Import button: a .wsz becomes the Classic skin (skins.js), anything else the
     library's import (library.js); Classic mode loads first when it isn't here yet */
  App.openFile = function (file) { App.openFiles([file]); };
  /* several at once (a drop): every .wsz is kept as a skin, and the library imports the first other file (one
     playlist at a time) */
  App.openFiles = function (files) {
    var skins = files.filter(function (f) { return /\.wsz$/i.test(f.name); });
    var other = files.filter(function (f) { return skins.indexOf(f) < 0; })[0];
    if (other) App.hooks.importFile(other);
    if (!skins.length) return;
    App.needClassic().then(function () { App.classicSkin.keepFiles(skins); }, function (err) { App.toast(err.message, { kind: 'error', icon: 'warning' }); });
  };

  /* ---------- where each page was: Back returns to it ----------
     Every history entry gets an id (history.state.lullEntry). Leaving an entry's page records where it was under that
     id: the scroll position, the element at the top third of the view and its offset (rows off screen keep an estimated
     height, so the element places the page more truly than the pixels), and the focused row or control. A page shown
     again for that entry (Back, Forward, a reload: kept in sessionStorage) goes back there; a new entry starts at the
     top. What a read adds after the first render is waited for, until the listener scrolls, types or 4 s pass. Every
     page gets this from the router; a screen that rewrites its URL in place passes history.state on. */
  var spots = {}, spotOrder = [], shownEntry = null, returnTo = null, entrySeq = 0;
  try { var keptSpots = JSON.parse(sessionStorage.getItem('lull:spots')); if (keptSpots && keptSpots.order) { spots = keptSpots.spots; spotOrder = keptSpots.order; } } catch (e) { /* storage off */ }
  function entryId() {
    var st = history.state && typeof history.state === 'object' ? history.state : null;
    if (st && st.lullEntry) return st.lullEntry;
    var id = Date.now().toString(36) + '-' + (++entrySeq);
    try { history.replaceState(Object.assign({}, st, { lullEntry: id }), ''); } catch (e) { return null; }
    return id;
  }
  /* an element named so it can be found again in a fresh render: the nearest of it and its parents with an
     identifying attribute, and which of the page's matches it is */
  var IDENTITY = ['data-track-id', 'data-place', 'data-album-id', 'data-fk', 'id'];
  function markOf(el, view) {
    for (var n = el; n && n !== view; n = n.parentElement) {
      for (var i = 0; i < IDENTITY.length; i++) {
        var v = n.getAttribute(IDENTITY[i]);
        if (v) { var sel = '[' + IDENTITY[i] + '="' + CSS.escape(v) + '"]'; return { sel: sel, n: $$(sel, view).indexOf(n) }; }
      }
    }
    return null;
  }
  function findMark(m, view) { return m && $$(m.sel, view)[m.n] || null; }
  var ANCHOR_AT = [[.4, 1 / 3], [.15, 1 / 3], [.65, 1 / 3], [.4, .45], [.15, .45], [.65, .45]];
  function recordSpot(entry, view) {
    if (!entry || !view.firstElementChild) return;
    var box = view.getBoundingClientRect(), a = document.activeElement, spot = { top: view.scrollTop };
    /* a few points a third down: the first may fall in a gap between tiles */
    for (var i = 0; i < ANCHOR_AT.length && !spot.anchor; i++) {
      var at = document.elementFromPoint(box.left + box.width * ANCHOR_AT[i][0], box.top + box.height * ANCHOR_AT[i][1]);
      var m = at && view.contains(at) && at !== view ? markOf(at, view) : null, el = findMark(m, view);
      if (el) { spot.anchor = m; spot.offset = Math.round(el.getBoundingClientRect().top - box.top); }
    }
    if (a && a !== view && view.contains(a)) {
      spot.focus = markOf(a, view);
      spot.href = a.getAttribute('href'); spot.act = a.getAttribute('data-act'); spot.selected = a.classList.contains('is-selected');
    }
    if (!spots[entry]) spotOrder.push(entry);
    spots[entry] = spot;
    while (spotOrder.length > 60) delete spots[spotOrder.shift()];
    try { sessionStorage.setItem('lull:spots', JSON.stringify({ spots: spots, order: spotOrder })); } catch (e) { /* storage off or full */ }
  }
  /* put the page back where it was, as far as its content allows yet. It holds it there while it waits (4 s, until the
     listener scrolls, types or clicks), so a section or picture that arrives above doesn't push the page away */
  function settleSpot() {
    var r = returnTo, view = $('#view'); if (!r) return;
    if (r.entry !== shownEntry || Date.now() > r.until) { returnTo = null; return; }
    var el = findMark(r.anchor, view);
    if (el) { placeAnchor(view, el, r.offset); fixAnchor(view, el, r.offset); } else view.scrollTop = r.top;
    if (r.focus) {
      var f = findMark(r.focus, view);
      if (f && !(f.tabIndex >= 0 && f.matches('a[href], button, [tabindex]'))) f = $(r.href ? '[href="' + CSS.escape(r.href) + '"]' : r.act ? '[data-act="' + r.act + '"]' : 'a[href], button', f);
      if (f) { f.focus({ preventScroll: true }); if (r.selected) f.classList.add('is-selected'); r.focus = null; }
    }
  }
  /* the anchor back at its offset from the top of the view. Rows off screen have only an estimated height until they
     are drawn, so the first placing is close and the next frames, with the rows around it drawn, make it exact */
  function placeAnchor(view, el, offset) {
    var d = el.getBoundingClientRect().top - view.getBoundingClientRect().top - offset;
    if (Math.abs(d) >= 1) view.scrollTop += d;
  }
  var anchorFix = null;
  function fixAnchor(view, el, offset) {
    var job = anchorFix = {};
    (function step(n) {
      requestAnimationFrame(function () {
        if (anchorFix !== job || !el.isConnected) return;
        placeAnchor(view, el, offset);
        if (n > 1) step(n - 1); else anchorFix = null;
      });
    })(4);
  }
  /* content that grows without a re-render (a screen patching its own part, search results streaming in) settles it
     too, and the "Playing from" reveal below, while they wait */
  var spotSizes = new ResizeObserver(function () { settleSpot(); settleReveal(); if (!returnTo && !reveal) spotSizes.disconnect(); });
  function awaitSpot(view) { spotSizes.disconnect(); if ((returnTo || reveal) && view.firstElementChild) spotSizes.observe(view.firstElementChild); }
  /* the listener's own scroll, key or click takes over from both */
  function stopReturning() { returnTo = null; anchorFix = null; reveal = null; spotSizes.disconnect(); }
  ['wheel', 'touchstart', 'keydown', 'pointerdown'].forEach(function (type) { document.addEventListener(type, stopReturning, { capture: true, passive: true }); });

  /* "Playing from" (App.goPlaying): the place the song plays from, opened at the song, which is scrolled to the middle,
     focused and briefly lit (.is-revealed). Its rows may come with the page's read: the reveal waits for the row until
     6 s pass or the listener goes elsewhere. Radio and the listener's own queue have no page: the queue opens. */
  var reveal = null;
  App.goPlaying = function () {
    var c = P.playingFrom(); if (!c) return;
    var path = App.placePath(c);
    if (!path) { showQueue(); return; }
    reveal = { path: path.split('?')[0], trackId: P.currentId(), until: Date.now() + 6000 };
    if (location.hash === '#' + path) settleReveal(); else App.go(path);
  };
  function settleReveal() {
    var r = reveal, view = $('#view'); if (!r) return;
    if (Date.now() > r.until || App.current.path !== r.path) { reveal = null; return; }
    var row = $('.trow.is-playing', view) || $('[data-track-id="' + CSS.escape(r.trackId) + '"]', view);
    if (!row) return;
    stopReturning();
    row.scrollIntoView({ block: 'center' });
    if (row.tabIndex >= 0) row.focus({ preventScroll: true });
    row.classList.remove('is-revealed'); void row.offsetWidth; row.classList.add('is-revealed');
    setTimeout(function () { row.classList.remove('is-revealed'); }, 1800);
  }

  var lastPath = null;
  function renderRoute(isRefresh) {
    var loc = parseHash(), route = null, params = {};
    var entry = isRefresh ? shownEntry : entryId(), arrived = !isRefresh && entry !== shownEntry;
    if (arrived) recordSpot(shownEntry, $('#view'));   // the page leaving is still in the view
    for (var i = 0; i < routes.length; i++) {
      var m = routes[i].re.exec(loc.path);
      if (m) { route = routes[i]; route.keys.forEach(function (k, j) { if (m[j + 1] !== undefined) params[k] = decodeURIComponent(m[j + 1]); }); break; }
    }
    /* a route whose code loads on first use (route.need): the page showing stays until it has run, then the route
       renders as if it had always been here (the screen it replaced its stub with, or the one it now has the code for) */
    var waiting = route && route.need && route.need(params, loc.query);   // a promise while there is code to load
    if (waiting) {
      waiting.then(function () { renderRoute(isRefresh); }, function (err) {
        App.toast(err.message, { kind: 'error', icon: 'warning' }); App.back();
      });
      return;
    }
    // global flags (not persisted): theme, motion, design
    if (loc.query.theme || loc.query.motion) { applyFlags(loc.query); applyPrefs(prefs); }
    if (!isRefresh && loc.query.design && loc.query.design !== Designs.current && Designs.valid(loc.query.design)) App.setDesign(loc.query.design, true);
    if (!isRefresh) M.nextRoute();                    // reads made by the previous route stop re-rendering

    cleanups.forEach(function (fn) { try { fn(); } catch (e) { console.error(e); } }); cleanups = [];
    var view = $('#view'), same = isRefresh && lastPath === loc.path;
    /* ctx.refresh: this render updates the page already showing (mount runs again on the same elements, so one-off
       effects check it); ctx.on(target, type, fn, opts) listens until the next render */
    var ctx = { params: params, query: loc.query, el: view, route: route, refresh: same,
      onCleanup: function (fn) { cleanups.push(fn); },
      on: function (target, type, fn, opts) {
        target.addEventListener(type, fn, opts);
        cleanups.push(function () { target.removeEventListener(type, fn, opts); });
      } };
    App.current = { path: loc.path, params: params, query: loc.query, route: route };

    var markup;
    if (!route) markup = notFound(loc.path);
    else { try { markup = route.render(ctx); } catch (e) { console.error(e); markup = renderError(e); } }
    if (same) patch(view, markup || ''); else view.innerHTML = String(markup || '');   // a new page starts fresh
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
    var spot = arrived && !(reveal && reveal.path === loc.path) && spots[entry];
    if (!isRefresh && (lastPath !== loc.path || spot)) {
      view.scrollTop = 0;
      if (lastPath !== null && !(route && route.keepFocus)) view.focus({ preventScroll: true });
    }
    if (arrived) { shownEntry = entry; anchorFix = null; returnTo = spot ? Object.assign({ entry: entry, until: Date.now() + 4000 }, spot) : null; }
    lastPath = loc.path;
    App.syncRows();
    App.emit('route', App.current);
    settleSpot(); settleReveal();
    if (arrived) awaitSpot(view);
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
  /* songs drawn without a picture (an import's line, a playlist song lull has no match for, a Last.fm-only result,
     file tags): each is asked about once a session (POST /api/catalog/resolve, discovery.md#bare-songs). The answer's
     entities, and `catalog.tracks` for the songs the hub resolves later, bring the catalog's song and cover; covers then
     repaint. Every cover goes through here, so no screen can show a song lull could picture without asking. */
  var resolveAsked = new Set(), resolveWaiting = [], resolveTimer = null, RESOLVE_BATCH = 200;
  function askResolve(t) {
    if (!t || !/^t-/.test(t.id || '') || resolveAsked.has(t.id) || App.isSaved(t.id)) return;
    var a = t.albumId && M.album(t.albumId);
    if (a && a.art) return;
    resolveAsked.add(t.id); resolveWaiting.push(t.id);
    if (!resolveTimer) resolveTimer = setTimeout(sendResolve, 300);
  }
  function sendResolve() {
    resolveTimer = null;
    var batch = resolveWaiting.splice(0, RESOLVE_BATCH);
    if (resolveWaiting.length) resolveTimer = setTimeout(sendResolve, 300);
    if (!batch.length) return;
    M.api('/api/catalog/resolve', { method: 'POST', body: { trackIds: batch } }).then(function (res) {
      if (res && res.entities && Object.keys(res.entities.tracks || {}).length) repaintCovers();
    }, function () { batch.forEach(function (id) { resolveAsked.delete(id); }); });   // asked again when next drawn
  }
  /* the route and the player bar again, at most twice a second, once entities brought covers */
  var repaintTimer = null;
  function repaintCovers() {
    if (repaintTimer) return;
    repaintTimer = setTimeout(function () { repaintTimer = null; App.refresh(); renderBar(); }, 500);
  }
  ui.cover = function (item, size, opts) {
    opts = opts || {}; size = size == null ? 48 : size;
    if (item && typeof item === 'object' && item.albumId) askResolve(item);
    var key = typeof item === 'string' ? item : (item.albumId || item.id);
    var px = typeof size === 'number';
    var cls = 'cover ' + App.coverClass(key) + (px && size < 40 ? ' cover--xs' : px && size <= 64 ? ' cover--thumb' : '') +
      (opts.round ? ' cover--round' : '') + (opts.cls ? ' ' + opts.cls : '');
    var a11y = opts.label ? ' role="img" aria-label="' + esc(opts.label) + '"' : ' aria-hidden="true"';
    var c = { key: String(key), recipe: App.coverClass(key), size: px ? size + 'px' : size, cls: cls, attrs: a11y,
      round: !!opts.round, label: opts.label || '', blurred: !!opts.blurred, hero: !!opts.hero };
    return raw(String(App.slot('coverArt', c, coverMarkup)));
  };
  /* default coverArt slot: one span; the .c1-.c8 recipe draws the art in CSS until (and unless) the real cover
     loads: <img class="cover__img"> from /api/art, and .has-art on the span once it has (see the load listener).
     { blurred: true }: drawn blurred behind something (the artist hero's glow), so its smallest size is all it shows;
     { hero: true }: the page's main picture (Now Playing, a page's header, the player bar), asked for first and at once */
  function coverMarkup(c) { return coverSpan(c.cls, ' style="--cover-size-base:' + c.size + '"' + c.attrs, c.key, c.size, null, c); }
  /* a picture this page has already loaded renders as shown from the start (.has-art, no lazy load), so a cover drawn
     again (another screen, a new row) shows its art at once instead of fading in over the recipe each time */
  var artReady = new Set();
  /* `own`: a picture of the item's own (a playlist's cover, or a blob being chosen) instead of the key's; o: { blurred, hero } */
  function coverSpan(cls, attrs, key, size, own, o) {
    o = o || {};
    var pic = own ? ownPic(own) : artPic(key), src = pic && !artWaits(pic) ? firstSrc(pic, size) : pic ? '' : own || '';
    if (o.blurred && pic) { src = artWaits(pic) ? '' : sized(pic, ART_SIZES[0]); pic = ''; }   // not fitted
    var ready = artReady.has(src), mono = own ? '' : monogram(key), wait = pic && !src;
    var photo = mono && !pic && !o.blurred;   // an artist drawn without a photo: the screen asks the hub for one
    return '<span class="' + cls + (mono ? ' cover--artist' : '') + (ready ? ' has-art' : '') + '"' + attrs +
      (wait ? ' data-wait-art="' + esc(key) + '" data-wait-pic="' + esc(pic) + '"' + (o.hero ? ' data-wait-hero' : '') : '') +
      (photo ? ' data-want-photo="' + esc(key) + '"' + (o.hero ? ' data-wait-hero' : '') : '') + '>' +
      (mono ? '<span class="cover__mono" aria-hidden="true">' + esc(mono) + '</span>' : '') + (src ? coverImg(key, pic, src, ready, o.hero) : '') + '</span>';
  }
  function coverImg(key, pic, src, ready, hero) {
    var load = ready ? '' : hero ? ' fetchpriority="high" decoding="async"' : ' loading="lazy" decoding="async"';
    return '<img class="cover__img"' + load + ' alt="" data-art="' + esc(key) + '"' + (pic ? ' data-pic="' + esc(pic) + '"' : '') + ' src="' + esc(src) + '">';
  }
  /* an artist's initial: what an artist without a photo shows, set in the design's display face over its recipe
     (never an album cover standing in for a photo); hidden once a photo loads */
  function monogram(key) {
    if (!/^ar-/.test(key)) return '';
    var a = M.artist(key), name = a && a.name ? a.name.trim().replace(/^the\s+/i, '') : '';
    return (Array.from(name)[0] || '').toLocaleUpperCase();
  }
  /* ---------- covers the hub doesn't have yet ----------
     A cover answered 404 is one the hub is still fetching ("not yet") or has none of (api.md /api/art). Every cover of
     that picture then waits as its span alone (data-wait-art, data-wait-pic), the recipe showing, until `art.ready` names
     its key (the hub has fetched it): then each gets its <img> and asks once. Never an <img> with no src: one that leaves
     the page stays pending in the browser, and keeps the whole screen it was part of in memory. The wait is the picture's,
     not the album's: an album that gains a catalog picture meanwhile asks for that at once. Nothing retries on a timer, so
     a page asks for a picture once, whatever number of rows show it. A wait ends by itself after ART_WAIT: one more request
     may go out then (a "none" is answered from the browser's cache), and a reconnect ends them all, since events may have
     been missed. */
  var artWaiting = {}, ART_WAIT = 2 * 60 * 1000;
  function artWaits(pic) { return Date.now() - (artWaiting[pic] || 0) < ART_WAIT; }
  App.on('server:art.ready', function (p) {
    var images = (p && p.images) || {};
    Object.keys(images).forEach(function (key) { photoCameIn(key, images[key]); });
    ((p && p.keys) || []).forEach(artCameIn);
  });
  /* an artist's photo the hub vetted after the page drew it without one: the entity takes it, and every cover drawn for
     the artist without a photo gets its <img> (the monogram goes once it loads), as a waiting cover does */
  function photoCameIn(key, image) {
    var a = M.artist(key); if (!a) return;
    a.image = image;
    var pic = artPic(key); if (!pic) return;
    $$('[data-want-photo="' + key + '"]').forEach(function (span) {
      var hero = span.hasAttribute('data-wait-hero');
      span.removeAttribute('data-want-photo'); span.removeAttribute('data-wait-hero');
      unwatchShown(span);
      span.insertAdjacentHTML('beforeend', coverImg(key, pic, firstSrc(pic, span.style.getPropertyValue('--cover-size-base').trim()), false, hero));
    });
  }
  function artCameIn(key) {
    $$('[data-wait-art="' + key + '"]').forEach(function (span) { delete artWaiting[span.getAttribute('data-wait-pic')]; unwait(span); });
    $$('img[data-art="' + key + '"]').forEach(function (img) {
      var pic = img.getAttribute('data-pic'); if (!pic) return;
      delete artWaiting[pic];
      ART_SIZES.forEach(function (s) { delete artFailed[sized(pic, s)]; });
      refit(img);
    });
  }
  function forgetArtWaits() {
    artWaiting = {}; artFailed = {};
    $$('[data-wait-art]').forEach(unwait);
    screenSent = ''; reportScreen();   // a restarted hub knows no screen
  }
  /* a waiting cover (its span) gets its <img> back, at the size its markup asked for; the fitter takes it from there */
  function unwait(span) {
    var key = span.getAttribute('data-wait-art'), pic = span.getAttribute('data-wait-pic'), hero = span.hasAttribute('data-wait-hero');
    span.removeAttribute('data-wait-art'); span.removeAttribute('data-wait-pic'); span.removeAttribute('data-wait-hero');
    unwatchShown(span);   // its <img> is watched instead
    span.insertAdjacentHTML('beforeend', coverImg(key, pic, firstSrc(pic, span.style.getPropertyValue('--cover-size-base').trim()), false, hero));
  }
  /* a cover whose picture 404'd: its <img> goes, the span waits (see above) */
  function toWait(img) {
    var span = img.parentNode; if (!span) return;
    span.setAttribute('data-wait-art', img.getAttribute('data-art')); span.setAttribute('data-wait-pic', img.getAttribute('data-pic'));
    if (img.hasAttribute('fetchpriority')) span.setAttribute('data-wait-hero', '');
    watchShown(span);
    img.remove();
  }
  /* the picture for an album or artist key, without a size: a catalog entity's own URL (album `art`, artist `image`, same
     origin through /api/catalog/art, ADR-0008), else the library's /api/art for albums */
  function artPic(key) {
    var e = /^ar-/.test(key) ? M.artist(key) : /^al-/.test(key) ? M.album(key) : null;
    var own = e && (e.art || e.image);
    if (!own && !/^al-/.test(key)) return '';
    if (!own && e && e.saved === 'none') return '';   // library art exists only for saved albums: asking would 404
    return own || M.url('/api/art/' + encodeURIComponent(key));
  }
  function ownPic(url) { return /^(blob|data):/.test(url) ? '' : M.url(url); }   // a blob being chosen is shown as it is
  function sized(url, px) { return /[?&]size=/.test(url) ? url.replace(/([?&]size=)\d+/, '$1' + px) : url + (url.indexOf('?') < 0 ? '?' : '&') + 'size=' + px; }
  App.artUrl = function (albumId, size) { return sized(M.url('/api/art/' + encodeURIComponent(albumId)), size); };

  /* ---------- covers at the size they are drawn ----------
     The hub serves every picture at ART_SIZES, scaled down and never up (api.md /api/art). A cover asks for the smallest
     size that covers its drawn box in device pixels, so it is as sharp as the screen can show and no larger, at any
     display scaling and in any design (designs resize covers in CSS, so the size a screen passes is only a first guess).
     - Markup: every cover <img> carries data-pic, its picture without a size. Its first src (firstSrc) is the size its
       CSS size needs when that is a number of pixels; a picture this page already decoded, when there is one, so a big
       view never starts blank; else 256 px until the fitter has measured it. Never none: a lazy <img> with no src
       stays pending in the browser after it leaves the page, and keeps the whole screen it was part of in memory.
     - The fitter: a ResizeObserver measures each cover in device pixels (and again when it, the zoom or the screen
       changes). A larger size is decoded off the page and swapped in, so the picture on show stays until the sharp one
       is ready; a smaller one replaces a big picture a small place borrowed. Swaps wait until the cover is near the
       viewport, so nothing large decodes off screen.
     - App.patch leaves a cover <img> with the same data-pic alone: its src is the fitter's. */
  var ART_SIZES = [96, 256, 512, 1024, 2048];
  /* a picture up to 10% short of its box isn't visibly softer, and the next size up has 2.7 to 7 times the pixels */
  var ART_SLACK = 0.9;
  /* picture -> its full size in pixels (long side), once an answer came back smaller than asked: no larger size is asked */
  var artFull = {};
  function devicePixels() { return window.devicePixelRatio || 1; }
  /* the size to ask for a box `px` device pixels wide, never past the size that holds the whole source */
  function artBucket(px, pic) {
    var full = artFull[pic], top = ART_SIZES[ART_SIZES.length - 1];
    for (var i = 0; i < ART_SIZES.length; i++) {
      if (full && ART_SIZES[i] >= full) return ART_SIZES[i];
      if (ART_SIZES[i] >= px * ART_SLACK) return ART_SIZES[i];
    }
    return top;
  }
  function bucketOf(src) { var m = /[?&]size=(\d+)/.exec(src || ''); return m ? +m[1] : 0; }
  /* the largest size of `pic` already decoded, no larger than `most` (any size when 0) */
  function decodedOf(pic, most) {
    for (var i = ART_SIZES.length - 1; i >= 0; i--) {
      var s = sized(pic, ART_SIZES[i]);
      if ((!most || ART_SIZES[i] <= most) && artReady.has(s)) return s;
    }
    return '';
  }
  function firstSrc(pic, size) {
    var px = typeof size === 'number' ? size : /^\d+(\.\d+)?(px)?$/.test(size) ? parseFloat(size) : 0;
    var want = px ? sized(pic, artBucket(px * devicePixels(), pic)) : '';
    if (want && artReady.has(want)) return want;
    return decodedOf(pic, bucketOf(want)) || want || sized(pic, 256);
  }
  /* an answer smaller than the size asked (250 counts for 256: the archive's thumbnail) means the source has no more */
  function noteFull(pic, src, w, h) {
    var asked = bucketOf(src), long = Math.max(w, h);
    if (pic && asked && long && long < asked * 0.95) artFull[pic] = long;
  }
  /* CSS pixels × devicePixelRatio: zoom changes the CSS size, and a screen with other scaling is caught by onScaling */
  var fitter = new ResizeObserver(function (entries) {
    entries.forEach(function (e) { fitArt(e.target, Math.max(e.contentRect.width, e.contentRect.height) * devicePixels()); });
  });
  function watchArt(img) { fitter.observe(img); }
  function refit(img) { fitter.unobserve(img); watchArt(img); }   // a new observation reports the size at once
  function fitArt(img, px) {
    if (!img.isConnected) { fitter.unobserve(img); return; }   // removed covers report 0 once
    var pic = img.getAttribute('data-pic'); if (!pic || !px) return;   // hidden: fitted once shown
    var w = img.naturalWidth, h = img.naturalHeight;
    if (w && h) px *= Math.max(w, h) / Math.min(w, h);   // object-fit: cover fills the box with the short side
    var size = artBucket(px, pic), src = img.getAttribute('src'), now = bucketOf(src);
    if (now === size || (artFull[pic] && Math.min(now, size) >= artBucket(Infinity, pic))) return;   // both hold the whole source
    var want = sized(pic, size);
    if (artWaits(pic) || Date.now() - (artFailed[want] || 0) < ART_WAIT) return;   // until art.ready
    if (!src || artReady.has(want) || !artReady.has(src)) { img.setAttribute('src', want); return; }   // nothing on show yet
    artWanted.set(img, want); nearby.observe(img);
  }
  /* swaps wait for the cover to come near the viewport; then the new size is decoded, and swapped in if still wanted */
  var artWanted = new WeakMap(), artFailed = {};
  var nearby = new IntersectionObserver(function (entries) {
    entries.forEach(function (e) {
      if (!e.isIntersecting) return;
      nearby.unobserve(e.target);
      var img = e.target, want = artWanted.get(img), pic = img.getAttribute('data-pic');
      if (!want) return;
      var im = new Image(); im.src = want;
      im.decode().then(function () {
        artReady.add(want); noteFull(pic, want, im.naturalWidth, im.naturalHeight);
        if (artWanted.get(img) === want) { artWanted.delete(img); if (img.isConnected) img.setAttribute('src', want); }
      }, function () {
        /* the hub is fetching this size ("not yet"): the smaller picture stays until art.ready names the key */
        artFailed[want] = Date.now();
        if (artWanted.get(img) === want) artWanted.delete(img);
      });
    });
  }, { rootMargin: '50%' });
  /* every cover <img> put on the page is fitted; one whose picture changed in place (App.patch) is fitted again */
  new MutationObserver(function (records) {
    records.forEach(function (r) {
      if (r.type === 'attributes') { if (r.target.hasAttribute('data-pic')) refit(r.target); return; }
      r.addedNodes.forEach(function (n) {
        if (n.nodeType !== 1) return;
        if (n.matches('img[data-pic]')) watchArt(n); else if (n.firstElementChild) n.querySelectorAll('img[data-pic]').forEach(watchArt);
        eachShown(n, watchShown);
      });
      r.removedNodes.forEach(function (n) { if (n.nodeType === 1) eachShown(n, unwatchShown); });
    });
  }).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ['data-pic'] });

  /* ---------- what is on screen, for the hub (PUT /api/pictures/screen, ADR-0033) ----------
     The hub fetches outside pictures in the order screens show them: what plays, then what is visible, then what is
     near, newest screen first, top to bottom, and drops work for pictures that left every screen. So a long list scrolled
     fast fills the screen the listener stops on first. Two IntersectionObservers keep which pictures are visible and
     which are within a screen of view: a cover's <img>, a cover waiting for its picture (data-wait-pic) and an artist
     drawn without a photo (data-want-photo, size 0: the hub looks for one). Each pair watches one scroll container (the
     page, or the box a list scrolls in, as the lazy rows' watchers do): with the page as their root, a list scrolling in
     its own box would clip "near" away. A change is reported at most four times a second, the last one when it settles;
     an unchanged screen isn't sent again. */
  var SHOWN = 'img[data-pic], [data-wait-pic], [data-want-photo]', SCREEN_EVERY = 250, SCREEN_MAX = 400;
  var shownVisible = new Set(), shownNear = new Set(), screenGen = Date.now() * 1000, screenSent = '', screenTimer = null, screenLast = 0;
  var shownWatchers = new Map();   // scroll container (null: the page) -> { visible, near } observers
  function shownWatcher(into, root, margin) {
    return new IntersectionObserver(function (entries) {
      entries.forEach(function (e) { if (e.isIntersecting && e.target.isConnected) into.add(e.target); else into.delete(e.target); });
      reportScreen();
    }, { root: root, rootMargin: margin });
  }
  /* a picture's scroll container, found once per list (its rows' covers share it) */
  function shownRoot(el) {
    var list = el.closest('ul, ol'), root = list && list.__shownRoot;
    if (root === undefined || (root && !root.isConnected)) { root = scrollerOf(el.parentElement); if (list) list.__shownRoot = root; }
    return root;
  }
  function watchShown(el) {
    var root = shownRoot(el), w = shownWatchers.get(root);
    if (!w) {
      shownWatchers.forEach(function (old, r) { if (r && !r.isConnected) { old.visible.disconnect(); old.near.disconnect(); shownWatchers.delete(r); } });
      w = { visible: shownWatcher(shownVisible, root, '0px'), near: shownWatcher(shownNear, root, '100% 0px') };
      shownWatchers.set(root, w);
    }
    el.__shownWatch = w; w.visible.observe(el); w.near.observe(el);
  }
  function unwatchShown(el) {
    var w = el.__shownWatch; if (w) { w.visible.unobserve(el); w.near.unobserve(el); el.__shownWatch = null; }
    var was = shownNear.has(el);
    shownVisible.delete(el); shownNear.delete(el);
    if (was) reportScreen();
  }
  function eachShown(node, fn) {
    if (node.matches(SHOWN)) fn(node);
    if (node.firstElementChild) node.querySelectorAll(SHOWN).forEach(fn);
  }
  function reportScreen() {
    if (!screenTimer) screenTimer = setTimeout(sendScreen, Math.max(0, screenLast + SCREEN_EVERY - Date.now()));
  }
  function sendScreen() {
    screenTimer = null;
    if (document.hidden) return;   // reported again when the page shows
    var visible = shownList(shownVisible), seen = {};
    visible.forEach(function (p) { seen[p[0]] = 1; });
    var near = shownList(shownNear).filter(function (p) { return !seen[p[0]]; });
    var room = SCREEN_MAX - visible.length, now = JSON.stringify([visible, near.slice(0, Math.max(0, room))]);
    if (now === screenSent) return;
    screenSent = now; screenLast = Date.now();
    M.api('/api/pictures/screen', { method: 'PUT', body: { gen: ++screenGen, visible: visible.slice(0, SCREEN_MAX), near: near.slice(0, Math.max(0, room)) } })
      .catch(function () { screenSent = ''; });   // sent again with the next change
  }
  /* the pictures in `set` as [route/id, size], in page order (top to bottom), each once at the largest size drawn */
  function shownList(set) {
    var els = [];
    set.forEach(function (el) { if (el.isConnected) els.push(el); else set.delete(el); });
    els.sort(function (a, b) { return a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1; });
    var out = [], at = {};
    els.forEach(function (el) {
      var p = shownPicture(el); if (!p) return;
      if (at[p[0]] == null) { at[p[0]] = out.length; out.push(p); } else if (p[1] > out[at[p[0]]][1]) out[at[p[0]]][1] = p[1];
    });
    return out;
  }
  function shownPicture(el) {
    if (el.hasAttribute('data-want-photo')) return ['catalog/art/' + el.getAttribute('data-want-photo'), 0];
    var pic = el.getAttribute(el.tagName === 'IMG' ? 'data-pic' : 'data-wait-pic'), size;
    if (!pic) return null;
    size = el.tagName === 'IMG' ? bucketOf(artWanted.get(el) || el.getAttribute('src'))
      : bucketOf(firstSrc(pic, el.style.getPropertyValue('--cover-size-base').trim()));
    var m = /(?:^|\/)api\/((?:catalog\/)?art\/(?:al|ar|pls)-[0-9a-f]+)(?:[?#]|$)/.exec(pic);
    return m && size ? [m[1], size] : null;
  }
  document.addEventListener('visibilitychange', function () { if (!document.hidden) { screenSent = ''; reportScreen(); } });
  /* moving to a screen with another scaling changes no CSS size: fit every cover again */
  (function onScaling() {
    matchMedia('(resolution: ' + devicePixels() + 'dppx)').addEventListener('change', function () {
      $$('img[data-pic]').forEach(refit); onScaling();
    }, { once: true });
  })();

  /* fetch and decode a track's cover ahead of time (the player warms the next song's) at the sizes the playing song's
     covers are drawn at now, so it shows at once when drawn */
  ui.warmArt = function (t) {
    var pic = t && artPic(t.albumId), cur = P.current(); if (!pic) return;
    var sizes = cur ? $$('img[data-art="' + cur.albumId + '"]').map(function (i) { return bucketOf(i.getAttribute('src')); }).filter(Boolean) : [];
    (sizes.length ? sizes : [artBucket(52 * devicePixels(), pic)]).forEach(function (px) {
      var src = sized(pic, artBucket(px, pic)); if (artReady.has(src)) return;
      var im = new Image(); im.src = src;
      im.decode().then(function () { artReady.add(src); noteFull(pic, src, im.naturalWidth, im.naturalHeight); }, function () { /* no cover: the recipe stays */ });
    });
  };
  document.addEventListener('load', function (e) {
    var img = e.target; if (img.tagName !== 'IMG' || !img.classList.contains('cover__img')) return;
    var src = img.getAttribute('src'), pic = img.getAttribute('data-pic');
    artReady.add(src); img.parentNode.classList.add('has-art');
    noteFull(pic, src, img.naturalWidth, img.naturalHeight);
    if (pic && img.naturalWidth !== img.naturalHeight) refit(img);   // a wide photo fills its box by its short side
  }, true);
  document.addEventListener('error', function (e) {
    var img = e.target; if (img.tagName !== 'IMG' || !img.classList.contains('cover__img')) return;
    var pic = img.getAttribute('data-pic');
    if (!pic) { img.remove(); return; }   // a blurred or chosen picture: the recipe shows
    artWaiting[pic] = Date.now();
    $$('img[data-art="' + img.getAttribute('data-art') + '"]').forEach(function (i) { if (i.getAttribute('data-pic') === pic && !i.naturalWidth) toWait(i); });
  }, true);
  /* 2x2 mosaic of the first four distinct album covers (playlists); one cover when the songs share one album, or when
     `own` is a picture of the playlist's own (its stored cover, or a picture being chosen). `.mosaic--one` keeps the
     frame every design styles, with one cover filling it. opts: { hero } as for ui.cover. */
  ui.mosaic = function (trackIds, size, label, own, opts) {
    var seen = {}, keys = [];
    trackIds.forEach(function (id) {
      var t = M.track(id); if (!t || seen[t.albumId] || keys.length >= 4) return;
      seen[t.albumId] = 1; keys.push(t.albumId); askResolve(t);
    });
    var one = !!own || keys.length === 1;
    if (own) keys = ['own-' + own];
    while (!one && keys.length < 4) keys.push('pad-' + keys.length);
    var px = typeof size === 'number' ? size + 'px' : size, each = typeof size === 'number' ? (one ? size : size / 2) : '100%';
    return raw('<span class="mosaic' + (one ? ' mosaic--one' : '') + '" style="--cover-size-base:' + px + '"' + (label ? ' role="img" aria-label="' + esc(label) + '"' : ' aria-hidden="true"') + '>' +
      keys.map(function (k) {
        var c = { key: k, recipe: App.coverClass(k), size: '100%', cls: 'cover ' + App.coverClass(k), attrs: '', round: false, label: '', mosaic: true };
        return String(App.slot('coverArt', c, function (x) { return coverSpan(x.cls, '', x.key, each, own, opts); }));
      }).join('') + '</span>');
  };
  /* a playlist's picture: its own cover (GET /api/playlists/{id}/cover, sized like album art), else its songs' mosaic */
  ui.playlistArt = function (p, size, label, opts) {
    return ui.mosaic((p.items || []).map(function (i) { return i.trackId; }), size, label, p.cover || null, opts);
  };

  ui.savedBadge = function () { return html`<span class="badge badge-saved">${icon('saved')}Saved</span>`; };
  ui.soulseekBadge = function () { return html`<span class="badge badge-slsk">${icon('soulseek')}Soulseek</span>`; };
  /* Saved / Soulseek badge for a track; flips in place when a save completes (App.syncRows).
     { button: true } (the player bar) opens source details, as the quality chip beside it does */
  ui.sourceBadge = function (trackOrId, opts) {
    var id = typeof trackOrId === 'string' ? trackOrId : trackOrId.id;
    var saved = App.isSaved(id), inner = saved ? ui.savedBadge() : ui.soulseekBadge();
    if (opts && opts.button) return html`<button class="src-badge" data-act="source-details" data-src-badge="${id}" data-was="${saved ? 'saved' : 'slsk'}" ${fitAttr(opts)} aria-label="${saved ? 'Saved' : 'From Soulseek'}. Show source details" data-tip="Source details">${inner}</button>`;
    return html`<span class="src-badge" data-src-badge="${id}" data-was="${saved ? 'saved' : 'slsk'}">${inner}</span>`;
  };
  /* opts.fit: the order a part gives way in, in a row that fits itself (layout.js, data-fit) */
  function fitAttr(opts) { return opts && opts.fit ? raw('data-fit="' + (+opts.fit) + '"') : ''; }
  /* quality chip: outlined by default; { solid: true } for player bar / device; { button: true } opens source details */
  ui.qualityChip = function (fmt, opts) {
    opts = opts || {};
    var f = M.formats[fmt] || { label: fmt, lossless: false };
    /* a YouTube copy's own chip ("YouTube · Opus 160", docs/context/youtube.md): never a peer file's format */
    var cls = 'q-chip ' + (f.lossless ? 'q-chip--ll' : 'q-chip--ly') + (opts.solid ? ' q-chip--solid' : '') + (/^YouTube /.test(String(fmt)) ? ' q-chip--yt' : '');
    var label = (f.lossless ? 'Lossless, ' : '') + f.label;
    if (opts.button) return html`<button class="${cls}" data-act="source-details" ${opts.id ? raw('data-id="' + esc(opts.id) + '"') : ''} ${fitAttr(opts)} aria-label="${label}. Show source details" data-tip="Source details">${f.label}</button>`;
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

  /* section label: lowercase text + hairline rule + optional quiet action { label, href | act } or end markup (a status) */
  ui.labelRule = function (text, opts) {
    opts = opts || {}; var tag = opts.level || 'h2';
    var action = opts.action ? (opts.action.href
      ? html`<a class="btn btn--quiet btn--sm" href="${opts.action.href}">${opts.action.label}</a>`
      : html`<button class="btn btn--quiet btn--sm" data-act="${opts.action.act}">${opts.action.label}</button>`)
      : opts.end ? html`<span class="label-rule__end">${opts.end}</span>` : '';
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
     queueIndex, actions (default true), unavailable, badges (markup placed before the source badge), artistEnd (markup
     at the end of the artist line), album (the album page's id: its cover stands in for an unknown one), lazy (see
     ui.row) } */
  /* the playing bars: in the playing row only (App.syncRows moves them), never as hidden markup in every row */
  var EQ = '<span class="eq" aria-hidden="true"><i></i><i></i><i></i></span>';
  function playingBars(row, on) {
    var num = $(':scope > .trow__num', row), bars = num && $(':scope > .eq', num);
    if (on && num && !bars) num.insertAdjacentHTML('beforeend', EQ); else if (!on && bars) bars.remove();
  }
  ui.trackRow = function (track, opts) {
    opts = opts || {}; if (typeof track === 'string') track = M.track(track);
    if (!track) return '';
    var playing = App.player && App.player.currentId() === track.id;
    var cls = 'trow' + (opts.dense ? ' trow--dense' : '') + (playing ? ' is-playing' : '') + (opts.unavailable ? ' is-unavailable' : '');
    var drag = opts.draggable === false ? '' : ' draggable="true"';
    var n = opts.index != null ? opts.index : track.no;
    /* file facts as data (data-khz / data-kbps / data-bits / data-size in MB) + a hidden .trow__fmt line a design may show */
    var facts = raw(' data-khz="' + (kHz(track.khz) || '') + '" data-kbps="' + (track.kbps || '') + '" data-bits="' + (track.bits || '') + '" data-size="' + (track.sizeMB || '') + '"');
    var open = html`<li class="${cls}" data-track-id="${track.id}" ${opts.queueIndex != null ? raw('data-queue-index="' + opts.queueIndex + '"') : ''}${facts} tabindex="0"${raw(drag)} aria-label="${track.title} by ${track.artist}"`;
    return ui.row(open, function () {
      var saved = App.isSaved(track.id), line = formatLine(track), bars = App.player && App.player.currentId() === track.id ? raw(EQ) : '';
      return html`
      <div class="trow__num"><span class="trow__n">${n}</span><button class="trow__play" data-act="play-track" data-id="${track.id}" tabindex="-1" aria-label="Play ${track.title}">${icon('play', null, 'ic-16')}</button>${bars}</div>
      <div class="trow__art">${ui.cover(rowCover(track, opts.album), opts.dense ? 40 : 48)}</div>
      <div class="trow__main">
        <button class="trow__title" data-act="play-track" data-id="${track.id}" tabindex="-1">${track.title}</button>
        <div class="trow__artist"><span class="trow__mini" data-src-mini="${track.id}">${icon(saved ? 'saved' : 'soulseek', saved ? 'Saved' : 'Soulseek')}</span><a href="#/artist/${track.artistId}" tabindex="-1">${track.artist}</a>${opts.artistEnd || ''}</div>
        ${line ? html`<div class="trow__fmt" aria-hidden="true">${line}</div>` : ''}
      </div>
      <div class="trow__album"><a href="#/album/${track.albumId}" tabindex="-1">${track.album}</a></div>
      ${opts.added ? html`<div class="trow__added">${fmtDate(opts.added, true)}</div>` : ''}
      <div class="trow__badges"><span class="dl-mark" data-dl-mark="${track.id}" ${App.downloads && App.downloads.has(track.id) ? raw('data-on') : ''}>${App.downloads && App.downloads.has(track.id) ? icon('downloaded', 'Downloaded', 'ic-16') : ''}</span>${opts.badges || ''}${opts.dup === 'better' ? ui.dupBetter() : opts.dup ? ui.dupNote() : ''}${ui.sourceBadge(track)}${track.format ? ui.qualityChip(track.format) : ''}${opts.showSources && sourceCount(track) ? html`<button class="trow__sources" data-act="show-sources" data-id="${track.id}" tabindex="-1" aria-label="${sourceCount(track)} sources for ${track.title}. Pick a source" data-tip="Pick a source">${sourceCount(track)} sources</button>` : ''}${opts.plays != null ? html`<span class="trow__plays">${opts.plays === 1 ? '1 play' : opts.plays + ' plays'}</span>` : ''}<span class="trow__rate">${rateLine(track)}</span></div>
      <div class="trow__like">${ui.likeButton(track.id)}</div>
      <div class="trow__dur">${track.duration ? fmtTime(track.duration) : '–'}</div>
      <div class="trow__more">${opts.actions === false ? '' : html`<button class="btn-icon btn-icon--bare" data-act="track-menu" data-id="${track.id}" aria-haspopup="menu" aria-label="More actions for ${track.title}" data-tip="More">${icon('more')}</button>`}</div>
    `;
    }, opts.lazy);
  };
  /* Rows of a long list draw what's inside them only near the screen. The <li> is always there, with its classes,
     data, label and tabindex, so selection, the keyboard, dragging, App.syncRows and scrolling to a song work on every
     row as before; its contents (~40 elements and five icons a track row, the bulk of a long list's memory: an 835-song
     playlist with its queue open held 200k DOM nodes) are made from the state of the moment when it comes within a
     screen or so of view, and dropped again once it is well out of it.
     ui.row(open, inner, lazy): `open` is the row's start tag without its ">", `inner()` its contents. lazy (what
     ui.lazyRow(n, count) says of a list's row): 'now', a long list's row drawn at once, then like the others; 'near',
     drawn only near the screen; falsy, a short list's row, always drawn. Every row of a long list is watched the same
     way, so none is ever skipped by content-visibility and each measures as it is drawn (holdPlace). */
  var fills = new Map(), fillSeq = 0, fillsChecked = 0, LAZY_MIN = 60, LAZY_HEAD = 24;
  ui.row = function (open, inner, lazy) {
    if (!lazy) return html`${open}>${inner()}</li>`;
    var k = String(++fillSeq); fills.set(k, inner);
    return lazy === 'near' ? html`${open} data-fill="${k}"></li>` : html`${open} data-fill="${k}">${inner()}</li>`;
  };
  /* a long list's first rows are drawn at once: they are the ones on screen when it opens */
  ui.lazyRow = function (n, count) { return count > LAZY_MIN && (n >= LAZY_HEAD ? 'near' : 'now'); };
  /* the same over several lists that read as one (an album's discs, History's days): next() for each row in turn */
  ui.lazyRows = function (count) { var n = 0; return function next() { return ui.lazyRow(n++, count); }; };
  function fillRow(li) {
    var inner = fills.get(li.getAttribute('data-fill'));
    if (!inner) return;
    var tpl = document.createElement('template');
    tpl.innerHTML = String(inner());
    if (li.firstChild) patchChildren(li, tpl.content); else li.appendChild(tpl.content);
  }
  /* An empty row holds the height it has drawn, so a list away from the screen is as long as it is drawn, and a jump
     (to the playing song, back to a spot) lands where it stays: the height it had when it left, or, never drawn, the
     height of a drawn row of its list. The design's min-height it had before was 2-4 px short of a drawn row in some
     designs, which moved a row jumped to as the rows above it were drawn. A row whose title wraps on a phone is a line
     taller than its list's: that is known only once it has been drawn. Inline, since nothing in CSS knows it; patchNode
     puts it back after a re-render's attributes. */
  var heldHeight = new WeakMap();
  function hold(li, h) { if (h) heldHeight.set(li, h); }   // a hidden row (an import's filter) measures 0: nothing to hold
  function holdPlace(li) { var h = heldHeight.get(li); if (h) li.style.height = h + 'px'; }
  function listRowHeight(li, drawn) {
    var list = li.parentNode;
    if (!drawn.has(list)) {
      var first = list.querySelector(':scope > li[data-fill].' + CSS.escape(li.classList[0]) + ':not(:empty)');
      drawn.set(list, first ? first.getBoundingClientRect().height : 0);
    }
    return drawn.get(list);
  }
  /* a row keeps its contents while something in it has focus, is being dragged or has its menu open */
  function inUse(li) { return li.contains(document.activeElement) || li.classList.contains('is-dragging') || !!li.querySelector('[aria-expanded="true"]'); }
  /* one IntersectionObserver per scroll container (null: the page) gives and takes a lazy row's contents. Watching
     rows was measured against `contentvisibilityautostatechange`, which the rows' content-visibility makes free: about
     the same CPU, but ~110 MB more kept heap after scrolling 2,500 songs */
  var watchers = new Map();
  /* the list itself, or the nearest box around it that scrolls (an import's lines scroll in their own box) */
  function scrollerOf(el) {
    for (var p = el; p && p !== document.body; p = p.parentElement) {
      var o = getComputedStyle(p).overflowY;
      if (o === 'auto' || o === 'scroll') return p;
    }
    return null;
  }
  function watcherFor(li) {
    var list = li.parentElement, root = list && list.__rowScroller;
    if (root === undefined || (root && !root.isConnected)) root = list.__rowScroller = scrollerOf(list);
    var w = watchers.get(root);
    if (!w) {
      w = new IntersectionObserver(function (entries) {
        var drawn = new Map();   // list -> its drawn row height, read once a callback (layout is current here)
        entries.forEach(function (e) {
          var li = e.target;
          if (!li.hasAttribute('data-fill')) return;
          if (e.isIntersecting) { if (!li.firstChild) { li.style.removeProperty('height'); fillRow(li); } }
          else if (li.firstChild) { if (!inUse(li)) { hold(li, e.boundingClientRect.height); li.textContent = ''; holdPlace(li); } }
          else if (!heldHeight.has(li)) { hold(li, listRowHeight(li, drawn)); holdPlace(li); }
        });
      }, { root: root, rootMargin: '100% 0px' });
      watchers.set(root, w);
    }
    return w;
  }
  function watchRow(li) { if (li.isConnected && !li.__rowWatch) { li.__rowWatch = watcherFor(li); li.__rowWatch.observe(li); } }
  function eachLazy(node, fn) {
    if (node.nodeType !== 1) return;
    if (node.hasAttribute('data-fill')) fn(node);
    else if (node.firstElementChild) node.querySelectorAll('li[data-fill]').forEach(fn);
  }
  new MutationObserver(function (records) {
    records.forEach(function (r) {
      r.removedNodes.forEach(function (n) { eachLazy(n, function (li) { if (!li.isConnected && li.__rowWatch) { li.__rowWatch.unobserve(li); li.__rowWatch = null; } }); });
      r.addedNodes.forEach(function (n) { eachLazy(n, watchRow); });
    });
  }).observe(document.documentElement, { childList: true, subtree: true });
  /* every render makes a new maker for each lazy row it draws (App.patch looks after each); those of rows no longer on
     the page go. One made since the last look may belong to markup not on the page yet, so it waits for the next:
     markup with lazy rows must reach the page in the task that made it, or soon after, not be kept for later */
  function pruneFills() {
    if (fillSeq === fillsChecked || fills.size < 4 * LAZY_MIN) return;   // nothing made since the last look
    var live = new Set(), mark = fillsChecked;
    document.querySelectorAll('li[data-fill]').forEach(function (li) { live.add(li.getAttribute('data-fill')); });
    fills.forEach(function (v, k) { if (+k <= mark && !live.has(k)) fills.delete(k); });
    fillsChecked = fillSeq;
  }
  /* A place to play from, wherever it shows (tiles, rail playlists, Home shelves, search results): its root carries
     data-place="kind:id" (App.placeKey) and ui.placeCls(key); ui.placeMark(key) goes at the start of its
     [data-place-at] part (a tile's title) or, without one, at its end. While it is the place playing (P.playingFrom),
     it is .is-playing-place, .is-playing-now while sound plays, and shows the playing bars: the same .eq as a playing
     row, still while paused. Rendered with the state of the moment; App.syncRows keeps them current. */
  function placePlaying(key) { return !!App.player && !!key && App.placeKey(App.player.playingFrom()) === key; }
  var PLACE_MARK = '<span class="place-mark">' + EQ + '<span class="vh">(playing)</span></span>';
  ui.placeCls = function (key) { return placePlaying(key) ? ' is-playing-place' + (App.player.state.playing ? ' is-playing-now' : '') : ''; };
  /* the expand button over a cover that opens it large (App.viewCover): shown on hover and focus, the way Spotify's
     arrow is; a touch screen has no hover, and its "…" menus have View cover */
  ui.coverZoom = function (trackId) {
    return html`<button class="cover-zoom__btn" data-act="view-cover" data-kind="track" data-id="${trackId}" aria-label="View cover" data-tip="View cover">${icon('expand', null, 'ic-16')}</button>`;
  };
  ui.placeMark = function (key) { return raw(placePlaying(key) ? PLACE_MARK : ''); };
  /* "Playing from <name>": the place the song plays from, as a link that opens it at the song (App.goPlaying); radio
     and the listener's own queue open the queue. Empty while nothing plays from anywhere. */
  ui.playingFrom = function (cls, label) {
    var c = App.player.playingFrom(); if (!c) return '';
    var path = App.placePath(c), name = App.placeName(c);
    var inner = html`${icon(App.placeIcon(c), null, 'ic-16')}<span class="from__text"><span class="from__label">${label || 'Playing from'}</span> <b>${name}</b></span>`;
    return path ? html`<a class="from ${cls}" href="#${path}" data-act="go-playing" data-tip="Show the song in ${name}">${inner}</a>`
      : html`<button class="from ${cls}" data-act="go-playing" data-tip="Show the queue">${inner}</button>`;
  };
  App.syncPlaces = function (root) {
    var key = App.placeKey(App.player.playingFrom()), playing = App.player.state.playing;
    $$('[data-place]', root).forEach(function (el) {
      var on = !!key && el.getAttribute('data-place') === key, now = on && playing;
      if (el.classList.contains('is-playing-place') === on && el.classList.contains('is-playing-now') === now) return;
      el.classList.toggle('is-playing-place', on); el.classList.toggle('is-playing-now', now);
      var mark = $('.place-mark', el), at = $('[data-place-at]', el);
      if (on && !mark) { if (at) at.insertAdjacentHTML('afterbegin', PLACE_MARK); else el.insertAdjacentHTML('beforeend', PLACE_MARK); }
      else if (!on && mark) mark.remove();
    });
  };

  /* a row on an album page shows the page's cover when the song's own album (and so its cover) isn't known */
  function rowCover(t, album) { var own = M.album(t.albumId); return album && t.albumId !== album && !(own && own.art) ? album : t; }
  /* bitrate and sample rate, the format column of a wide list ("1040 kbps, 44.1 kHz"); "–" until the file is known,
     as the duration column has it */
  function rateLine(t) {
    var f = M.formats[t.format] || {}, kbps = t.kbps || f.kbps, khz = kHz(t.khz || f.khz);
    return [kbps ? kbps + ' kbps' : '', khz ? khz + ' kHz' : ''].filter(Boolean).join(', ') || '–';
  }
  /* how many peers offer a track: a search result's count, or the length of a fetched source list */
  function sourceCount(t) { return typeof t.sources === 'number' ? t.sources : t.sources ? t.sources.length : 0; }
  /* list wrapper: data-ids gives play-track its context (the rest of the list becomes the queue) */
  ui.trackList = function (tracks, opts) {
    opts = opts || {};
    tracks = tracks.map(function (t) { return typeof t === 'string' ? M.track(t) : t; }).filter(Boolean);
    return html`<ol class="tlist${opts.dense ? ' tlist--dense' : ''}${opts.dated ? ' tlist--dated' : ''}${opts.plays ? ' tlist--plays' : ''}" data-ids="${tracks.map(function (t) { return t.id; }).join(',')}" ${opts.label ? raw('aria-label="' + esc(opts.label) + '"') : ''}>${tracks.map(function (t, i) {
      return ui.trackRow(t, Object.assign({}, opts, { index: opts.numbered === false ? t.no : i + 1, added: opts.added ? opts.added[i] : null, plays: opts.plays ? opts.plays[i] : null, lazy: ui.lazyRow(i, tracks.length) }));
    })}</ol>`;
  };

  /* release tile (shelves): art + flat round play + title + artist + new chip + pixel-face "album, Oct 29" */
  ui.releaseTile = function (album, opts) {
    opts = opts || {}; if (typeof album === 'string') album = M.album(album);
    var key = 'album:' + album.id;
    return html`<div class="tile${ui.placeCls(key)}" data-album-id="${album.id}" data-place="${key}">
      <div class="tile__art"><a href="#/album/${album.id}" aria-label="${album.title} by ${album.artist}">${ui.cover(album, '100%')}</a>
        <button class="tile__play" data-act="play-album" data-id="${album.id}" aria-label="Play ${album.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/album/${album.id}" data-place-at>${ui.placeMark(key)}${album.title}</a></div>
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
    return html`<div class="wave-times"><b data-bind="pos">0:00</b><span class="wave-times__buf">buffered to <span data-bind="buffered-to">0:00</span></span><span data-bind="dur">0:00</span></div>`;
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
    else if (st === 'waiting') { label = 'Waiting for a good copy'; ic = 'clock'; }
    else { label = opts.label || 'Save'; ic = 'save'; }
    var act = st === 'library' ? 'go-album' : st === 'saved' ? 'noop-saved' : st === 'waiting' ? 'save-waiting' : 'save';
    return html`<button class="btn btn-save ${opts.cls || ''}" data-state="${st}" data-save-btn="${trackId}" data-act="${act}" data-id="${trackId}"${raw(attrs)}>${icon(ic)}<span>${label}</span></button>`;
  };
  /* the copy playing (or saved) doesn't pass the listener's minimum (ADR-0008) */
  ui.belowBadge = function (opts) {
    return html`<span class="badge badge-below" ${fitAttr(opts)} data-tip="Below your minimum in Settings > Sources">${icon('warning')}below minimum</span>`;
  };
  /* 5-step pipeline. step = current step (1-5); status saving|failed|saved|waiting */
  ui.pipeline = function (step, status, error) {
    var segs = [];
    for (var i = 1; i <= 5; i++) {
      var c = status === 'saved' || i < step ? 'is-done' : i === step ? (status === 'failed' ? 'is-failed' : 'is-current') : '';
      segs.push(html`<span class="pipeline__seg ${c}" aria-hidden="true">${c === 'is-done' ? icon('check') : c === 'is-failed' ? icon('close') : ''}</span>`);
    }
    var text = status === 'saved' ? 'Saved to your library' : status === 'failed' ? (error || 'Step failed: ' + M.saveSteps[step - 1])
      : status === 'waiting' ? (error || 'No copy meets your saving minimum yet. Looking again by itself.') : M.saveSteps[step - 1];
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
    /* a loading placeholder is an indeterminate progress bar: its name needs that role (a labelled div has none) */
    return raw('<div class="skel-group" role="progressbar" aria-busy="true" aria-label="Loading">' + out.join('') + '</div>');
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
      var by = card.getAttribute('aria-labelledby'), lab = by && (scope.querySelector('[id="' + by + '"]') || document.getElementById(by));
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
    /* unless its content has already focused a control of its own (a form filled in as it arrives: the pairing gate) */
    setTimeout(function () { var a = document.activeElement; if (!layer.closed && (a === el || !el.contains(a))) first.focus(); }, 30);
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
  /* ---------- the cover viewer: a song's, album's or playlist's art, large, without leaving the page ----------
     App.viewCover({ kind: track|album|playlist, id }) opens the design's dialog: the picture with its name, artist and
     album as links. Esc, the scrim and Close shut it, and focus goes back to what opened it (openLayer). The picture
     shows at once with the size this page already decoded, and the fitter swaps in the size the dialog needs once that
     has decoded (covers at the size they are drawn), so it never fades in twice.
     Opened from the player bar's and the Now column's cover (an expand button on hover and focus), the player's "…",
     and every song, album and playlist menu (App.coverItem), so also by right-click. */
  App.viewCover = function (what) {
    var v = coverSubject(what); if (!v) return;
    App.openDialog({ title: 'cover', className: 'cover-view', body: html`<div class="cover-view__art">${v.art}</div>
      <div class="cover-view__info">${v.info}</div>` });
  };
  App.coverItem = function (kind, id) { return { label: 'View cover', icon: 'expand', run: function () { App.viewCover({ kind: kind, id: id }); } }; };
  function coverSubject(w) {
    function link(href, text, cls) { return html`<a class="${cls}" href="${href}" data-close>${text}</a>`; }
    if (w.kind === 'playlist') {
      var p = M.playlist(w.id); if (!p) return null;
      return { art: ui.playlistArt(p, '100%', 'Cover of ' + p.name, { hero: true }),
        info: html`${link('#/playlist/' + encodeURIComponent(p.id), p.name, 'cover-view__title')}<span class="cover-view__by">${p.items.length === 1 ? '1 song' : p.items.length + ' songs'}</span>` };
    }
    var t = w.kind === 'track' ? M.track(w.id) : null, a = t ? M.album(t.albumId) : M.album(w.id), e = t || a;
    if (!e) return null;
    var albumHref = '#/album/' + encodeURIComponent(t ? t.albumId : a.id), albumTitle = t ? t.album : a.title;
    return { art: ui.cover(e, '100%', { label: 'Cover of ' + albumTitle, hero: true }),
      info: html`${link(albumHref, t ? t.title : a.title, 'cover-view__title')}<span class="cover-view__by">${link('#/artist/' + encodeURIComponent(e.artistId), e.artist, 'link')}${t && t.album ? html`, ${link(albumHref, t.album, 'link-plain')}` : ''}</span>` };
  }
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

  /* popover anchored to an element: App.popover(anchor, markup, { label, className, onMount(el, close), role, at })
     -> { el, close, place } (place: position it again after its content changed size). `at` { x, y } opens it at a
     point instead (a context menu at the pointer); the anchor still gets aria-expanded and focus back */
  var openPop = null, contextAt = null;
  function placeNear(el, anchor, at) {
    var r = at ? { left: at.x, right: at.x, top: at.y, bottom: at.y, width: 0 } : anchor.getBoundingClientRect(), w = el.offsetWidth, h = el.offsetHeight;
    var left = clamp(at ? at.x : r.left + r.width / 2 - w / 2, 12, window.innerWidth - w - 12);
    var top = r.bottom + (at ? 2 : 10);
    if (top + h > window.innerHeight - 12) top = Math.max(12, r.top - h - (at ? 2 : 14));
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
    var at = o.at || contextAt;
    placeNear(el, anchor, at);
    anchor.setAttribute('aria-expanded', 'true');
    openPop = { el: el, anchor: anchor, onClose: o.onClose };
    afterPaint(function () { el.classList.add('is-open'); });
    var close = function () { App.closePopover(true); };
    if (o.onMount) o.onMount(el, close);
    var f = focusables(el)[0]; if (f) f.focus();
    return { el: el, close: close, place: function () { placeNear(el, anchor, at); } };
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
    setText(tipEl, text); tipFor = target;
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
    importFile: function (file) { App.toast('Only playlists (.m3u, .m3u8, Exportify .csv) can be dropped here, not ' + file.name, { kind: 'error' }); },
    moreSheet: function () { defaultMoreSheet(); },
    /* media keys / lock screen target: null = App.player; Classic mode sets { play, pause, prev, next, seek, seekBy } */
    media: null
  };
  var SAVE_STATUS = { running: 'saving', done: 'saved', failed: 'failed', pending: 'pending', waiting: 'waiting' };
  App.saves = {
    get: function (id) { return state.saves[id]; },
    /* opts: { mode: 'separate'|'replace' (the duplicate dialog's answer: a saved song is saved again), source: { trackId: { peer, path, size } } } */
    start: function (ids, opts) {
      opts = opts || {};
      ids = [].concat(ids).filter(function (id) { return (opts.mode || !App.isSaved(id)) && !(state.saves[id] && state.saves[id].status === 'saving'); });
      if (!ids.length) { App.toast('Already in your library', { icon: 'saved' }); return; }
      if (opts.mode) { startSaves(ids, opts); return; }           // the duplicate question is already answered
      App.hooks.beforeSave(ids, function (ok, o) { startSaves(ok || ids, Object.assign({}, opts, o)); });
    },
    /* restarts a failed save from its failed step; source ({ peer, path, size }) answers a transfer failure */
    retry: function (id, source) {
      var s = state.saves[id];
      if (!s || !s.jobId) { var src = {}; if (source) src[id] = source; App.saves.start([id], { source: src }); return; }
      setSave(id, Object.assign({}, s, { status: 'saving', error: null }));
      M.api('/api/saves/' + encodeURIComponent(s.jobId) + '/retry', { method: 'POST', body: source ? { source: source } : undefined })
        .catch(function (err) { saveFailed([id], err); });
    },
    /* DELETE /api/saves/{jobId}: stops a waiting (or failed) save; a `cancelled` event follows. 409 while it transfers or
       writes: the hub's sentence says so */
    cancel: function (id) {
      var s = state.saves[id]; if (!s || !s.jobId) { setSave(id, null); return Promise.resolve(); }
      return M.api('/api/saves/' + encodeURIComponent(s.jobId), { method: 'DELETE' }).then(function () {
        setSave(id, null); var t = M.track(id);
        App.toast('Stopped saving ' + (t ? t.title : 'the song'), { icon: 'close' });
      }, function (err) { App.toast('Couldn\'t stop that save: ' + err.message, { kind: 'error' }); });
    },
    set: function (id, rec) { setSave(id, rec); }
  };
  function startSaves(ids, opts) {
    if (!ids.length) return;
    ids.forEach(function (id) { setSave(id, { step: 1, status: 'saving', pct: 0 }); });
    if (ids.length > 1) App.toast('Saving ' + ids.length + ' songs to your library', { icon: 'save' });
    /* the playing song is saved from the peer it streams from, so the transfer is not started twice */
    var source = Object.assign({}, opts.source), cur = P.currentId();
    if (ps.source && ids.indexOf(cur) >= 0 && !source[cur]) source[cur] = { peer: ps.source.peer, path: ps.source.path, size: ps.source.sizeBytes };
    var body = { ids: ids };
    if (Object.keys(source).length) body.source = source;
    if (opts.mode) { body.duplicate = {}; ids.forEach(function (id) { body.duplicate[id] = opts.mode; }); }
    M.api('/api/saves', { method: 'POST', body: body }).then(function (res) {
      var skipped = res.skipped || [], started = ids.filter(function (id) { return skipped.indexOf(id) < 0; });
      skipped.forEach(function (id) { setSave(id, null); M.patchTrack(id, { saved: true }); });   // already in the library
      (res.jobs || []).forEach(function (jobId, i) {
        var id = started[i]; if (id && state.saves[id]) setSave(id, Object.assign({}, state.saves[id], { jobId: jobId }));
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
  /* `save` event: { jobId, trackId, step, status: running|done|failed|pending|waiting|cancelled, pct, error, kind, nocover,
     upgrade, belowMinimum }. waiting: no copy passes the saving minimum yet (`below: wait`), and the job searches again
     by itself; cancelled: the job is gone. belowMinimum: the copy saved (or saving) doesn't pass the saving minimum */
  function onSaveEvent(e) {
    if (e.status === 'cancelled') { setSave(e.trackId, null); return; }
    var was = state.saves[e.trackId], status = SAVE_STATUS[e.status] || 'saving';
    setSave(e.trackId, { jobId: e.jobId, step: e.step, status: status, pct: e.pct || 0, error: e.error || null, kind: e.kind || null, nocover: !!e.nocover,
      upgrade: !!e.upgrade, belowMinimum: !!e.belowMinimum });
    if (status !== 'saved' || (was && was.status === 'saved')) return;
    M.patchTrack(e.trackId, { saved: true });
    var t = M.track(e.trackId);
    App.toast('Saved to library: ' + (t ? t.title : 'one song'), { kind: 'ok', icon: 'saved', action: t ? 'Show' : null,
      onAction: function () { App.go('/album/' + t.albumId); } });
    App.announce('Saved ' + (t ? t.title : 'the song') + ' to your library');
  }

  /* =====================================================================
     8. Player engine: the queue, radio, plays and streaming states. Sound comes from an output (8b: two
        <audio> decks; in the Android app the native player). Saved songs come from /api/library/stream/{id},
        downloaded ones from this device (App.downloads); Soulseek songs from /api/stream/{id}, which serves
        bytes while the transfer runs (`stream` events report finding, queued, stalled, fallback).
     status: idle | finding | choose | queued | buffering | streaming | stalled | failed | library | ended | jellyfin-error
     `playing` is separate: paused = playing false while streaming or library.
     Fractions (0-1) of the track: received (bytes transferred), buffered ranges [[from, to], ...].
     seekWait: a seek past what arrived, kept until it arrives ({ fraction }), or null.
     ===================================================================== */
  var P = App.player = {};
  var saved0 = storage('player') || {};
  var ps = P.state = {
    queue: [], index: 0, playing: false, pos: 0, status: 'idle',
    received: 0, buffered: [], durationKnown: false, seekWait: null,
    peer: null, peersFound: 0, queuePos: 0, fallback: null, failedPeers: [], source: null,
    crossfade: false, shuffle: !!saved0.shuffle, repeat: saved0.repeat || 'off',
    volume: saved0.volume == null ? .7 : saved0.volume, muted: !!saved0.muted, radio: null, context: null, error: null
  };
  var quSeq = 0, duration = 0;
  P.current = function () { var q = ps.queue[ps.index]; return q ? M.track(q.trackId) : null; };
  P.currentId = function () { var q = ps.queue[ps.index]; return q ? q.trackId : null; };
  P.duration = function () { var t = P.current(); return duration || (t ? t.duration : 0); };
  P.upNext = function (n) { return ps.queue.slice(ps.index + 1, ps.index + 1 + (n || 2)).map(function (q) { return M.track(q.trackId); }).filter(Boolean); };
  function changed(kind) { App.emit('player', kind || 'state'); persistSoon(); }

  /* playback settings (Settings > Playback; GET /api/settings), defaults until the server answers */
  var playback = { gapless: true, crossfade: 0, normalisation: 'off', loudnessTarget: -14, radioWhenQueueEnds: true, volumeStep: 5, prefetchCount: 2,
    switchToBetter: true, jumpWhenDownloaded: true };
  P.volumeStep = function () { return (playback.volumeStep || 5) / 100; };
  App.setPlayback = function (patch) {
    Object.assign(playback, patch);
    if (!playback.jumpWhenDownloaded && ps.seekWait) { ps.seekWait = null; changed('seek'); }   // the song plays on where it is
    App.store.set('crossfadeSec', playback.crossfade || 0);
    applyLevels(); prefetchSoon();
  };

  /* ---------- what the output plays: the playing queue entry (cur) and the one set to follow it (nxt) ----------
     Entries carry their stream facts: url, soulseek, complete (the transfer finished), attempt (fallbacks seen). */
  var cur = null, nxt = null, loadSeq = 0, sound = { playing: false, waiting: false };
  /* sound comes from an output (docs/context/web.md#output-interface): <audio> here, the native player in the app */
  var out = window.LullNative && window.LullNative.player ? nativeOutput(window.LullNative.player) : audioOutput();
  function streamUrl(t, source, forceSoulseek) {
    if (App.isSaved(t.id) && !forceSoulseek) return M.url('/api/library/stream/' + encodeURIComponent(t.id));
    var q = source ? '?peer=' + encodeURIComponent(source.peer) + '&path=' + encodeURIComponent(source.path) : '';
    return M.url('/api/stream/' + encodeURIComponent(t.id) + q);
  }
  /* ReplayGain in dB for Settings > Playback's mode, when the library supplies it (track.replayGain { track, album }) */
  function replayGainDb(trackId) {
    var t = M.track(trackId);
    return LullRules.replayGainDb(t && t.replayGain, playback.normalisation);
  }
  /* the output's item for an entry (docs/context/web.md#output-interface). A downloaded song plays from this
     device (App.downloads), otherwise from the hub; resolves to null when the track is unknown. */
  function itemFor(e, forceSoulseek) {
    var t = M.track(e.trackId); if (!t) return Promise.resolve(null);
    var local = !forceSoulseek && App.downloads ? App.downloads.localUrl(t.id) : Promise.resolve(null);
    return local.then(function (localUrl) {
      var url = localUrl || streamUrl(t, e.source, forceSoulseek);
      e.url = url; e.local = !!localUrl; e.soulseek = !localUrl && url.indexOf('api/stream/') >= 0;
      if (!e.soulseek) e.complete = true;
      return item(e, t, localUrl || url);
    });
  }
  function item(e, t, url) {
    return { key: e.uid, url: new URL(url, location.href).href, trackId: t.id, title: t.title, artist: t.artist, album: t.album,
      duration: t.duration || null, artwork: t.albumId ? new URL(App.artUrl(t.albumId, 512), location.href).href : null, gainDb: replayGainDb(t.id) };
  }
  /* a Soulseek transfer nobody will listen to any more is cancelled (DELETE /api/stream/{id}) */
  function letGo(e) {
    if (!e || e === cur || e === nxt || !e.soulseek || e.complete) return;
    if ((cur && cur.trackId === e.trackId) || (nxt && nxt.trackId === e.trackId)) return;
    M.api('/api/stream/' + encodeURIComponent(e.trackId), { method: 'DELETE' }).catch(function () {});
  }
  function playRefused(err) {
    if (err && err.name === 'NotAllowedError') { ps.playing = false; changed('play'); }   // autoplay blocked until a tap
  }

  /* ---------- load, play, pause ---------- */
  function resetTrackState() {
    ps.pos = 0; duration = 0; ps.fallback = null; ps.failedPeers = []; ps.error = null; ps.crossfade = false;
    ps.peer = null; ps.peersFound = 0; ps.queuePos = 0; ps.durationKnown = false; lastRanges = [];
    ps.belowMinimum = false; ps.choices = null; ps.upgrade = null; ps.seekWait = null;
    sound = { playing: false, waiting: false };
  }
  /* begin playing queue[index]: opts { autoplay (default true), at (seconds), reason (how the last song ended), forceSoulseek } */
  function load(index, opts) {
    opts = opts || {};
    if (!opts.keepPlay) endPlay(opts.reason || 'skipped');          // a new source for the same song is the same play
    ps.index = clamp(index, 0, Math.max(0, ps.queue.length - 1));
    resetTrackState();
    var was = [cur, nxt], e = ps.queue[ps.index], seq = ++loadSeq;
    cur = e || null; nxt = null;
    was.forEach(letGo);
    if (!e || !M.track(e.trackId)) { out.pause(); out.setNext(null); ps.status = 'idle'; ps.playing = false; ps.source = null; changed('track'); return; }
    e.attempt = null; e.job = null; e.better = null; if (opts.forceSoulseek) e.complete = false;
    ps.source = e.source || null;
    ps.playing = opts.autoplay !== false;
    var library = App.isSaved(e.trackId) && !opts.forceSoulseek;
    ps.status = library ? 'library' : 'finding'; ps.received = library ? 1 : 0; ps.buffered = []; ps.durationKnown = library;
    if (opts.at) ps.pos = opts.at;
    changed('track');
    itemFor(e, opts.forceSoulseek).then(function (it) {
      if (seq !== loadSeq || !it) return;
      if (!e.soulseek && ps.status === 'finding') { ps.status = 'library'; ps.received = 1; ps.durationKnown = true; changed('status'); }
      out.load(it, { at: opts.at || 0, autoplay: ps.playing }).catch(playRefused);
      applyLevels();
      if (out.eagerNext) preloadNext();
    });
    topUpRadio();
  }
  /* after a peer fallback cut the response: the same entry again, at the same time */
  function reopen() {
    var e = cur, at = ps.pos;
    nxt = null;                                           // load drops the next one; preloading sets it again
    itemFor(e).then(function (it) { if (it && cur === e) out.load(it, { at: at, autoplay: ps.playing }).catch(playRefused); });
  }
  P.play = function () {
    if (!ps.queue.length) { App.toast('Nothing queued. Search or pick something from Home.', { icon: 'info' }); return; }
    if (ps.status === 'ended') { load(0); return; }
    if (ps.status === 'failed' || ps.status === 'jellyfin-error') { P.retry(); return; }
    /* restored after a reload: resume where it was. A Soulseek song may no longer be on the hub at that point (the
       cache evicted it, or only its start arrived before): if the resumed stream fails before it plays, it starts
       again from 0:00 (onOutputError) */
    if (!cur) { ps.queue[ps.index].resumeAt = ps.pos; load(ps.index, { at: ps.pos }); return; }
    ps.playing = true; out.play().catch(playRefused); changed('play');
  };
  P.pause = function () { ps.playing = false; out.pause(); changed('play'); };
  P.toggle = function () { if (ps.playing) P.pause(); else P.play(); };

  /* Shuffle is an order, not a pick: turning it on shuffles the songs after the listener's "next up" in the queue
     itself, so the queue shows what will play, and every song plays once per pass. Each entry keeps `orig`, its
     place before shuffling, so turning shuffle off puts the rest back in order. Next up is never shuffled. */
  function shuffleFrom() { var at = ps.index + 1; while (ps.queue[at] && ps.queue[at].group === 'next') at++; return at; }
  function shuffleRest() {
    var from = shuffleFrom(), rest = ps.queue.slice(from);
    for (var i = rest.length - 1; i > 0; i--) { var j = Math.floor(Math.random() * (i + 1)), x = rest[i]; rest[i] = rest[j]; rest[j] = x; }
    ps.queue.splice.apply(ps.queue, [from, rest.length].concat(rest));
  }
  function unshuffleRest() {
    var from = shuffleFrom(), rest = ps.queue.slice(from);
    rest = rest.map(function (e, k) { return { e: e, k: k }; })            // songs added later keep their place after
      .sort(function (a, b) { return (a.e.orig == null ? Infinity : a.e.orig) - (b.e.orig == null ? Infinity : b.e.orig) || a.k - b.k; })
      .map(function (x) { return x.e; });
    ps.queue.splice.apply(ps.queue, [from, rest.length].concat(rest));
  }
  function markOrder() { ps.queue.forEach(function (e, i) { e.orig = i; }); }
  /* the index the queue moves to next: repeat one (only when a song ends by itself), the next entry, wrap, or -1 */
  function nextIndex(auto) { return LullRules.nextIndex({ index: ps.index, length: ps.queue.length, repeat: ps.repeat }, auto); }
  P.next = function (auto) {
    if (!ps.queue.length) return;
    var i = nextIndex(auto);
    /* repeat one replays the same response, unless a better copy waits for the next play: that is now */
    if (i === ps.index && auto && !(ps.upgrade && ps.upgrade.state === 'next-play')) { endPlay('completed'); out.seek(0); out.play().catch(playRefused); return; }
    if (i < 0) {                                        // the end: radio may still add songs
      topUpRadio().then(function (added) {
        if (!added) { endQueue(auto); return; }
        load(ps.index + 1, { reason: auto ? 'completed' : 'skipped' });
      });
      return;
    }
    load(i, { reason: auto ? 'completed' : 'skipped' });
  };
  /* a Soulseek song no peer delivers while the music was playing gives way to the next one after a beat, as an
     unavailable song does on Spotify; the toast says which and why. Paused, or at the end of the queue, it waits. */
  function skipFailed(e) {
    setTimeout(function () {
      if (cur !== e || ps.status !== 'failed') return;
      var i = nextIndex(false); if (i < 0 || i === ps.index) return;
      var t = M.track(e.trackId);
      App.toast('Skipped ' + (t ? t.title : 'a song') + ': ' + (ps.error || 'no peer shares it right now'), { icon: 'soulseek' });
      P.next();
    }, LullRules.SKIP_FAILED_MS);
  }
  function endQueue(auto) {
    endPlay(auto ? 'completed' : 'skipped');
    ps.status = 'ended'; ps.playing = false; ps.pos = P.duration(); out.pause();
    changed('ended');
  }
  P.prev = function () {
    var i = LullRules.previous(ps.index, ps.pos);
    if (i < 0) { P.seek(0); return; }
    load(i);
  };

  /* ---------- seeking: inside what has arrived, or kept until it arrives ---------- */
  /* how far a seek may go now (0-1): everything for saved songs, what the transfer delivered for Soulseek ones */
  function seekLimit() { return LullRules.seekLimit(!!(cur && cur.soulseek), ps.buffered, ps.received); }
  P.seekable = function (f) { return P.current() ? f <= seekLimit() : false; };
  /* past what arrived: with "Jump to where I seek once it has downloaded" (Settings > Playback) the song plays on and
     the seek is kept until its bytes are here (landKept); without, it snaps to the furthest point that arrived, the
     seek bar shakes and this returns false. The next seek, or another song, replaces a kept one */
  P.seekFraction = function (f) {
    f = clamp(f, 0, 1);
    var dur = P.duration(); if (!dur || !P.current()) return false;
    /* a song restored after a reload (or the app starting) isn't loaded until it plays, and P.play then resumes it at
       ps.pos: a seek before that moves where it will start */
    if (!cur) { ps.pos = f * dur; changed('seek'); return true; }
    var to = LullRules.seekTo(f, seekLimit(), playback.jumpWhenDownloaded);
    ps.seekWait = to.how === 'waits' ? { fraction: to.fraction } : null;
    if (!ps.seekWait) { ps.pos = to.fraction * dur; out.seek(ps.pos); }
    changed('seek');
    if (to.how === 'waits') App.emit('seek-kept', ps.seekWait);
    if (to.how === 'snapped') App.emit('seek-denied', { fraction: f, received: ps.received });
    return to.how !== 'snapped';
  };
  /* a kept seek lands once what arrived reaches it */
  function landKept() {
    var w = ps.seekWait; if (!w || !cur) return;
    if (LullRules.seekTo(w.fraction, seekLimit(), true).how !== 'now') return;
    ps.seekWait = null; ps.pos = w.fraction * P.duration(); out.seek(ps.pos);
    changed('seek');
    App.announce('Playing from ' + fmtTime(ps.pos));
  }
  /* about how long a kept seek (or one at f) waits, in words: "about 8 s", or null while the speed isn't known */
  function waitWords(f) {
    var p = ps.peer, n = p && LullRules.seekWaitS(f, ps.received, (p.sizeMB || 0) * 1e6, (p.speed || 0) * 1e6);
    return n == null ? null : n < 100 ? 'about ' + Math.max(1, n) + ' s' : 'about ' + Math.round(n / 60) + ' min';
  }
  P.seekWaitText = function () {
    var w = ps.seekWait; if (!w) return null;
    var eta = waitWords(w.fraction);
    return 'Waiting for ' + fmtTime(w.fraction * P.duration()) + (eta ? ', ' + eta : '');
  };
  /* the seek bar's hover tip at f */
  P.seekTip = function (f) {
    var at = fmtTime(f * P.duration());
    if (P.seekable(f)) return at;
    if (!playback.jumpWhenDownloaded) return 'Not downloaded yet (' + Math.round(ps.received * 100) + '%)';
    var eta = waitWords(f);
    return at + (eta ? ', downloaded in ' + eta : ', not downloaded yet');
  };
  /* a seek past what has arrived is kept until it has, rather than stopping where the download is */
  P.seeksAhead = function () { return !!playback.jumpWhenDownloaded; };
  /* where the listener asked to be: a kept seek's time, else where the song is. Seeking by steps goes on from there */
  P.seekTarget = function () { return ps.seekWait ? ps.seekWait.fraction * P.duration() : ps.pos; };
  P.seek = function (sec) { return P.seekFraction(sec / (P.duration() || 1)); };
  P.seekBy = function (d) { return P.seek(P.seekTarget() + d); };

  P.setShuffle = function (on) {
    on = on == null ? !ps.shuffle : !!on;
    if (on !== ps.shuffle) {
      ps.shuffle = on;
      if (on) { markOrder(); shuffleRest(); } else unshuffleRest();
      queueChanged();
    }
    changed('mode');
  };
  P.cycleRepeat = function () { ps.repeat = LullRules.cycleRepeat(ps.repeat); releaseNext(); applyLevels(); changed('mode'); };
  P.setVolume = function (v) { ps.volume = clamp(v, 0, 1); ps.muted = false; applyLevels(); changed('volume'); };
  P.toggleMute = function () { ps.muted = !ps.muted; applyLevels(); changed('volume'); };

  /* ---------- queue ---------- */
  function entry(id, group, source) {
    quSeq += 1;
    return { uid: 'q' + quSeq, trackId: id, group: group || 'context', resolve: App.isSaved(id) ? 'ready' : 'resolved', source: source || null };
  }
  /* the song set as next stays loaded when it is still next (a reorder elsewhere in the queue, an append) */
  function queueChanged() {
    if (nxt && ps.queue[nextIndex(true)] !== nxt) releaseNext();
    changed('queue');
  }
  /* replace the playing context and start: playList(ids, startIndex, { radio, context, sources: { trackId: source }, at,
     autoplay }). `context` is the place it plays from ({ kind, id, name }, see "where the music plays from"); left out,
     it is the page showing (App.here()), which is where a click on a list or its Play button came from.
     Songs the listener queued themselves ("next up": Play next, Add to queue) that haven't played yet are theirs, not
     the context's: they stay, right after the new song, as Spotify keeps its "next in queue". */
  P.playList = function (ids, start, opts) {
    opts = opts || {};
    var group = opts.radio ? 'radio' : 'context', sources = opts.sources || {};
    ps.context = opts.context !== undefined ? opts.context : App.here();
    var nextUp = ps.queue.slice(ps.index + 1).filter(function (e) { return e.group === 'next'; });
    endPlay('skipped'); releaseNext();
    start = clamp(start || 0, 0, Math.max(0, ids.length - 1));
    ps.queue = ids.map(function (id) { return entry(id, group, sources[id]); });
    markOrder();
    ps.queue.splice.apply(ps.queue, [start + 1, 0].concat(nextUp));
    ps.radio = opts.radio || null;
    ps.index = start;
    if (ps.shuffle) shuffleRest();                       // the chosen song first, the rest shuffled after next up
    load(start, { reason: 'skipped', at: opts.at, autoplay: opts.autoplay });
    queueChanged();
  };
  /* move to queue[index] without touching the queue: jumpTo(index, { at, autoplay }) */
  P.jumpTo = function (index, opts) { load(index, Object.assign({ reason: 'skipped' }, opts)); };
  P.streamUrl = function (trackId, source) { var t = M.track(trackId); return t ? streamUrl(t, source) : null; };
  /* The stream event a song's opening sends is lost when the player opens the stream before /api/events is
     connected (the page reloading, the app starting with a restored song) or while the events are down. The
     player then knows only what its own buffer holds and refuses to seek past it, which shows on a paused song.
     On every (re)connect it opens the playing Soulseek song's stream once more for one byte, and the hub answers
     with where the song stands (docs/context/api.md, stream). */
  P.resyncStream = function () {
    var t = cur && cur.soulseek && !cur.complete ? M.track(cur.trackId) : null; if (!t) return;   // loaded songs only
    fetch(streamUrl(t, cur.source), { headers: { Range: 'bytes=0-0' }, credentials: 'include' })
      .then(function (r) { if (r.body) r.body.cancel(); }, function () { /* offline: the next connect asks again */ });
  };
  P.playTrack = function (id, contextIds, opts) {
    var ids = contextIds && contextIds.indexOf(id) >= 0 ? contextIds : [id];
    P.playList(ids, ids.indexOf(id), opts);
  };
  function titleOf(id) { var t = M.track(id); return t ? t.title : 'one song'; }
  /* the listener's own picks go to "next up", right after the playing song: Play next at its front, Add to queue at
     its end. They play before the context continues and outlive a new context (playList). */
  function nextUpEnd() { var at = ps.index + 1; while (ps.queue[at] && ps.queue[at].group === 'next') at++; return at; }
  P.playNext = function (ids) {
    ids = [].concat(ids); var at = ps.queue.length ? ps.index + 1 : 0;
    ps.queue.splice.apply(ps.queue, [at, 0].concat(ids.map(function (id) { return entry(id, 'next'); })));
    if (ps.status === 'idle') load(0);
    queueChanged();
    App.toast(ids.length > 1 ? ids.length + ' songs will play next' : 'Playing next: ' + titleOf(ids[0]), { icon: 'queue' });
  };
  P.addToQueue = function (ids) {
    ids = [].concat(ids);
    var at = ps.queue.length ? nextUpEnd() : 0;
    ps.queue.splice.apply(ps.queue, [at, 0].concat(ids.map(function (id) { return entry(id, 'next'); })));
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
    ps.queue.splice(to, 0, item); ps.index = ps.queue.indexOf(playingEntry);
    /* the queue lists its groups (next up, then, radio) in that order, each in one run: a song moved past a group's
       edge joins the group it now sits in, so the list shows the order it plays in */
    var before = to - 1 > ps.index ? ps.queue[to - 1] : null, after = ps.queue[to + 1];
    if (to > ps.index && (before || after)) item.group = (before || after).group;
    queueChanged();
  };
  P.clear = function () {
    var before = ps.queue.slice(), beforeIndex = ps.index, playingEntry = ps.queue[ps.index];
    ps.queue = playingEntry ? [playingEntry] : []; ps.index = 0; queueChanged();
    App.toast('Queue cleared', { icon: 'trash', action: 'Undo', onAction: function () { ps.queue = before; ps.index = beforeIndex; queueChanged(); } });
  };
  /* manual source choice (source picker): useSource(trackId, { peer, path, format, sizeBytes, speed }). The playing
     song switches mid-song (ADR-0008): the chosen peer's stream reopens at the same time, as the same play. */
  P.useSource = function (trackId, source) {
    var i = ps.queue.findIndex(function (q) { return q.trackId === trackId; });
    var current = i >= 0 && i === ps.index && !!cur && ps.status !== 'ended';
    if (i < 0) { i = ps.queue.length ? ps.index + 1 : 0; ps.queue.splice(i, 0, entry(trackId, 'next', source)); }
    else ps.queue[i].source = source;
    load(i, current ? { forceSoulseek: true, at: ps.pos, autoplay: ps.playing, keepPlay: true } : { forceSoulseek: true });
    queueChanged();
  };
  P.retry = function () { load(ps.index, { at: ps.status === 'failed' ? 0 : ps.pos }); };
  /* a saved song Jellyfin can't serve: stream it from Soulseek instead */
  P.trySoulseek = function () { if (P.current()) load(ps.index, { forceSoulseek: true }); };

  /* ---------- where the music plays from: ps.context = { kind, id, name } | null ----------
     Every play started from a place records it beside the queue (and persists with it): the page's own place (a
     route's `place(ctx)`, App.here()), or the album, playlist or radio a tile or button plays. P.playingFrom() is the
     place of the song playing now; the player bar, Now Playing, the mini player and the queue say it, App.goPlaying()
     opens it at the song, and every tile or link of that place (data-place="kind:id") shows the playing bars.
     PLACES: each kind's icon, its page (radio and the queue have none: they open the queue) and its live name (a
     renamed playlist reads its new name; the stored one is the fallback). */
  function enc(s) { return encodeURIComponent(s); }
  var PLACES = {
    playlist: { icon: 'playlists', path: function (id) { return '/playlist/' + enc(id); }, name: function (id) { var p = M.playlist(id); return p && p.name; } },
    album: { icon: 'library', path: function (id) { return '/album/' + enc(id); }, name: function (id) { var a = M.album(id); return a && a.title; } },
    artist: { icon: 'follow', path: function (id) { return '/artist/' + enc(id); }, name: function (id) { var a = M.artist(id); return a && a.name; } },
    tag: { icon: 'tag', path: function (id) { return '/tag/' + enc(id); } },
    outside: { icon: 'globe', path: function (id) { return '/outside/' + enc(id); } },
    liked: { icon: 'heart-fill', path: function () { return '/liked'; } },
    history: { icon: 'clock', path: function () { return '/history'; } },
    songs: { icon: 'saved', path: function () { return '/library/songs'; } },
    downloads: { icon: 'downloaded', path: function () { return '/downloads'; } },
    stats: { icon: 'stats', path: function () { return '/stats'; } },
    home: { icon: 'home', path: function () { return '/home'; } },
    search: { icon: 'search', path: function (q) { return '/search?q=' + enc(q); } },
    radio: { icon: 'radio' },
    queue: { icon: 'queue' }
  };
  /* a stored or reported place, checked once: an unknown kind or a missing name is no place */
  function validPlace(c) {
    return c && PLACES[c.kind] && typeof c.name === 'string' && c.name ? { kind: c.kind, id: String(c.id || ''), name: c.name } : null;
  }
  App.place = function (kind, id, name) { return validPlace({ kind: kind, id: id, name: name }); };
  App.placeKey = function (c) { return c ? c.kind + ':' + c.id : ''; };
  App.placeIcon = function (c) { return PLACES[c.kind].icon; };
  App.placeName = function (c) { var d = PLACES[c.kind]; return (d.name && d.name(c.id)) || c.name; };
  App.placePath = function (c) { var d = PLACES[c.kind]; return d.path ? d.path(c.id) : null; };
  /* the page showing as a place to play from (its route's place(ctx)), or null */
  App.here = function () {
    var cur = App.current, r = cur.route;
    if (!r || !r.place) return null;
    try { return validPlace(r.place(cur)); } catch (e) { console.error(e); return null; }
  };
  function radioPlace(seed, name) { return validPlace({ kind: 'radio', id: seed.kind + ':' + seed.id, name: name + ' radio' }); }
  /* the place of the song playing now: the context it was started from, the radio that added it, or the listener's
     own queue (their "next up", or a song played on its own) */
  P.playingFrom = function () {
    var e = ps.queue[ps.index]; if (!e) return null;
    if (e.group === 'next') return PLACE_QUEUE;
    if (e.group === 'radio' && ps.radio && !(ps.context && ps.context.kind === 'radio')) return radioPlace(ps.radio.seed, ps.radio.name);
    return ps.context || PLACE_QUEUE;
  };
  var PLACE_QUEUE = { kind: 'queue', id: '', name: 'your queue' };

  /* ---------- radio: GET /api/radio tops the queue up while "keeps adding" is on ---------- */
  function radioName(seed) {
    var e = seed.kind === 'artist' ? M.artist(seed.id) : seed.kind === 'track' ? M.track(seed.id) : seed.kind === 'album' ? M.album(seed.id) : null;
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
  /* one radio start at a time: a repeat click gets the same promise; a sticky toast says it is starting; an answer that
     takes longer than RADIO_WAIT_MS counts as failed (and is ignored if it comes later); a failure offers Retry */
  var RADIO_WAIT_MS = 25000, radioStart = null;
  P.startRadio = function (from) {
    var seed = radioSeed(from);
    var key = seed.kind + ':' + seed.id;
    if (radioStart && radioStart.key === key) return radioStart.promise;
    if (radioStart) radioStart.cancel();
    var name = radioName(from), live = true, timer = null;     // an album's radio is named after the album, not its first song
    var note = App.toast('Starting ' + name + ' radio', { icon: 'radio', sticky: true });
    function settle() { live = false; clearTimeout(timer); note.close(); radioStart = null; }
    function failed(message) {
      App.toast('Couldn\'t start ' + name + ' radio: ' + message, { kind: 'error', action: 'Retry', onAction: function () { P.startRadio(from); } });
      return false;
    }
    var promise = new Promise(function (resolve) {
      timer = setTimeout(function () { if (live) { settle(); resolve(failed('it didn\'t answer in time')); } }, RADIO_WAIT_MS);
      fetchRadio(seed, .4, []).then(function (ids) {
        if (!live) return;
        settle();
        if (seed.kind === 'track') ids = [seed.id].concat(ids.filter(function (id) { return id !== seed.id; }));
        if (!ids.length) { App.toast('No radio for ' + name + ' yet: nothing similar was found', { icon: 'radio' }); resolve(false); return; }
        P.playList(ids, 0, { radio: { name: name, seed: seed, keepsAdding: true, steer: .4 }, context: radioPlace(from, name) });
        App.toast('Radio: ' + name + ', keeps adding songs', { icon: 'radio' });
        resolve(true);
      }, function (err) { if (live) { settle(); resolve(failed(err.message)); } });
    });
    radioStart = { key: key, promise: promise, cancel: function () { settle(); } };
    return promise;
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
    var plan = LullRules.radio({ index: ps.index, length: ps.queue.length, repeat: ps.repeat, radio: ps.radio }, playback.radioWhenQueueEnds);
    if (plan.seed) {
      var last = ps.queue[ps.index];
      ps.radio = { name: radioName({ kind: 'track', id: last.trackId }), seed: { kind: 'track', id: last.trackId }, keepsAdding: true, steer: LullRules.RADIO_STEER };
    }
    if (!plan.ask) return Promise.resolve(0);
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

  /* radio paused by an unreachable hub (or an error) tries again once the hub answers */
  App.on('server', function (srv) {
    if (!srv.reachable || !ps.radio || !ps.radio.error || topping) return;
    ps.radio = Object.assign({}, ps.radio, { error: null }); changed('queue');
    topUpRadio();
  });

  /* ---------- server `stream` events: the Soulseek side of the playing song ---------- */
  function onStreamEvent(ev) {
    var e = cur && cur.trackId === ev.trackId ? cur : nxt && nxt.trackId === ev.trackId ? nxt : null;
    if (!e || !e.soulseek) return;
    if (ev.size && ev.received >= ev.size) e.complete = true;
    /* a fallback to another peer ends the old response: reopen the stream and seek back to where we were.
       The first event of a fetch only says which attempt the response already belongs to (the hub sends one on
       every opening, also for a song that fell back long ago; and the better copy switched to counts its own), so
       it is adopted, not reopened */
    if (e.job !== ev.jobId) { e.job = ev.jobId; e.attempt = ev.attempt || 0; }
    else if (ev.attempt > e.attempt) { e.attempt = ev.attempt; if (e === cur) reopen(); }
    if (e === nxt) { e.resolve = ev.status === 'failed' ? 'none' : ev.status === 'streaming' ? 'ready' : 'resolving'; changed('queue'); return; }
    onUpgrade(e, ev.upgrade);
    var prevPeer = ps.peer && ps.peer.name, t = P.current();
    ps.received = ev.size ? clamp(ev.received / ev.size, 0, 1) : ps.received;
    ps.peersFound = ev.peersFound || ps.peersFound;
    ps.queuePos = ev.queuePos || 0;
    ps.failedPeers = (ev.failedPeers || []).map(function (f) { return typeof f === 'string' ? { peer: f, at: null } : f; });
    if (ev.peer) {
      ps.peer = { name: ev.peer, rank: ps.failedPeers.length + 1, of: ps.peersFound || null, speed: ev.speed ? ev.speed / 1e6 : (ps.peer && ps.peer.speed) || 0,
        format: ev.format || (ps.source && ps.source.format) || t.format, sizeMB: ev.size ? ev.size / 1e6 : t.sizeMB };
    }
    ps.belowMinimum = !!ev.belowMinimum;
    var heard = sound.playing && !sound.waiting;
    switch (ev.status) {
      /* streaming `below: ask`: no copy passes the minimum; the listener picks one of `choices` (or skips) */
      case 'choose':
        ps.status = 'choose'; ps.choices = ev.choices || [];
        if (!e.asked) { e.asked = true; App.hooks.sourcePicker(e.trackId, { choose: true }); }
        break;
      case 'finding': case 'queued': case 'stalled':
        if (!heard || ev.status === 'stalled') ps.status = ev.status; break;
      case 'buffering': if (!heard) ps.status = 'buffering'; break;
      case 'streaming': ps.status = heard || !ps.playing ? 'streaming' : 'buffering'; ps.fallback = null; break;
      case 'fallback':
        ps.fallback = { from: prevPeer, to: ev.peer, n: ps.failedPeers.length + 1, of: ps.peersFound || null };
        ps.status = 'buffering';
        if (ev.peer && ev.peer !== prevPeer) App.toast('Now from ' + ev.peer, { icon: 'soulseek', kind: 'ok' });
        break;
      case 'failed':
        if (ps.playing) skipFailed(cur);
        ps.status = 'failed'; ps.playing = false; out.pause(); ps.seekWait = null;
        ps.error = ev.error || 'Couldn\'t play: no peer responded';
        cur.resolve = 'none';
        break;
    }
    if (ps.received >= 1 && duration) ps.durationKnown = true;
    updateBuffered();
    landKept();
    changed(ev.peer !== prevPeer ? 'source' : 'status');
  }

  /* ---------- a better copy mid-song (docs/context/web.md#better-copies) ----------
     While the song plays a lesser copy (YouTube's, one below the minimum, a peer fallen back to), the hub fetches a
     better one beside it and offers it in `stream` events (`upgrade`). Once what arrived covers where the song is,
     the output plays it silently on its second deck, lines it up by ear (align.js) and crossfades; right before the
     fade the hub makes it the song's stream (POST /api/stream/{id}/switch). When the copies can't be lined up, or the
     output can't do it, nothing changes mid-song and the better copy plays from the next play: the hub picks the best
     cached copy. A copy the listener picked (`e.source`) is never switched away from.
     ps.upgrade: { peer, path, format, pct, state: arriving | ready | switching | switched | next-play, note; once switched:
     from (the copy before), at (the song's time), residualMs (how far apart the copies were left), offsetS (where the copy's own time was) } */
  var switching = false;
  function onUpgrade(e, up) {
    if (ps.upgrade && /^(switching|switched)$/.test(ps.upgrade.state)) return;
    if (!up) { ps.upgrade = null; return; }
    var key = up.peer + '\n' + up.path;
    if (!e.better || e.better.key !== key) e.better = { key: key, tries: 0, note: null };
    ps.upgrade = { peer: up.peer, path: up.path, format: up.format, pct: up.size ? clamp(up.received / up.size, 0, 1) : 0,
      state: e.better.note ? 'next-play' : up.ready ? 'ready' : 'arriving', note: e.better.note };
    trySwitch();
  }
  function waitForNextPlay(e, note) {
    e.better.note = note;
    if (cur === e && ps.upgrade) { ps.upgrade.state = 'next-play'; ps.upgrade.note = note; changed('source'); }
  }
  /* switch when the song plays steadily, far enough from its end, from a copy nobody picked, with the better copy
     here up to a little past where the song is; anything else waits for a later tick */
  function trySwitch() {
    var up = ps.upgrade, e = cur, dur = P.duration();
    if (!up || up.state !== 'ready' || switching || !e || !e.soulseek) return;
    var now = LullRules.switchNow({ picked: !!e.source, canSwitch: !!out.switchTo, duration: dur, pos: ps.pos, arrived: up.pct,
      steady: ps.playing && sound.playing && !sound.waiting && !ps.crossfade });
    if (now === 'next-play') {
      waitForNextPlay(e, out.switchTo ? 'The song is nearly over, so the better copy plays from the next play.'
        : 'This player can\'t line two copies up, so the better copy plays from the next play.');
    }
    if (now !== 'switch') return;
    var t = M.track(e.trackId); if (!t) return;
    var id = encodeURIComponent(t.id), from = playingFormat(t) || (ps.peer && ps.peer.format), at = ps.pos;
    var url = M.url('/api/stream/' + id + '/copy?peer=' + encodeURIComponent(up.peer) + '&path=' + encodeURIComponent(up.path));
    switching = true; up.state = 'switching'; releaseNext(); changed('source');
    out.switchTo(item(e, t, url), { commit: function () {
      return M.api('/api/stream/' + id + '/switch', { method: 'POST', body: { peer: up.peer, path: up.path } });
    } }).then(null, function () { return { switched: false, why: 'error' }; })
      .then(function (r) {
        switching = false;
        if (cur !== e) return;
        if (r.switched) {
          e.url = url; e.complete = up.pct >= 1;
          ps.upgrade = { peer: up.peer, path: up.path, format: up.format, pct: up.pct, state: 'switched', from: from, at: at, residualMs: r.residualMs, offsetS: r.offsetS };
          App.toast('Switched to a better copy' + (up.format ? ': ' + up.format : ''), { icon: 'soulseek', kind: 'ok' });
        } else if (r.why === 'mismatch') {
          waitForNextPlay(e, 'The two copies couldn\'t be lined up closely enough to switch without a jump, so the better copy plays from the next play.');
        } else if (r.why === 'refused' || ++e.better.tries >= LullRules.SWITCH_TRIES) {
          waitForNextPlay(e, 'Switching didn\'t work out this time, so the better copy plays from the next play.');
        } else up.state = 'ready';                       // interrupted (a seek, a pause, a quiet passage): later
        changed('source');
        preloadNext();
      });
  }

  /* buffered = what the output holds (seconds), cut to what the transfer delivered; both as 0-1 of the song */
  var lastRanges = [];
  function updateBuffered() {
    var dur = P.duration(), outR = [];
    if (dur) lastRanges.forEach(function (r) {
      var a = r[0] / dur, b = Math.min(r[1] / dur, cur && cur.soulseek ? ps.received || 1 : 1);
      if (b > a) outR.push([clamp(a, 0, 1), clamp(b, 0, 1)]);
    });
    ps.buffered = outR;
    if (cur && cur.soulseek && !ps.received && outR.length) ps.received = outR[outR.length - 1][1];   // until the first event
  }
  /* the output failed: ask the hub why (the body says it in a sentence), then show it */
  function onOutputError(e, msg) {
    if (e.resumeAt && e === cur) { e.resumeAt = 0; ps.pos = 0; load(ps.index, { at: 0 }); return; }   // see P.play
    var t = P.current();
    /* a Soulseek stream's own `stream` events carry the hub's sentence (a failed event sets ps.error); asking the stream
       URL again would start another transfer and answer about that one instead (409) */
    var why = e.local ? Promise.resolve(msg || 'The downloaded file can\'t be played')
      : e.soulseek ? Promise.resolve(ps.error || msg || 'No peer delivered this song')
      : fetch(e.url, { headers: { Range: 'bytes=0-0' }, credentials: 'same-origin' }).then(function (res) {
        if (res.ok) return msg || 'This file can\'t be played here';
        return res.json().then(function (b) { return b.message; }, function () { return 'The server answered ' + res.status; });
      }, function () { return 'Can\'t reach the lull server'; });
    why.then(function (text) {
      if (e !== cur || P.current() !== t) return;
      ps.playing = false; ps.error = text;
      ps.status = e.soulseek ? 'failed' : 'jellyfin-error';
      changed('status');
    });
  }

  /* ---------- gapless and crossfade: the output continues into the entry set as next ---------- */
  function releaseNext() { if (!nxt) return; var e = nxt; nxt = null; out.setNext(null); letGo(e); }
  /* set the next song once the playing one is close to its end (or its transfer is done). The Android app's
     output wants it at once (eagerNext): its page may be asleep when the song ends. */
  function preloadNext() {
    if (switching) return;                                // the second deck holds the better copy meanwhile
    var i = nextIndex(true);
    if (i < 0 || i === ps.index || !playback.gapless && !playback.crossfade) { if (nxt) releaseNext(); return; }
    /* a Soulseek song still transferring gets the bandwidth to itself until its last half minute */
    var e = ps.queue[i], lead = LullRules.preloadLead(playback.crossfade, cur.soulseek, ps.received);
    if (!out.eagerNext && !LullRules.preloadNow(lead, P.duration(), ps.pos)) return;
    if (nxt === e) return;
    var was = nxt; nxt = e; letGo(was);
    if (e.resolve === 'resolved') { e.resolve = 'resolving'; changed('queue'); }
    itemFor(e).then(function (it) { if (it && nxt === e) out.setNext(it); });
  }
  /* the output moved on by itself into `next` (gapless or the end of a crossfade) */
  function handOver(e) {
    endPlay('completed');
    var was = cur; cur = e; nxt = null;
    ps.index = ps.queue.indexOf(e); resetTrackState();
    ps.status = e.soulseek ? (e.complete ? 'streaming' : 'buffering') : 'library';
    ps.received = e.soulseek && !e.complete ? 0 : 1; ps.durationKnown = !e.soulseek; ps.source = e.source || null;
    letGo(was); applyLevels(); changed('track'); topUpRadio();
    if (out.eagerNext) preloadNext();
  }
  function onEnded(next) {
    if (next) { if (nxt && nxt.uid === next) handOver(nxt); return; }
    /* a Soulseek response that ends early is a fallback in progress, not the song's end: the `stream` event
       with the next attempt reopens it */
    if (LullRules.endedEarly(cur.soulseek, cur.complete, P.duration(), ps.pos)) { ps.status = 'buffering'; changed('status'); return; }
    P.next(true);
  }

  /* ---------- output events (docs/context/web.md#output-interface) ---------- */
  function isCur(key) { return cur && cur.uid === key; }
  function onTime(d) {
    if (!isCur(d.key)) return;
    var was = ps.status;
    if (d.duration > 0 && d.duration !== duration) { duration = d.duration; ps.durationKnown = !cur.soulseek || ps.received >= 1; changed('status'); }
    trackListening(d.position, d.playing);
    ps.pos = d.position;
    sound = { playing: d.playing, waiting: d.waiting };
    if (d.playing && !d.waiting && (ps.status === 'finding' || ps.status === 'queued' || ps.status === 'buffering' || ps.status === 'stalled' || ps.status === 'library' || ps.status === 'streaming')) {
      ps.status = cur.soulseek ? 'streaming' : 'library'; ps.error = null;
      beginPlay(P.current());
    } else if (d.waiting && ps.playing && (ps.status === 'streaming' || ps.status === 'library')) ps.status = 'buffering';
    if (ps.status !== was) changed('status');
  }
  out.on('time', onTime);
  out.on('buffered', function (d) { if (isCur(d.key)) { lastRanges = d.ranges; updateBuffered(); } });
  out.on('ended', function (d) { if (isCur(d.key)) onEnded(d.next); });
  out.on('error', function (d) {
    if (nxt && nxt.uid === d.key) { nxt.resolve = 'none'; changed('queue'); return; }
    if (isCur(d.key)) onOutputError(cur, d.message);
  });
  out.on('crossfade', function (d) { if (isCur(d.key)) { ps.crossfade = !!d.next; changed('status'); } });
  out.on('remote', function (d) { if (d.action === 'next') P.next(); else if (d.action === 'prev') P.prev(); });

  /* the equaliser: this device's (web/eq.js), followed live, also when Classic's Webamp or another window changes it.
     An Android app older than its native equaliser has none: Settings says so (P.canEqualize) */
  function applyEq(s) { if (out.equalize) out.equalize(LullEq.spec(s)).catch(function (err) { console.warn('The equaliser didn\'t apply', err); }); }
  P.canEqualize = function () { return !!out.equalize; };
  applyEq(LullEq.read()); LullEq.on(applyEq);

  /* volume (perceptual curve, mute) and crossfade length go to the output; repeat one never crossfades */
  function applyLevels() {
    out.setVolume(LullRules.outputVolume(ps.volume, ps.muted));
    out.crossfade(LullRules.crossfadeS(ps.repeat, playback.crossfade));
  }

  /* =====================================================================
     8b. The browser's output: two <audio> decks, the playing one and the next, preloaded for gapless and
         crossfade. Loudness normalisation and crossfade go through Web Audio gains when switched on.
         The same contract as window.LullNative.player (docs/context/web.md#output-interface), plus the
         optional `crossfade { key, next }` event (next null: the fade was abandoned) and `switchTo` (a better copy of
         the playing song, lined up and crossfaded; below).
     ===================================================================== */
  function audioOutput() {
    var subs = {}, decks = [deck(), deck()], a = decks[0], b = decks[1], volume = 1, xfade = 0, fading = null;
    function emit(type, detail) { (subs[type] || []).forEach(function (fn) { fn(detail); }); }
    function deck() {
      /* offset: how far this copy's own time runs ahead of the song's (a better copy switched to); lag: what its delay
         holds it back by. Both 0 until a switch */
      var d = { el: new Audio(), item: null, waiting: false, fade: 1, norm: 1, gain: null, analyser: null, ms: 0, msN: 0,
        offset: 0, lag: 0, delay: null, sw: null };
      d.el.preload = 'auto';
      ['playing', 'waiting', 'pause', 'timeupdate', 'durationchange', 'ended', 'error'].forEach(function (type) {
        d.el.addEventListener(type, function () { onEl(d, type); });
      });
      return d;
    }
    /* waiting: no sound yet for want of data, from the start until 'playing' */
    function hold(d, it) {
      d.item = it; d.waiting = true; d.fade = 1; d.norm = 1; d.ms = 0; d.msN = 0;
      if (it) { d.el.src = it.url; d.el.load(); } else { d.el.pause(); d.el.removeAttribute('src'); d.el.load(); }
      setLag(d, 0); d.offset = 0;
      if (d.sw) { d.sw.gain.cancelScheduledValues(0); d.sw.gain.value = 1; }
    }
    function key(d) { return d.item ? d.item.key : null; }
    function dur(d) { return isFinite(d.el.duration) && d.el.duration > 0 ? d.el.duration : d.item && d.item.duration || 0; }
    /* the song's time = the deck's own, less the copy's offset and its delay */
    function shift(d) { return d.offset + d.lag; }
    function time(d) {
      emit('time', { key: key(d), position: Math.max(0, (d.el.currentTime || 0) - shift(d)),
        duration: isFinite(d.el.duration) && d.el.duration > 0 ? d.el.duration - d.offset : null, playing: !d.el.paused, waiting: d.waiting });
    }
    function ranges(d) { var r = [], br = d.el.buffered, k = shift(d); for (var i = 0; i < br.length; i++) r.push([Math.max(0, br.start(i) - k), br.end(i) - k]); return r; }
    function start(d) {
      ensureGraph();
      if (d.el.readyState < 3) d.waiting = true;
      var p = d.el.play();
      return p && p.catch ? p : Promise.resolve();
    }
    function onEl(d, type) {
      if (!d.item) return;
      if (type === 'waiting') d.waiting = true;
      else if (type === 'playing') d.waiting = false;
      if (switching && d === b) { if (type === 'error') abandonSwitch(); return; }   // the better copy, still silent
      if (type === 'error') { emit('error', { key: key(d), message: null, status: null }); return; }
      if (d !== a) return;                                   // the preloading deck reports only errors
      if (type === 'ended') { abandonSwitch(); if (fading) finishFade(); else ended(); return; }
      time(d);
    }
    /* gapless: continue into the next deck only when the song really reached its end; an early end (a
       Soulseek response cut by a fallback) is reported with next null and the next deck stays */
    function ended() {
      var from = key(a), d = dur(a);
      if (b.item && LullRules.gapless(d, a.el.currentTime)) { swap(); start(a).catch(function () {}); emit('ended', { key: from, next: key(a) }); time(a); return; }
      emit('ended', { key: from, next: null });
    }
    function swap() { var s = a; a = b; b = s; hold(b, null); a.fade = 1; levels(); }

    /* crossfade: the next deck starts under an equal-power fade over the playing song's last seconds */
    function maybeFade() {
      var d = dur(a);
      if (!xfade || switching || fading || a.el.paused || !b.item || !d || d - a.el.currentTime > xfade || b.el.readyState < 3) return;
      fading = { start: Date.now(), len: LullRules.crossfadeLengthS(d - a.el.currentTime) * 1000 };
      b.fade = 0; start(b).catch(function () {}); levels();
      emit('crossfade', { key: key(a), next: key(b) });
    }
    function stepFade() {
      if (!fading) return;
      var k = clamp((Date.now() - fading.start) / fading.len, 0, 1);
      var f = LullRules.fade(k); a.fade = f[0]; b.fade = f[1];
      levels();
      if (k >= 1) finishFade();
    }
    function finishFade() { if (!fading) return; fading = null; var from = key(a); swap(); emit('ended', { key: from, next: key(a) }); time(a); }
    function abandonFade() {
      if (!fading) return;
      fading = null; b.fade = 1; b.el.pause(); b.el.currentTime = 0; a.fade = 1; levels();
      emit('crossfade', { key: key(a), next: null });
    }

    /* levels: volume x crossfade x loudness normalisation; then the equaliser (web/eq.js), after the decks are mixed, so
       crossfades, gapless and a better copy's handover pass through one unchanged chain */
    var ctx = null, master = null, eqSpec = null, eqChain = null;
    function wantGraph() { return playback.normalisation && playback.normalisation !== 'off' || playback.crossfade > 0 || playback.switchToBetter || !!eqSpec; }
    /* Web Audio only when a feature needs it; built inside a user gesture (play), once */
    function ensureGraph() {
      if (ctx || !wantGraph() || !window.AudioContext) { if (ctx && ctx.state === 'suspended') ctx.resume(); return; }
      ctx = new AudioContext();
      master = ctx.createGain(); eqChain = LullEq.chain(ctx); eqChain.set(eqSpec);
      master.connect(eqChain.input); eqChain.output.connect(ctx.destination);
      /* each deck: source -> delay (a better copy's, to line it up) -> switch fade -> level -> master; the analyser
         listens to the source */
      decks.forEach(function (d) {
        var src = ctx.createMediaElementSource(d.el);
        d.delay = ctx.createDelay(MAX_DELAY_S); d.sw = ctx.createGain();
        d.gain = ctx.createGain(); d.analyser = ctx.createAnalyser(); d.analyser.fftSize = 2048;
        src.connect(d.delay); d.delay.connect(d.sw); d.sw.connect(d.gain); d.gain.connect(master); src.connect(d.analyser);
      });
      levels();
    }
    /* turned on mid-song: the graph is built at once (the page has had the listener's click, so it starts running);
       otherwise the next play builds it */
    function equalize(spec) {
      eqSpec = spec;
      if (eqChain) eqChain.set(spec);
      else if (spec && !a.el.paused && (!navigator.userActivation || navigator.userActivation.hasBeenActive)) ensureGraph();
      return Promise.resolve();
    }
    function levels() {
      decks.forEach(function (d) {
        var level = d.fade * (playback.normalisation === 'off' ? 1 : d.norm);
        if (d.gain) { d.gain.gain.setTargetAtTime(level, ctx.currentTime, .05); d.el.volume = 1; }
        else d.el.volume = clamp(volume * d.fade, 0, 1);
      });
      if (master) master.gain.setTargetAtTime(volume, ctx.currentTime, .05);
    }
    /* ReplayGain when the library supplies it, otherwise measured: mean loudness of what has played (unweighted
       RMS, close enough to LUFS for levelling songs against each other), levelled toward Settings > Playback's
       target (-18 / -14 / -11), within -12 to +6 dB */
    function normalise(d) {
      if (!d.gain || playback.normalisation === 'off' || !d.item) return;
      var db = replayGainDb(d.item.trackId);
      if (db != null) { d.norm = LullRules.gain(db); return; }
      if (d.el.paused || playback.normalisation !== 'loudness') return;
      var buf = d.buf || (d.buf = new Float32Array(d.analyser.fftSize));
      d.analyser.getFloatTimeDomainData(buf);
      var sum = 0; for (var i = 0; i < buf.length; i++) sum += buf[i] * buf[i];
      d.ms += sum / buf.length; d.msN += 1;
      var norm = LullRules.loudness(playback.loudnessTarget, Math.sqrt(d.ms / d.msN), d.msN);   // after ~3 s of listening
      if (norm != null) d.norm = norm;
    }
    /* ---------- a better copy of the playing song, switched to without a jump (docs/context/web.md#better-copies) ----------
       It plays silently on the second deck from about where the song is. One ScriptProcessor records both decks in the
       same render quanta, and LullAlign measures how much later the copy sounds: a seek puts it a little ahead, its
       DelayNode holds it back to the sample, and another measurement checks what is left (`residualMs`). At a quiet
       moment (or after QUIET_S) the engine's commit() makes it the song's stream, and an equal-power fade of FADE_S
       hands over, the copy's level matched to the song's and gliding to its own. Copies that don't clearly line up are
       never switched ({ switched: false, why: 'mismatch' }). A load abandons it; a pause, seek or end before the hub
       switched abandons it too, and after that finishes the handover at once. */
    var FADE_S = LullRules.SWITCH_FADE_S, RECORD_S = 2.5, WIDE_S = 1, QUIET_S = LullRules.SWITCH_QUIET_S, AHEAD_S = .15, MAX_DELAY_S = 1.5, TOLERANCE_S = .00025, switching = null;
    function setLag(d, s) { d.lag = s; if (d.delay) d.delay.delayTime.value = s; }
    function stop(why) { var e = new Error(why); e.why = why; return e; }
    function switchTo(it, o) {
      if (!ctx || ctx.state !== 'running' || !a.item || a.el.paused || fading || switching || !window.LullAlign) return Promise.resolve({ switched: false, why: 'busy' });
      var sw = switching = { live: true }, rate = ctx.sampleRate, rec = recorder();
      function alive(x) { if (!sw.live) throw stop('abandoned'); return x; }
      function wait(ms) { return new Promise(function (r) { setTimeout(r, ms); }).then(alive); }
      function measure(wide) {
        return rec.take(RECORD_S).then(function (x) {
          alive();
          var m = LullAlign.measure(x.a, x.b, rate, wide); m.levels = [LullAlign.rms(x.a), LullAlign.rms(x.b)];
          return m;
        });
      }
      /* the copy at `at` of its own time, playing steadily */
      function place(at) {
        b.el.currentTime = Math.max(0, at);
        var until = Date.now() + 20000;
        return new Promise(function (resolve, reject) {
          (function look() {
            if (!sw.live) reject(stop('abandoned'));
            else if (!b.el.paused && !b.el.seeking && b.el.readyState >= 3 && !b.waiting) resolve();
            else if (Date.now() > until) reject(stop('slow'));
            else setTimeout(look, 100);
          })();
        }).then(function () { return wait(300); });
      }
      /* where the song is now in the copy, at the same point of its own time first, then shifted by how much longer it
         is (an intro the other hasn't) */
      function find(shifted) {
        var here = a.el.currentTime - a.lag;
        return place(here + (shifted ? dur(b) - dur(a) : 0)).then(function () { return measure(WIDE_S); }).then(function (m) {
          if (LullAlign.locked(m)) return m;
          if (m.quiet) throw stop('quiet');
          if (!shifted && Math.abs(dur(b) - dur(a)) > .8 * WIDE_S) return find(true);
          throw stop('mismatch');
        });
      }
      /* the copy a little ahead (a seek can't land to the sample), so its delay can hold it back exactly */
      function ahead(m) {
        if (m.lag <= -.002 && m.lag >= -MAX_DELAY_S) return m;
        return place(b.el.currentTime + m.lag + AHEAD_S).then(function () { return measure(.5); }).then(function (m2) {
          if (!LullAlign.similar(m2)) throw stop(m2.quiet ? 'quiet' : 'mismatch');
          if (m2.lag > -.002 || m2.lag < -MAX_DELAY_S) throw stop('mismatch');
          return m2;
        });
      }
      /* held back by what was measured, then measured again: what is left must be under TOLERANCE_S */
      function verify(left) {
        return wait(b.lag * 1000 + 200).then(function () { return measure(.05); }).then(function (m) {
          if (!LullAlign.similar(m)) throw stop(m.quiet ? 'quiet' : 'mismatch');
          if (Math.abs(m.lag) <= TOLERANCE_S) return m;
          var next = b.lag - m.lag;
          if (!left || next < 0 || next > MAX_DELAY_S) throw stop('mismatch');
          setLag(b, next);
          return verify(left - 1);
        });
      }
      hold(b, it); b.norm = a.norm; b.sw.gain.value = 0;
      return start(b).then(function () { return find(false); })
        .then(ahead)
        .then(function (m) { setLag(b, -m.lag); return verify(2); })
        .then(function (m) {
          sw.residual = m.lag;
          sw.match = LullRules.levelMatch(m.levels[0], m.levels[1]);
          b.offset = b.el.currentTime - b.lag - (a.el.currentTime - shift(a));
          return quietMoment(sw);
        })
        .then(function () {
          sw.committing = true;
          return Promise.resolve(o && o.commit ? o.commit() : null).then(null, function () { throw stop('refused'); });
        })
        .then(function () { if (!sw.live) throw stop('abandoned'); return fadeOver(sw); })
        .then(function () { return { switched: true, residualMs: sw.residual * 1000, offsetS: a.offset }; },
          function (err) {
            if (sw.live && !sw.done) { sw.live = false; hold(b, null); }
            return { switched: false, why: err && err.why || 'error' };
          })
        .then(function (r) { rec.close(); if (switching === sw) switching = null; return r; });
    }
    /* a quiet moment in the song (well under its recent level), or QUIET_S from now */
    function quietMoment(sw) {
      var buf = new Float32Array(a.analyser.fftSize), levels = [], until = Date.now() + QUIET_S * 1000;
      return new Promise(function (resolve, reject) {
        (function look() {
          if (!sw.live) { reject(stop('abandoned')); return; }
          a.analyser.getFloatTimeDomainData(buf);
          var level = LullAlign.rms(buf); levels.push(level);
          if (LullRules.quiet(levels) || Date.now() >= until) resolve();
          else setTimeout(look, 50);
        })();
      });
    }
    function fadeOver(sw) {
      if (sw.cut) { finishSwitch(sw); return Promise.resolve(); }
      var t0 = ctx.currentTime + .03, n = 64, down = new Float32Array(n), up = new Float32Array(n);
      for (var i = 0; i < n; i++) { var f = LullRules.fade(i / (n - 1)); down[i] = f[0]; up[i] = sw.match * f[1]; }
      a.sw.gain.setValueCurveAtTime(down, t0, FADE_S);
      b.sw.gain.setValueCurveAtTime(up, t0, FADE_S);
      b.sw.gain.setTargetAtTime(1, t0 + FADE_S + .01, LullRules.SWITCH_GLIDE_S);   // from the song's level to the copy's own
      sw.fading = true;
      return new Promise(function (r) { setTimeout(r, (FADE_S + .08) * 1000); }).then(function () { finishSwitch(sw); });
    }
    /* the better copy becomes the playing deck. Cut short (a pause or seek during the handover): at once, at the song's
       time, paused if the song was */
    function finishSwitch(sw) {
      if (!sw.live || sw.done) return;
      sw.done = true;
      var old = a, pos = old.el.currentTime - shift(old), paused = old.el.paused;
      if (sw.cut) { b.sw.gain.cancelScheduledValues(0); b.sw.gain.value = 1; b.el.currentTime = pos + shift(b); }
      a = b; b = old; hold(b, null);
      if (paused) a.el.pause();
      levels(); time(a);
    }
    /* a load (hard) drops it; a pause, seek or end before the hub switched drops it, after that finishes it at once */
    function abandonSwitch(hard) {
      var sw = switching; if (!sw || sw.done) return;
      if (!hard && sw.committing) { sw.cut = true; if (sw.fading) finishSwitch(sw); return; }
      sw.live = false; switching = null; hold(b, null);
    }
    /* both decks into one ScriptProcessor (a mono channel each): take(seconds) resolves to what they played meanwhile */
    function recorder() {
      var merger = ctx.createChannelMerger(2), proc = ctx.createScriptProcessor(4096, 2, 1), mute = ctx.createGain(), taps = [], got = null;
      mute.gain.value = 0;
      [a, b].forEach(function (d, i) {
        var mono = ctx.createGain(); mono.channelCount = 1; mono.channelCountMode = 'explicit'; mono.channelInterpretation = 'speakers';
        d.delay.connect(mono); mono.connect(merger, 0, i); taps.push([d.delay, mono]);
      });
      merger.connect(proc); proc.connect(mute); mute.connect(ctx.destination);
      proc.onaudioprocess = function (ev) {
        if (!got) return;
        var x = ev.inputBuffer.getChannelData(0), y = ev.inputBuffer.getChannelData(1), n = Math.min(x.length, got.a.length - got.n);
        got.a.set(x.subarray(0, n), got.n); got.b.set(y.subarray(0, n), got.n); got.n += n;
        if (got.n >= got.a.length) { var done = got; got = null; done.resolve(done); }
      };
      return {
        take: function (sec) {
          return new Promise(function (resolve) { var len = Math.round(sec * ctx.sampleRate); got = { a: new Float32Array(len), b: new Float32Array(len), n: 0, resolve: resolve }; });
        },
        close: function () {
          proc.onaudioprocess = null;
          taps.forEach(function (t) { t[0].disconnect(t[1]); });
          merger.disconnect(); proc.disconnect(); mute.disconnect();
        }
      };
    }

    /* the deck clock: time at least 4 times a second while playing, buffered when it moved, normalisation, the fade */
    var held = '';
    setInterval(function () {
      if (!a.item) return;
      if (!a.el.paused) { normalise(a); if (fading) normalise(b); levels(); time(a); }
      var r = ranges(a), k = key(a) + JSON.stringify(r);
      if (k !== held) { held = k; emit('buffered', { key: key(a), ranges: r }); }
      stepFade(); maybeFade();
    }, 250);

    return {
      eagerNext: false,
      on: function (type, fn) {
        (subs[type] = subs[type] || []).push(fn);
        return function () { subs[type] = subs[type].filter(function (f) { return f !== fn; }); };
      },
      load: function (it, o) {
        o = o || {};
        abandonSwitch(true); abandonFade();
        if (b.item && b.item.key === it.key && b.item.url === it.url) { var s = a; a = b; b = s; a.fade = 1; }   // already preloaded
        else hold(a, it);
        hold(b, null);
        if (o.at) a.el.currentTime = o.at;
        levels();
        if (o.autoplay === false) { a.el.pause(); return Promise.resolve(); }
        return start(a);
      },
      play: function () { return start(a); },
      pause: function () { abandonSwitch(); a.el.pause(); abandonFade(); return Promise.resolve(); },
      seek: function (sec) { abandonSwitch(); a.el.currentTime = sec + shift(a); time(a); return Promise.resolve(); },
      setVolume: function (v) { volume = v; levels(); return Promise.resolve(); },
      setNext: function (it) {
        if (it && b.item && b.item.key === it.key && b.item.url === it.url) return Promise.resolve();
        abandonSwitch(); abandonFade(); hold(b, it);
        return Promise.resolve();
      },
      crossfade: function (sec) { xfade = sec; if (!sec) abandonFade(); return Promise.resolve(); },
      equalize: equalize,
      switchTo: switchTo
    };
  }
  /* the Android app's native player: the same contract; it wants the next song as soon as one plays */
  function nativeOutput(n) {
    function call(name) {
      return function () {
        try { return Promise.resolve(n[name].apply(n, arguments)); } catch (e) { return Promise.reject(e); }
      };
    }
    return { eagerNext: true, on: function (type, fn) { return n.on(type, fn); }, load: call('load'), play: call('play'), pause: call('pause'),
      seek: call('seek'), setVolume: call('setVolume'), setNext: call('setNext'), crossfade: call('crossfade'),
      equalize: n.equalize ? call('equalize') : null };
  }

  /* ---------- plays: POST /api/plays when a song starts, PATCH when it ends (scrobbling, history, taste) ---------- */
  /* App.recordPlay(trackId, durationS, recommended) -> { listened (seconds heard, the caller adds to it), end(how) };
     recommended: started from radio or a feed pick (Stats hit rate);
     how: completed (ended by itself) | skipped (the listener moved on) | stopped (page closed).
     Also used by Classic mode, where Webamp plays. */
  var playWarned = false, OUTBOX = {};
  App.recordPlay = function (trackId, dur, recommended) {
    var rec = { listened: 0, ended: false }, startedAt = Math.floor(Date.now() / 1000);
    var body = LullRules.playStart(trackId, dur, recommended);
    /* out of the hub's reach the whole play goes to the outbox when it ends, with its real start time */
    var id = App.outbox.offline() ? Promise.resolve(OUTBOX) : M.api('/api/plays', { method: 'POST', body: body })
      .then(function (r) {
        /* the hub says this device listens privately: nothing was recorded, so there is no play to end */
        if (r && r.private && !state.privateListening) App.store.set('privateListening', true);
        return r && r.playId;
      }, function (err) {
        if (err.status === 0) return OUTBOX;
        if (!playWarned) { playWarned = true; App.toast('Listening history is off: ' + err.message, { kind: 'error' }); }
        return null;
      });
    rec.end = function (how) {
      if (rec.ended) return; rec.ended = true;
      var end = LullRules.playEnd(rec.listened, how);
      id.then(function (playId) {
        if (playId === OUTBOX) App.outbox.add(Object.assign({ op: 'play', trackId: trackId, startedAt: startedAt }, end));
        else if (playId) M.api('/api/plays/' + encodeURIComponent(playId), { method: 'PATCH', keepalive: true, body: end }).catch(function () {});
      });
    };
    return rec;
  };
  var play = null;            // { uid (queue entry), rec, last (element time at the last timeupdate) }
  function beginPlay(t) {
    var e = ps.queue[ps.index];
    if (play && play.uid === e.uid) return;
    endPlay('skipped');
    play = { uid: e.uid, rec: App.recordPlay(t.id, P.duration(), e.group === 'radio'), last: null };
  }
  function trackListening(now, playing) {
    if (!play) return;
    var dt = play.last == null ? 0 : now - play.last;
    play.rec.listened += LullRules.heard(dt, playing);
    play.last = now;
  }
  function endPlay(how) { if (play) { play.rec.end(how); play = null; } }
  window.addEventListener('pagehide', function () { endPlay('stopped'); persist(); });

  /* ---------- the clock: preload, and 'tick' for screens (the output reports the position) ---------- */
  var TICK = LullRules.TICK_MS;
  /* a paused song changes nothing on screen: the tick goes out only when what it shows moved (the waiting states
     count seconds), instead of rewriting the seek bars and times four times a second */
  var tickWas = '';
  setInterval(function () {
    if (!cur) return;
    if (cur.resumeAt && ps.playing && ps.pos > cur.resumeAt + 1) cur.resumeAt = 0;   // the resume worked
    preloadNext(); trySwitch();
    updateBuffered(); landKept();
    if (document.hidden) return;
    var waiting = /^(finding|queued|buffering|stalled)$/.test(ps.status) ? Math.floor(Date.now() / 1000) : 0;
    var sig = [ps.playing, ps.pos.toFixed(2), ps.received.toFixed(3), bufKey(ps.buffered), ps.status, waiting].join('|');
    if (sig === tickWas) return;
    tickWas = sig;
    App.emit('tick', ps);
  }, TICK);

  /* ---------- preloading (Settings > Playback): the hub starts fetching the next few Soulseek songs while this one
     plays, so a skip finds them on disk instead of searching and waiting for a peer. PUT /api/stream/prefetch takes
     the whole list each time; songs that drop off it are cancelled by the hub. Saved songs need nothing. Nothing is
     asked while the playing song is still finding its own source: it gets Soulseek's searches first. ---------- */
  var prefetchSent = '', prefetchTimer = null;
  function prefetchSoon() { clearTimeout(prefetchTimer); prefetchTimer = setTimeout(syncPrefetch, LullRules.PREFETCH_MS); }
  function upcoming(n) {
    if (!cur) return [];
    var q = { index: ps.index, ids: ps.queue.map(function (e) { return e.trackId; }), shuffle: ps.shuffle, repeat: ps.repeat };
    return LullRules.upcoming(q, function (id) { return App.isSaved(id); }, n);
  }
  function syncPrefetch() {
    if (cur && cur.soulseek && /^(finding|queued|buffering|choose)$/.test(ps.status)) return;   // the 'status' change after it re-runs this
    var ids = upcoming(playback.prefetchCount), key = ids.join(',');
    if (key === prefetchSent) return;
    prefetchSent = key;
    M.api('/api/stream/prefetch', { method: 'PUT', body: { tracks: ids } }).catch(function (err) { console.warn('Preloading unavailable:', err.message); });
  }
  App.on('player', function (k) { if (k === 'queue' || k === 'track' || k === 'status' || k === 'mode') prefetchSoon(); });

  /* ---------- the queue survives a reload on this device (localStorage, a convenience) ---------- */
  var persistTimer = null;
  function persistSoon() { clearTimeout(persistTimer); persistTimer = setTimeout(persist, 600); }
  function persist() {
    var keep = {};
    ps.queue.forEach(function (q) { var t = M.track(q.trackId); if (t) keep[t.id] = t; });
    storage('player', { volume: ps.volume, muted: ps.muted, shuffle: ps.shuffle, repeat: ps.repeat, radio: ps.radio, context: ps.context,
      index: ps.index, pos: ps.pos, queue: ps.queue.map(function (q) { return { trackId: q.trackId, group: q.group, source: q.source, orig: q.orig }; }), tracks: keep });
  }
  function restore() {
    if (!saved0.queue || !saved0.queue.length) return;
    M.absorb({ tracks: saved0.tracks });
    ps.queue = saved0.queue.filter(function (q) { return M.track(q.trackId); }).map(function (q) { var e = entry(q.trackId, q.group, q.source); e.orig = q.orig; return e; });   // orig: shuffle's way back
    if (!ps.queue.length) return;
    ps.index = clamp(saved0.index || 0, 0, ps.queue.length - 1);
    ps.radio = saved0.radio ? Object.assign({}, saved0.radio, { error: null }) : null;    // an old error was that session's
    ps.context = validPlace(saved0.context);
    ps.pos = saved0.pos || 0;
    var t = P.current();
    ps.status = App.isSaved(t.id) ? 'library' : 'streaming';
    ps.received = App.isSaved(t.id) ? 1 : 0;
  }
  restore();

  /* how long the song has waited in this state, once that is long enough to wonder (5 s) */
  var statusWas = null, statusSince = 0;
  App.on('player', function () { if (ps.status !== statusWas) { statusWas = ps.status; statusSince = Date.now(); } });
  function waited() { var sec = Math.floor((Date.now() - statusSince) / 1000); return sec >= 5 ? ', ' + sec + ' s' : ''; }
  /* one line describing the current status (player bar state tab, mini-player sub line, aria) */
  P.statusText = function () {
    var peer = ps.peer ? ps.peer.name : 'a peer', of = ps.peer && ps.peer.of ? ' of ' + ps.peer.of : '';
    switch (ps.status) {
      case 'idle': return 'Search or pick something from Home';
      case 'finding': return 'Finding a source' + (ps.peersFound ? ', ' + ps.peersFound + ' peers answered' : '') + waited();
      case 'queued': return 'Queued at peer ' + peer + (ps.queuePos ? ', position ' + ps.queuePos : '') + waited();
      case 'buffering':
        if (cur && !cur.soulseek) return cur.local ? 'Loading from this device' : 'Loading from your library';
        return ps.fallback ? 'Switching to ' + ps.fallback.to + ' (' + ps.fallback.n + (ps.fallback.of ? ' of ' + ps.fallback.of : '') + ')' : 'Buffering from ' + peer;
      case 'stalled': return 'Peer stalled, switching to the next source (' + ((ps.peer ? ps.peer.rank : 1) + 1) + of + ')';
      case 'choose': return 'No copy meets your streaming minimum: pick one or skip';
      case 'failed': return ps.error || 'Couldn\'t play: no peer responded';
      case 'jellyfin-error': return 'Library item unavailable' + (ps.error ? ': ' + ps.error : '');
      case 'ended': return 'End of queue';
      default:
        if (ps.seekWait) return P.seekWaitText();
        if (ps.crossfade) { var n = nxt && M.track(nxt.trackId); return 'Crossfading into ' + (n ? n.title : 'the next song'); }
        return null;
    }
  };

  /* =====================================================================
     9. Live bindings: seek bars, waveforms, data-bind text, rows (playing / liked / saved)
     ===================================================================== */
  function bufKey(r) { return r.map(function (x) { return x[0].toFixed(3) + '-' + x[1].toFixed(3); }).join('|'); }
  /* writes that skip an unchanged value: the clock runs four times a second, and rewriting the same text, attribute or
     custom property still costs a style recalculation (and for text, a layout) of what it touches. Text changes the
     existing text node's data: replacing the node (textContent =) counts as inserting one, which makes the designs'
     page-wide :has() rules restyle the whole page each second */
  function setText(el, v) {
    var n = el.firstChild;
    if (n && n.nodeType === 3 && !n.nextSibling) { if (n.data !== v) n.data = v; }
    else if (el.textContent !== v) el.textContent = v;
  }
  function setAttr(el, name, v) { v = String(v); if (el.getAttribute(name) !== v) el.setAttribute(name, v); }
  function setVar(el, name, v) { v = String(v); if (el.style.getPropertyValue(name) !== v) el.style.setProperty(name, v); }
  function updateSeeks() {
    var dur = P.duration() || 1, f = clamp(ps.pos / dur, 0, 1), bk = bufKey(ps.buffered);
    var bufTo = ps.buffered.length ? ps.buffered[ps.buffered.length - 1][1] * dur : 0;
    var valText = fmtTime(ps.pos) + ' of ' + (ps.durationKnown ? '' : 'about ') + fmtTime(dur) + (ps.received < 1 ? ', downloaded to ' + fmtTime(bufTo) : '')
      + (ps.seekWait ? ', waiting for ' + fmtTime(ps.seekWait.fraction * dur) : '');
    var transferring = ps.received < 1 && (ps.status === 'streaming' || ps.status === 'buffering');
    // player bar + mini: play state and the received percentage (an integer, usable as counter(rx) in CSS)
    ['pbar', 'mini'].forEach(function (s) { var el = document.getElementById(s); if (!el) return; setAttr(el, 'data-playing', !!ps.playing); setVar(el, '--rx', Math.round(ps.received * 100)); });
    $$kept('.seek[data-seek="player"]').forEach(function (el) {
      setVar(el, '--pos', f.toFixed(4));
      setVar(el, '--xfer', ps.received.toFixed(4));
      if (el.classList.contains('is-transferring') !== transferring) el.classList.toggle('is-transferring', transferring);
      if (el._bk !== bk) {
        el._bk = bk;
        var host = $('.seek__bufs', el);
        if (host) host.innerHTML = ps.buffered.map(function (r) { return '<i class="seek__buf" style="left:' + (r[0] * 100).toFixed(2) + '%;width:' + ((r[1] - r[0]) * 100).toFixed(2) + '%"></i>'; }).join('');
      }
      setAttr(el, 'aria-valuenow', Math.round(f * 100));
      setAttr(el, 'aria-valuetext', valText);
    });
    $$kept('.wave[data-seek="player"]').forEach(function (el) {
      var bars = $$('.wave__bar', el), n = bars.length;
      setVar(el, '--pos', f.toFixed(4));
      var moving = ps.playing && ps.status !== 'buffering';
      if (el.classList.contains('is-playing') !== moving) el.classList.toggle('is-playing', moving);
      bars.forEach(function (b, i) {
        var x = (i + .5) / n, inBuf = ps.buffered.some(function (r) { return x >= r[0] && x <= r[1]; });
        var c = x <= f ? 'is-played' : inBuf ? 'is-buf' : x <= ps.received ? 'is-xfer' : '';
        if (b._c !== c) { if (b._c) b.classList.remove(b._c); if (c) b.classList.add(c); b._c = c; }
      });
      setAttr(el, 'aria-valuenow', Math.round(f * 100));
      setAttr(el, 'aria-valuetext', valText);
    });
    $$kept('.seek-row').forEach(function (el) { setVar(el, '--time-ch', fmtTime(dur, !ps.durationKnown).length); });
    $$kept('[data-bind="pos"]').forEach(function (el) { setText(el, fmtTime(ps.pos)); });
    $$kept('[data-bind="dur"]').forEach(function (el) { setText(el, fmtTime(dur, !ps.durationKnown)); });
    $$kept('[data-bind="buffered-to"]').forEach(function (el) { setText(el, fmtTime(bufTo)); });
    $$kept('[data-bind="received"]').forEach(function (el) { setText(el, Math.round(ps.received * 100) + '%'); });
    var vol = ps.muted ? 0 : ps.volume;
    $$kept('.seek[data-seek="volume"]').forEach(function (el) {
      setVar(el, '--pos', vol.toFixed(3)); setVar(el, '--xfer', 0);
      setAttr(el, 'aria-valuenow', Math.round(vol * 100)); setAttr(el, 'aria-valuetext', ps.muted ? 'Muted' : Math.round(vol * 100) + '%');
    });
  }
  App.setText = setText;
  App.on('tick', updateSeeks);
  App.on('player', updateSeeks);

  /* rows and buttons that show per-track state: playing row, like buttons, Saved/Soulseek badges, save buttons */
  App.syncRows = function () {
    var cur = P.currentId();
    $$('.trow[data-track-id]').forEach(function (r) {
      var on = r.getAttribute('data-track-id') === cur;
      if (r.classList.contains('is-playing') !== on) { r.classList.toggle('is-playing', on); playingBars(r, on); }
      r.classList.toggle('is-playing-now', on && ps.playing);
    });
    App.syncPlaces();
    $$('[data-like]').forEach(function (b) {
      var id = b.getAttribute('data-like'), on = App.isLiked(id);
      if (b.getAttribute('aria-pressed') === String(on)) return;
      b.setAttribute('aria-pressed', String(on)); b.classList.toggle('is-liked', on);
      b.setAttribute('data-tip', on ? 'Liked' : 'Like');
      var u = $('use', b); if (u) u.setAttribute('href', '#i-' + (on ? 'heart-fill' : 'heart'));
    });
    $$('[data-src-badge]').forEach(function (el) {
      var want = App.isSaved(el.getAttribute('data-src-badge')) ? 'saved' : 'slsk';
      if (el.getAttribute('data-was') === want) return;
      el.setAttribute('data-was', want);
      patch(el, want === 'saved' ? ui.savedBadge() : ui.soulseekBadge());
    });
    $$('[data-src-mini]').forEach(function (el) {
      var on = App.isSaved(el.getAttribute('data-src-mini'));
      if (el._saved !== on) { el._saved = on; patch(el, window.icon(on ? 'saved' : 'soulseek', on ? 'Saved' : 'Soulseek')); }
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
    /* focused for the arrow keys, but a click draws no focus ring: data-pointer holds it back until a key is used */
    e.preventDefault(); el.setAttribute('data-pointer', ''); el.focus({ preventScroll: true });
    drag = { el: el }; el.classList.add('is-scrubbing');   // not is-dragging: that class boxes a dragged row
    try { el.setPointerCapture(e.pointerId); } catch (err) { /* synthetic events */ }
    applySeek(el, seekFrac(el, e.clientX), false);
  });
  /* the hover tip showing, remembered: this runs on every pointer move anywhere, so it must not search the page */
  var tipOn = null, tipAt = 0;
  document.addEventListener('pointermove', function (e) {
    if (drag) { applySeek(drag.el, seekFrac(drag.el, e.clientX), false); return; }
    var el = e.target.closest && e.target.closest('.seek[data-seek="player"]');
    var tip = el && $('.seek__tip', el);
    if (tipOn && tipOn !== tip) { tipOn.classList.remove('is-on'); tipOn = null; }
    if (!tip) return;
    var f = seekFrac(el, e.clientX);
    setText(tip, P.seekTip(f));
    tip.style.left = (f * 100) + '%'; tip.classList.add('is-on'); tipOn = tip; tipAt = f;
  });
  App.on('tick', function () { if (tipOn) setText(tipOn, P.seekTip(tipAt)); });   // what arrived moves on under a still pointer
  document.addEventListener('pointerup', function (e) {
    if (!drag) return;
    applySeek(drag.el, seekFrac(drag.el, e.clientX), true);
    drag.el.classList.remove('is-scrubbing'); drag = null;
  });
  document.addEventListener('keydown', function (e) {
    var el = e.target.closest && e.target.closest('[data-seek]'); if (!el) return;
    el.removeAttribute('data-pointer');
    var vol = el.getAttribute('data-seek') === 'volume', step = vol ? P.volumeStep() : 5 / (P.duration() || 1);
    var cur = vol ? (ps.muted ? 0 : ps.volume) : P.seekTarget() / (P.duration() || 1), f = null;
    if (e.key === 'ArrowRight' || e.key === 'ArrowUp') f = cur + step * (e.shiftKey ? 6 : 1);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') f = cur - step * (e.shiftKey ? 6 : 1);
    else if (e.key === 'Home') f = 0; else if (e.key === 'End') f = 1;
    else if (e.key === 'PageUp') f = cur + step * 6; else if (e.key === 'PageDown') f = cur - step * 6;
    if (f === null) return;
    e.preventDefault(); e.stopPropagation(); applySeek(el, clamp(f, 0, 1), true);
  });
  document.addEventListener('focusout', function (e) { if (e.target.closest && e.target.closest('[data-seek]')) e.target.removeAttribute('data-pointer'); });
  /* the wheel over a volume bar turns it: one notch (or 100 px of trackpad scroll) is one volume step */
  var wheelAcc = 0;
  document.addEventListener('wheel', function (e) {
    var el = e.target.closest && e.target.closest('[data-seek="volume"]'); if (!el) return;
    e.preventDefault();
    var dy = e.deltaMode === 1 ? e.deltaY * 33 : e.deltaMode === 2 ? e.deltaY * 400 : e.deltaY;
    if ((dy < 0) !== (wheelAcc < 0)) wheelAcc = 0;
    wheelAcc += dy;
    var steps = Math.trunc(wheelAcc / 100); if (!steps) return;
    wheelAcc -= steps * 100;
    P.setVolume(clamp((ps.muted ? 0 : ps.volume) - steps * P.volumeStep(), 0, 1));
  }, { passive: false });
  App.on('seek-denied', function (d) {
    App.emit('announce', 'Not downloaded yet, ' + Math.round(d.received * 100) + ' percent');
  });
  App.on('seek-kept', function () { App.emit('announce', P.seekWaitText() + '. The song plays on until then.'); });

  /* segmented controls: the indicator sits on the selected button, measured (--seg-x, --seg-w on .seg__ind). A .seg's
     columns are equal only while every label fits its share, and a design may add gaps; until the first measure the
     CSS equal split stands in. Placed when a .seg appears, when its selection changes and when a button resizes; it
     slides only when the selection moves. A refresh's patch strips the indicator's inline style, so that re-places it */
  var SEG_PICKED = ['aria-selected', 'aria-checked', 'aria-pressed'];
  var segs = new Set(), segWatched = new WeakSet(), segLast = new WeakMap();
  function segPicked(s) {
    return $$('.seg__btn', s).filter(function (b) { return SEG_PICKED.some(function (a) { return b.getAttribute(a) === 'true'; }); })[0];
  }
  function placeSeg(s) {
    var b = segPicked(s), ind = $('.seg__ind', s), sr = s.getBoundingClientRect();
    if (!b || !ind || !sr.width) return;                     // nothing picked (the CSS hides it), or not rendered
    var k = Math.abs(sr.width - s.offsetWidth) > 1 ? s.offsetWidth / sr.width : 1, br = b.getBoundingClientRect();   // k undoes a scaled ancestor
    var at = { btn: b, x: ((br.left - sr.left) * k - s.clientLeft).toFixed(2) + 'px', w: (br.width * k).toFixed(2) + 'px' };
    var last = segLast.get(s), slide = last && last.btn !== b && last.btn.isConnected;
    segLast.set(s, at);
    if (!slide && ind.style.getPropertyValue('--seg-x') === at.x && ind.style.getPropertyValue('--seg-w') === at.w) return;
    var from = slide ? last : at;                             // a slide starts from where it was, even after a patch
    ind.style.transition = 'none';
    ind.style.setProperty('--seg-x', from.x); ind.style.setProperty('--seg-w', from.w);
    void ind.offsetWidth;
    ind.style.removeProperty('transition');
    if (slide) { ind.style.setProperty('--seg-x', at.x); ind.style.setProperty('--seg-w', at.w); }
  }
  var segSizes = new ResizeObserver(function (entries) {
    var seen = new Set();
    entries.forEach(function (e) { var s = e.target.closest('.seg'); if (s && !seen.has(s)) { seen.add(s); placeSeg(s); } });
    segMarks.takeRecords();                                   // our own writes
  });
  function watchSeg(s) {
    segs.add(s);
    [s].concat($$('.seg__btn', s)).forEach(function (el) { if (!segWatched.has(el)) { segWatched.add(el); segSizes.observe(el); } });
  }
  var segMarks = new MutationObserver(function (records) {
    var touched = new Set();
    records.forEach(function (r) {
      var t = r.target.nodeType === 1 ? r.target : r.target.parentNode, s = t && t.closest && t.closest('.seg');
      if (s) touched.add(s);
      if (r.type !== 'childList') return;
      r.addedNodes.forEach(function (n) {
        if (n.nodeType !== 1) return;
        if (n.matches('.seg')) touched.add(n);
        n.querySelectorAll('.seg').forEach(function (x) { touched.add(x); });
      });
      if (r.removedNodes.length) segs.forEach(function (x) {
        if (x.isConnected) return;
        segs.delete(x); [x].concat($$('.seg__btn', x)).forEach(function (el) { segWatched.delete(el); segSizes.unobserve(el); });
      });
    });
    touched.forEach(function (s) { if (s.isConnected) { watchSeg(s); placeSeg(s); } });
    segMarks.takeRecords();
  });
  segMarks.observe(document.documentElement, { subtree: true, childList: true, attributes: true, attributeFilter: SEG_PICKED.concat('style') });
  $$('.seg').forEach(function (s) { watchSeg(s); placeSeg(s); });

  /* =====================================================================
     10. Shell: rail, header, status tray, context panel, player bar, mini-player, tab bar
     ===================================================================== */
  /* the rail's (and the shelf popover's) two sections; an empty one says what will show there instead of a bare heading */
  function followedTags() {
    var ids = Object.keys(state.followed).filter(function (id) { return state.followed[id]; });
    var tags = ids.filter(function (id) { return !/^ar-/.test(id); }), artists = ids.length - tags.length;
    if (!tags.length) return html`<p class="rail__empty">Tags you follow show up here.${artists ? html` <a href="#/following">${artists === 1 ? 'The artist you follow' : 'The ' + artists + ' artists you follow'}</a> ${artists === 1 ? 'is' : 'are'} on Following.` : ''}</p>`;
    return html`${tags.map(function (id) { return ui.tagChip(id); })}`;
  }
  function playlistLinks() {
    if (!state.playlists.length) return html`<li><p class="rail__empty">Playlists you make show up here. <a href="#/playlists">Make one</a></p></li>`;
    return html`${state.playlists.map(function (p) {
      var key = 'playlist:' + p.id;
      return html`<li><a class="rail__pl${ui.placeCls(key)}" href="#/playlist/${p.id}" data-drop="playlist:${p.id}" data-place="${key}">${ui.playlistArt(p, 36)}<span class="ellipsis"><b>${p.name}</b><small>${p.items.length} songs</small></span>${ui.placeMark(key)}</a></li>`;
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
  var TRAY_WARN = { soulseek: 'reconnecting', library: 'slow', catalog: 'limited', cache: 'full', lastfm: 'queued' };
  function trayWorst() {
    var s = state.services, keys = ['soulseek', 'library', 'catalog', 'cache', 'lastfm'], i;
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
    var full = html`${['soulseek', 'library', 'lastfm'].map(function (k) {
      var extra = k === 'lastfm' && s.lastfm.queued ? html`&nbsp;<span class="tray__count">${s.lastfm.queued}</span><span class="tray__label-long" data-fit="2">&nbsp;queued</span>` : '';
      return html`<span class="tray__pill" data-svc="${k}" data-state="${s[k].state}">${ui.statusGlyph(s[k].state)}${s[k].label}${extra}</span>`;
    })}<span class="tray__pill tray__pill--cache" data-svc="cache" data-state="${c.state}" ${cacheAttrs}>${ui.statusGlyph(c.state)}<span class="tray__label-long" data-fit="2">cache&nbsp;</span>${gb}</span>`;
    var worst = trayWorst();
    var compact = html`<span class="tray__pill" data-worst="${worst.state}">${['soulseek', 'library', 'lastfm'].map(function (k) { return ui.statusGlyph(s[k].state); })}<span class="tray__word">${worst.word}</span>${c.capGB ? html`<span class="tray__gb">${c.usedGB}G</span>` : ''}</span>`;
    $$('[data-tray]').forEach(function (b) {
      b.innerHTML = String(b.getAttribute('data-tray') === 'compact' ? compact : full);
      b.setAttribute('aria-label', trayLabel());
      b.style.setProperty('--cache-frac', frac.toFixed(3));
      b.setAttribute('data-cache-used', c.usedGB || 0); b.setAttribute('data-cache-cap', c.capGB || 0);
    });
    if (App.layout) App.layout.refit();     // the tray's words changed: the header fits again
  }
  function statusPopover(anchor) {
    var s = state.services;
    var actions = { soulseek: ['Settings', 'open-sources'], library: ['Rescan', 'jellyfin-rescan'], catalog: ['Check again', 'sys-health'],
      lastfm: ['Retry now', 'lastfm-retry'], cache: ['Manage', 'open-cache'] };
    var body = html`<div class="popover__head">services</div><div class="status-list">${Object.keys(s).map(function (k) {
      var st = s[k].state, act = k === 'lastfm' && st === 'off' ? ['Set up', 'open-lastfm'] : actions[k];   // nothing to retry while Last.fm isn't connected
      return html`<div class="status-row">${ui.statusGlyph(st, STATE_WORDS[st])}<div><span class="status-row__name">${s[k].label}</span><span class="status-row__detail">${s[k].detail}</span></div>
        <button class="btn btn--sm" data-act="${act[1]}">${act[0]}</button></div>`;
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
    var was = state.services[key] || {};
    if (Object.keys(patch).every(function (k) { return was[k] === patch[k]; })) return;   // the same report again: nothing to redraw
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
  function onJobEvent(e) { upsertJob(Object.assign({}, e, { id: e.jobId })); learnJobTracks([e]); }
  /* the next song can start at once: the player has it loaded, or the hub has bytes of it on disk (a preload) */
  P.isPreloaded = function (id) {
    if (nxt && nxt.trackId === id) return true;
    return state.activity.some(function (j) { return j.kind === 'transfer' && j.trackId === id && (j.state === 'done' || j.state === 'transferring' && j.pct > 0); });
  };
  function onTransferEvent(e) {
    upsertJob({ id: e.jobId, kind: 'transfer', trackId: e.trackId, peer: e.peer, state: e.state, queuePos: e.queuePos,
      pct: e.size ? Math.round(e.received / e.size * 100) : 0, speed: e.speed ? e.speed / 1e6 : 0, error: e.error || null });
    learnJobTracks([e]);
  }
  function loadJobs() {
    return M.api('/api/jobs').then(function (res) {
      var rows = (res.items || []).map(function (j) { return Object.assign({ id: j.jobId }, j); });
      /* unfinished saves survive a reload: their rows seed the save records the buttons read (newest row wins) */
      rows.slice().reverse().forEach(function (j) {
        if (j.kind !== 'save' || !j.trackId || j.stepState === 'done') return;
        setSave(j.trackId, { jobId: j.jobId, step: j.step, status: SAVE_STATUS[j.stepState] || 'saving', pct: j.pct || 0,
          error: j.error || null, kind: j.failKind || null, nocover: !!j.nocover, upgrade: !!j.upgrade, belowMinimum: !!j.belowMinimum });
      });
      App.store.set({ activity: rows, activityError: null });
      learnJobTracks(rows);
    }, function (err) { App.store.set({ activity: [], activityError: err.message }); });
  }
  /* GET /api/jobs lists rows without entities: read the songs they name (once each), then repaint Activity */
  var jobTracksAsked = {};
  function learnJobTracks(rows) {
    var ids = rows.map(function (j) { return j.trackId; }).filter(function (id, i, a) { return id && !M.track(id) && !jobTracksAsked[id] && a.indexOf(id) === i; });
    if (!ids.length) return;
    ids.forEach(function (id) { jobTracksAsked[id] = true; });
    Promise.all(ids.map(function (id) { return M.api('/api/tracks/' + encodeURIComponent(id)).catch(function () { return null; }); }))
      .then(function () { App.store.set('activity', state.activity.slice()); });
  }
  App.loadJobs = loadJobs;
  function updateActivityBadge() {
    var n = activeJobs();
    $$('[data-activity-count]').forEach(function (el) { el.textContent = n; el.hidden = !n; });
    $$('[data-act="open-activity"]').forEach(function (b) { b.setAttribute('aria-label', 'Activity, ' + n + ' running'); });
  }
  App.activeJobs = activeJobs;

  /* context panel: App.panel(id, { label, render(el) -> markup, mount(el) }) registers a tab (queue, lyrics, activity).
     From 1360 px it is docked: closing it collapses it to an icon strip (layout.js, ADR-0007), which stays. Below, it
     opens over the page. From `xwide`, Queue and Lyrics can show as two panes at once (layout panelSplit). */
  var panels = {}, panelOrder = ['queue', 'lyrics', 'activity'], SPLIT = ['queue', 'lyrics'];
  App.panel = function (id, def) { panels[id] = def; if (panelOrder.indexOf(id) < 0) panelOrder.push(id); renderCtx(); };
  /* a panel def's hidden() takes its tab away for now (Lyrics for a song with no words, or shown by the page or the Now
     column already: one place shows a thing at a time); hiddenNote (a string or a function) says why, and reveal(),
     when it returns true, has shown the listener where it is instead. The tab the listener chose (ctxWant) comes back
     once it is shown again. */
  function shown(id) { return !!panels[id] && !(panels[id].hidden && safeCall(panels[id].hidden)); }
  function hiddenNote(p) { return (typeof p.hiddenNote === 'function' ? safeCall(p.hiddenNote) : p.hiddenNote) || p.label + ' isn\'t available right now'; }
  var shownTabs = '';
  function retab() { if (panelOrder.filter(shown).join() !== shownTabs) renderCtx(); }
  App.on('route', retab); App.on('nowcol', retab);
  function panelOpen() {
    var mode = $('#app').getAttribute('data-ctx');
    return mode === 'open' || mode === 'docked' && !App.layout.get().panelCollapsed;
  }
  App.openPanel = function (tab) {
    if (tab) state.ctxTab = state.ctxWant = tab;
    var app = $('#app');
    if (window.innerWidth >= 1360) { app.setAttribute('data-ctx', 'docked'); App.layout.makeRoom('panel'); App.layout.set({ panelCollapsed: false }); }
    else app.setAttribute('data-ctx', 'open');
    renderCtx();
    var body = $('#ctx-body'); if (body && app.getAttribute('data-ctx') === 'open') setTimeout(function () { var t = $('.seg__btn[aria-selected="true"]', $('#ctx')); if (t) t.focus(); }, 60);
  };
  App.closePanel = function () {
    var app = $('#app');
    if (app.getAttribute('data-ctx') === 'docked') App.layout.set({ panelCollapsed: true });
    else app.setAttribute('data-ctx', 'closed');
    updatePanelButtons();
  };
  App.togglePanel = function (tab) {
    if (tab && panels[tab] && !shown(tab)) {
      if (!(panels[tab].reveal && safeCall(panels[tab].reveal))) App.toast(hiddenNote(panels[tab]), { icon: 'info' });
      return;
    }
    if (panelOpen() && (!tab || tab === state.ctxTab || split() && SPLIT.indexOf(tab) >= 0 && SPLIT.indexOf(state.ctxTab) >= 0)) App.closePanel();
    else App.openPanel(tab);
  };
  /* Queue and Lyrics side by side (or stacked, in a narrow panel) */
  function split() { return $('#app').hasAttribute('data-panel-split') && SPLIT.every(shown); }
  App.refreshPanel = function () { renderCtx(); };
  function renderCtx() {
    var host = $('#ctx'); if (!host) return;
    var two = split(), tabs = panelOrder.filter(function (id) { return shown(id) && !(two && id === 'lyrics'); });
    shownTabs = panelOrder.filter(shown).join();
    if (state.ctxWant && tabs.indexOf(state.ctxWant) >= 0) state.ctxTab = state.ctxWant;
    if (two && state.ctxTab === 'lyrics') state.ctxTab = 'queue';
    if (tabs.indexOf(state.ctxTab) < 0) state.ctxTab = tabs[0];
    var i = tabs.indexOf(state.ctxTab);
    $('#ctx-tabs').style.setProperty('--n', tabs.length); $('#ctx-tabs').style.setProperty('--i', i);
    patch($('#ctx-tabs'), '<span class="seg__ind" aria-hidden="true"></span>' + tabs.map(function (id) {
      var on = id === state.ctxTab, label = two && id === 'queue' ? panels.queue.label + ' + ' + panels.lyrics.label : panels[id].label;
      return '<button class="seg__btn" role="tab" id="ctx-tab-' + id + '" aria-controls="ctx-body" aria-selected="' + on + '" tabindex="' + (on ? 0 : -1) + '" data-ctx-tab="' + id + '">' + esc(label) + (id === 'activity' && activeJobs() ? ' <span class="badge-count">' + activeJobs() + '</span>' : '') + '</button>';
    }).join(''));
    var body = $('#ctx-body'), body2 = $('#ctx-body-2'), p = panels[state.ctxTab], second = two && state.ctxTab === 'queue' ? panels.lyrics : null;
    var title = $('#ctx-title'); if (title) title.textContent = p ? p.label.toLowerCase() : '';
    updateCtxMeta();
    host.toggleAttribute('data-two', !!second);
    body.setAttribute('aria-labelledby', 'ctx-tab-' + state.ctxTab);
    patch(body, p ? p.render(body) || '' : '');
    body2.hidden = !second;
    patch(body2, second ? second.render(body2) || '' : '');
    if (p && p.mount) p.mount(body);
    if (second && second.mount) second.mount(body2);
    renderStrip();
    updatePanelButtons();
  }
  /* the collapsed panel's icon strip: one button per tab, each opens the panel there */
  var STRIP_ICONS = { queue: 'queue', lyrics: 'lyrics', activity: 'activity' };
  function renderStrip() {
    var strip = $('#ctx-strip'); if (!strip) return;
    patch(strip, panelOrder.filter(shown).map(function (id) {
      var n = id === 'activity' ? activeJobs() : 0;
      return String(html`<button class="btn-icon btn-icon--bare" data-act="panel-expand" data-tab="${id}" aria-label="${panels[id].label}${n ? ', ' + n + ' running' : ''}" data-tip="${panels[id].label}">${icon(STRIP_ICONS[id] || 'expand')}${n ? html`<span class="btn-icon__badge">${n}</span>` : ''}</button>`);
    }).join(''));
  }
  /* the panel's meta line: #ctx[data-meta] + .region-meta (panel def meta(), e.g. queue "14 songs, 52 min") */
  function updateCtxMeta() {
    var p = panels[state.ctxTab], m = p && p.meta ? safeCall(p.meta) : '';
    $('#ctx').setAttribute('data-meta', m);
    var el = $('#ctx-meta'); if (el) el.textContent = m;
  }
  function updatePanelButtons() {
    var open = panelOpen(), two = split();
    $$('[data-act="toggle-panel"]').forEach(function (b) {
      var tab = b.getAttribute('data-tab');
      b.setAttribute('aria-pressed', String(open && (tab === state.ctxTab || two && state.ctxTab === 'queue' && SPLIT.indexOf(tab) >= 0)));
      b.setAttribute('aria-expanded', String(open));
    });
  }
  App.on('layout', function () { if (split() !== $('#ctx').hasAttribute('data-two') && state.ctxTab !== 'activity') renderCtx(); else updatePanelButtons(); });
  App.on('layout:size', function () { if (split() !== $('#ctx').hasAttribute('data-two') && state.ctxTab !== 'activity') renderCtx(); });
  document.addEventListener('click', function (e) {
    var t = e.target.closest('[data-ctx-tab]'); if (!t) return;
    state.ctxTab = state.ctxWant = t.getAttribute('data-ctx-tab'); renderCtx();
    var nb = $('[data-ctx-tab="' + state.ctxTab + '"]'); if (nb) nb.focus();
  });
  App.on('player', function (k) { if (k === 'queue' || k === 'track') { if (state.ctxTab === 'queue' || state.ctxTab === 'lyrics') renderCtx(); } });
  function layoutCtx() {
    var app = $('#app'), mode = app.getAttribute('data-ctx'), w = window.innerWidth;
    if (w >= 1360 && mode !== 'docked') app.setAttribute('data-ctx', 'docked');
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
    /* private listening shows with the track, wherever the design puts it: the bar's tools are laid out per design */
    var priv = state.privateListening ? html`<a class="pbar__private" href="#/settings/privacy" aria-label="Private listening is on. Open its setting" data-tip="Private listening: nothing you play here is recorded or scrobbled" data-fit="4">${icon('eye-off')}<span class="pbar__private-label">Private</span></a>` : '';
    var nowMarkup;
    if (idle) {
      nowMarkup = html`<span class="bezel-thumb bezel-thumb--empty" aria-hidden="true"><span></span></span><div class="pbar__idle">${priv || html`${icon('sparkle')}<span>Search or pick something from Home</span>`}</div>`;
    } else {
      /* the cover opens Now Playing; the title, and the "Playing from" line where the design shows it, open the place
         it plays from at the song; with no such page (a single song, radio, your queue), the title opens the album */
      var from = P.playingFrom(), fromPath = from && App.placePath(from);
      nowMarkup = html`<span class="cover-zoom"><a class="bezel-thumb" href="#/now" aria-label="Open Now Playing: ${t.title}" data-tip="Now Playing">${ui.cover(t, 52, { hero: true })}</a>${ui.coverZoom(t.id)}</span>
        <div class="pbar__text">
          ${ui.playingFrom('pbar__from')}
          <div class="pbar__title">${fromPath ? html`<a href="#${fromPath}" data-act="go-playing" data-tip="Show the song in ${App.placeName(from)}">${t.title}</a>` : html`<a href="#/album/${t.albumId}">${t.title}</a>`}</div>
          <div class="pbar__artist"><a href="#/artist/${t.artistId}">${t.artist}</a></div>
          <div class="pbar__badges" data-fit-row>${playingYoutube() ? '' : ui.sourceBadge(t, { button: true, fit: 1 })}${playingFormat(t) ? ui.qualityChip(playingFormat(t), { solid: true, button: true, id: t.id, fit: 3 }) : ''}${ps.belowMinimum ? ui.belowBadge({ fit: 2 }) : ''}${priv}</div>
        </div>`;
    }
    patch(now, nowMarkup);
    /* the song side's words changed: the bar fits again (only then: a queue change re-renders the bar too) */
    var fitKey = String(nowMarkup);
    if (App.layout && now._fitKey !== fitKey) { now._fitKey = fitKey; App.layout.refit(); }
    // state tab above the seek bar
    var line = P.statusText(), stEl = $('#pbar-state');
    if (line && st !== 'idle') {
      var err = st === 'failed' || st === 'jellyfin-error';
      var acts = st === 'failed' ? html`<button class="btn btn--quiet" data-act="retry-search">Retry search</button><button class="btn btn--quiet" data-act="pick-source">Pick another source</button><button class="btn btn--quiet" data-act="next">Skip</button>`
        : st === 'jellyfin-error' ? html`<button class="btn btn--quiet" data-act="try-soulseek">Try Soulseek instead</button>`
        : st === 'stalled' || (st === 'buffering' && ps.fallback) ? html`<button class="btn btn--quiet" data-act="pick-source">Choose manually</button>`
        : st === 'ended' ? html`<button class="btn btn--quiet" data-act="start-radio-current">Start radio</button>` : '';
      var ic = err ? 'warning' : st === 'finding' || st === 'queued' || st === 'buffering' ? 'soulseek' : st === 'stalled' ? 'refresh' : ps.seekWait ? 'clock' : ps.crossfade ? 'shuffle' : 'info';
      /* a kept seek's line counts down with every stream event: the tab is a live region, so screen readers hear it
         once from the announcer (seek-kept) instead of at each change */
      patch(stEl, html`${icon(ic)}<span ${raw(ps.seekWait ? 'aria-hidden="true"' : '')}>${line}</span>${acts}`);
      stEl.classList.toggle('is-error', err); stEl.classList.toggle('is-seek-wait', !!ps.seekWait); stEl.hidden = false;
    } else { stEl.hidden = true; stEl.innerHTML = ''; }
    // transport
    var busy = st === 'finding' || st === 'queued' || st === 'buffering' || st === 'stalled';
    $$kept('[data-bind="play"]').forEach(function (b) {
      var playingNow = ps.playing && !idle && st !== 'ended' && st !== 'failed';
      b.setAttribute('aria-label', playingNow ? 'Pause' : 'Play'); b.setAttribute('data-tip', playingNow ? 'Pause (Space)' : 'Play (Space)');
      $('use', b).setAttribute('href', '#i-' + (playingNow ? 'pause' : 'play'));
      $('svg', b).classList.toggle('ic-play', !playingNow);
      b.classList.toggle('is-buffering', busy && ps.playing);
    });
    $$kept('[data-bind="shuffle"]').forEach(function (b) { b.setAttribute('aria-pressed', String(ps.shuffle)); });
    $$kept('[data-bind="repeat"]').forEach(function (b) {
      b.setAttribute('aria-pressed', String(ps.repeat !== 'off'));
      var label = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat];
      b.setAttribute('aria-label', label); b.setAttribute('data-tip', label + ' (R)');
      $('use', b).setAttribute('href', '#i-' + (ps.repeat === 'one' ? 'repeat-one' : 'repeat'));
    });
    $$kept('[data-bind="like"]').forEach(function (b) {
      if (!t) { b.disabled = true; b.removeAttribute('data-like'); b.setAttribute('aria-pressed', 'false'); b.classList.remove('is-liked'); $('use', b).setAttribute('href', '#i-heart'); return; } b.disabled = false;
      b.setAttribute('data-like', t.id); b.setAttribute('data-id', t.id); b.removeAttribute('aria-pressed');
    });
    $$kept('[data-bind="save"]').forEach(function (b) {
      if (!t) { b.disabled = true; return; } b.disabled = false;
      var sv = state.saves[t.id], saved = App.isSaved(t.id), stp = $('.pbar__save-step', b);
      var label = saved ? 'Saved to library' : sv && sv.status === 'saving' ? 'Saving, step ' + sv.step + ' of 5' : sv && sv.status === 'failed' ? 'Save failed, retry' : 'Save to library (Shift+S)';
      b.setAttribute('aria-label', label); b.setAttribute('data-tip', label); b.setAttribute('data-id', t.id);
      $('use', b).setAttribute('href', '#i-' + (saved ? 'saved' : sv && sv.status === 'failed' ? 'warning' : 'save'));
      b.setAttribute('aria-pressed', String(saved));
      if (sv && sv.status === 'saving') { if (!stp) { stp = document.createElement('span'); stp.className = 'pbar__save-step'; stp.setAttribute('aria-hidden', 'true'); b.appendChild(stp); } stp.textContent = sv.step + '/5'; }
      else if (stp) stp.remove();
    });
    $$kept('[data-bind="mute"]').forEach(function (b) {
      var v = ps.muted ? 0 : ps.volume;
      $('use', b).setAttribute('href', '#i-' + (ps.muted ? 'mute' : v === 0 ? 'volume-0' : v < .5 ? 'volume-1' : 'volume-2'));
      b.setAttribute('aria-label', ps.muted ? 'Unmute (M)' : 'Mute (M)'); b.setAttribute('data-tip', ps.muted ? 'Unmute (M)' : 'Mute (M)');
      b.setAttribute('aria-pressed', String(ps.muted));
    });
    $$kept('[data-bind="xfade"]').forEach(function (el) { el.hidden = !ps.crossfade; });
    // mini-player text
    setText($('#mini-title'), idle ? 'Nothing playing' : t.title);
    /* the artist, and where the song plays from (the whole text opens Now Playing, whose "Playing from" opens it);
       "your queue" goes without saying */
    var miniFrom = !idle && P.playingFrom(); if (miniFrom && miniFrom.kind === 'queue') miniFrom = null;
    patch($('#mini-sub'), idle ? 'Search or pick something from Home' : line && (st !== 'streaming' && st !== 'library' || ps.seekWait) ? esc(line)
      : html`<span class="mini__artist">${t.artist}</span>${miniFrom ? html`<span class="mini__from">${App.placeName(miniFrom)}</span>` : ''}`);
    var mt = $('#mini-thumb'); patch(mt, idle ? '<span></span>' : ui.cover(t, 40, { hero: true })); mt.classList.toggle('bezel-thumb--empty', idle);
    App.syncRows();
    updateSeeks();
    updateMediaSession();
  }
  /* the format actually playing: the picked peer's file for Soulseek, the library file otherwise */
  function playingFormat(t) { return ps.peer && ps.status !== 'library' ? ps.peer.format || t.format : t.format; }
  /* the song plays from YouTube: its own chip ("YouTube · Opus 160") says so, not the Soulseek badge */
  function playingYoutube() { return !!(ps.peer && ps.status !== 'library' && ps.peer.name === 'YouTube'); }
  App.on('player', renderBar);
  App.on('player', function (k) { if (k === 'track' || k === 'queue') ui.warmArt(P.upNext(1)[0]); });
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
    var links = [['#/playlists', 'playlists', 'Playlists'], ['#/liked', 'heart', 'Liked songs', 'liked:'], ['#/history', 'clock', 'History', 'history:'],
      ['#/following', 'follow', 'Following'], ['#/downloads', 'download', 'Downloads', 'downloads:'], ['#/stats', 'stats', 'Stats'], ['#/settings', 'settings', 'Settings'], ['#/classic', 'classic', 'Classic mode']];
    var d = App.openSheet({ title: 'more', body: html`<nav class="menu-list" aria-label="More">${links.map(function (l) {
      return html`<a class="menu__item${ui.placeCls(l[3])}" href="${l[0]}" data-close ${l[3] ? raw('data-place="' + l[3] + '"') : ''}>${icon(l[1])}<span>${l[2]}</span>${ui.placeMark(l[3])}</a>`;
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
  /* out of the hub's reach, playlist changes go to the outbox (offline.js) and show here at once. A playlist made
     offline has a temporary id (pl-tmp-…) until the outbox delivers it; changes to it can only wait there. */
  function isTemp(plId) { return /^pl-tmp-/.test(plId); }
  function plOp(op, plId, fields) { var o = Object.assign({ op: op }, fields); o[isTemp(plId) ? 'tempId' : 'playlistId'] = plId; return o; }
  function plSend(path, opts, op) { return isTemp(op.tempId || '') ? App.outbox.add(op) : App.outbox.call(path, opts, op); }
  function localPlaylist(plId, patch) {
    var p = M.playlist(plId), at = Math.floor(Date.now() / 1000); if (!p) return null;
    var next = Object.assign({}, p, patch, { updated: at });
    if (patch.trackIds) {
      var added = {}; p.items.forEach(function (i) { added[i.trackId] = i.added; });
      next.items = patch.trackIds.map(function (id) { return { trackId: id, added: added[id] || at }; }); delete next.trackIds;
    }
    return putPlaylist(next);
  }
  App.createPlaylist = function (fields) {
    var made = function () {
      var at = Math.floor(Date.now() / 1000), tempId = 'pl-tmp-' + Date.now().toString(36);
      var ids = fields.trackIds || [];
      return App.outbox.add({ op: 'playlist.create', tempId: tempId, name: fields.name, desc: fields.desc || '', trackIds: ids }).then(function () {
        return putPlaylist({ id: tempId, name: fields.name, desc: fields.desc || '', created: at, updated: at, items: ids.map(function (id) { return { trackId: id, added: at }; }) });
      });
    };
    if (App.outbox.offline()) return made();
    return M.api('/api/playlists', { method: 'POST', body: fields }).then(putPlaylist, function (err) {
      if (err.status === 0) return made();
      App.toast('Couldn\'t create the playlist: ' + err.message, { kind: 'error' }); return null;
    });
  };
  App.savePlaylistOrder = function (plId, trackIds) {
    return plSend('/api/playlists/' + encodeURIComponent(plId) + '/tracks', { method: 'PUT', body: trackIds }, plOp('playlist.edit', plId, { trackIds: trackIds })).then(function (p) {
      if (p && p.queued) return localPlaylist(plId, { trackIds: trackIds });
      if (p && p.id) return putPlaylist(p);
      return loadPlaylists();
    });
  };
  /* how a playlist is shown (its `sort` { key, dir }): it changes here at once and the hub keeps it for every device
     (PUT /api/playlists/{id}/sort). A view setting, not an edit, so it doesn't go through the outbox: when the hub
     can't keep it, it stays here for this visit and a toast says so. */
  App.setPlaylistSort = function (plId, sort) {
    var path = '/api/playlists/' + encodeURIComponent(plId), read = M.peek(path), p = M.playlist(plId);
    if (read) read.sort = sort;
    if (p) putPlaylist(Object.assign({}, p, { sort: sort })); else App.refresh();
    if (isTemp(plId)) return Promise.resolve();
    return M.api(path + '/sort', { method: 'PUT', body: sort }).then(function (back) {
      if (read) read.sort = back.sort;
      if (M.playlist(plId)) putPlaylist(Object.assign({}, M.playlist(plId), { sort: back.sort }));
    }, function (err) {
      App.toast('Sorted here only: lull couldn\'t keep it for next time. ' + err.message, { kind: 'error' });
    });
  };
  /* name and description: PATCH /api/playlists/{id} */
  App.updatePlaylist = function (plId, fields) {
    return plSend('/api/playlists/' + encodeURIComponent(plId), { method: 'PATCH', body: fields }, plOp('playlist.edit', plId, fields)).then(function (p) {
      return p && p.queued ? localPlaylist(plId, fields) : loadPlaylists();
    });
  };
  App.deletePlaylist = function (plId) {
    return plSend('/api/playlists/' + encodeURIComponent(plId), { method: 'DELETE' }, plOp('playlist.delete', plId, {})).then(function (r) {
      if (r && r.queued) { App.store.update('playlists', function (l) { return l.filter(function (x) { return x.id !== plId; }); }); return; }
      return loadPlaylists();
    });
  };
  /* songs dropped on a playlist: only those it lacks, unless `again` (the toast's "Add again" when it has them all) */
  function addToPlaylist(plId, ids, again) {
    ids = [].concat(ids).filter(function (id, i, all) { return all.indexOf(id) === i; });
    var p = M.playlist(plId); if (!p) return;
    var before = p.items.map(function (i) { return i.trackId; });
    var adding = again ? ids : ids.filter(function (id) { return before.indexOf(id) < 0; });
    if (!adding.length) {
      App.toast((ids.length === 1 ? 'Already in ' : 'All already in ') + p.name, { icon: 'check', action: 'Add again', onAction: function () { addToPlaylist(plId, ids, true); } });
      return;
    }
    var skipped = ids.length - adding.length;
    App.savePlaylistOrder(plId, before.concat(adding)).then(function () {
      App.toast('Added to ' + p.name + (skipped ? ' (' + skipped + ' already in it)' : ''), { icon: 'playlists', action: 'Undo', onAction: function () { App.savePlaylistOrder(plId, before); } });
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
  /* the Queue panel (phone: Now Playing, Queue tab) */
  function showQueue() { if (window.innerWidth < 640) App.go('/now?tab=queue'); else App.openPanel('queue'); }
  /* start radio, then show where it landed */
  function startRadio(seed) {
    P.startRadio(seed).then(function (ok) { if (ok) showQueue(); });
  }
  /* an album's songs, fetched when only the album head is known yet (shelf tiles) */
  /* an album's songs, known to the cache: a list's album entity carries trackIds without the tracks, and the player can't
     load a song it doesn't know, so the album is read whenever any of them is missing */
  function albumTracks(id) {
    var a = M.album(id);
    if (a && a.trackIds.length && a.trackIds.every(function (t) { return M.track(t); })) return Promise.resolve(a.trackIds);
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
  /* menus by what they're for, opened by id at an anchor: track here, album / artist / playlist from library.js; a
     right-click on an item without its own "…" opens these (section 11) */
  App.itemMenus = {};
  /* a song's menu (its "…" in any list, a right-click on it) */
  function trackMenu(id, anchor) {
    var t = M.track(id); if (!t) return;
    App.menu(anchor, [
      { label: 'Play next', icon: 'queue', run: function () { P.playNext([id]); } },
      { label: 'Add to queue', icon: 'plus', run: function () { P.addToQueue([id]); } },
      { label: App.isSaved(id) ? 'In your library' : 'Save to library', icon: App.isSaved(id) ? 'saved' : 'save', run: function () { App.saves.start([id]); } },
      { label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist([id]); } },
      { label: 'Start radio', icon: 'radio', run: function () { startRadio({ kind: 'track', id: id }); } },
      App.downloads.has(id) ? { label: 'Remove download', icon: 'downloaded', run: function () { App.downloads.remove([id]); } }
        : { label: 'Download', icon: 'download', run: function () { App.downloads.keep({ kind: 'track', id: id }, [id]); } },
      'sep',
      { label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } },
      { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + t.artistId); } },
      App.coverItem('track', id),
      { label: 'Show sources', icon: 'soulseek', run: function () { App.hooks.sourcePicker(id); } }
    ], { label: 'Actions for ' + t.title });
  }
  App.itemMenus.track = trackMenu;
  /* several selected songs (section 11, selecting rows): their "…" and right-click act on all of them */
  function songsMenu(list, anchor) {
    var unsaved = list.filter(function (id) { return !App.isSaved(id); }), unliked = list.filter(function (id) { return !App.isLiked(id); });
    App.menu(anchor, [
      { label: 'Play next', icon: 'queue', run: function () { P.playNext(list); } },
      { label: 'Add to queue', icon: 'plus', run: function () { P.addToQueue(list); } },
      { label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist(list); } },
      unsaved.length ? { label: 'Save to library', icon: 'save', run: function () { App.saves.start(unsaved); } } : null,
      unliked.length ? { label: 'Like', icon: 'heart', run: function () { setLiked(unliked, true); App.toast('Added ' + unliked.length + ' to Liked songs', { icon: 'heart-fill' }); } } : null,
      { label: 'Download', icon: 'download', run: function () { App.downloads.keep({ kind: 'track', id: list[0] }, list); } }
    ].filter(Boolean), { label: 'Actions for ' + list.length + ' songs' });
  }
  App.itemMenus.songs = songsMenu;
  /* the player's "…" (the bar's, Now Playing's, a right-click on the playing song): the song's actions, then the bar's
     views. Every tool a narrow bar lets give way (data-fit) is here: Like and Save with the song, Lyrics, Classic,
     Devices and Device view always, and the equaliser (screens/eq.js) */
  function playerMenu(anchor) {
    var t = P.current();
    App.needClassic().then(null, function () {});      // it lists Classic mode: have it ready
    var views = [
      { label: 'Lyrics', icon: 'lyrics', run: function () { App.togglePanel('lyrics'); } },
      { label: 'Classic mode', icon: 'classic', run: function () { App.go('/classic'); } },
      { label: 'Play on another device', icon: 'connect', run: function () { App.connect.open(anchor); } },
      { label: 'Device view', icon: 'device', run: actions['open-now'] },
      { label: 'Equaliser', icon: 'equaliser', run: function () {
        App.needEq().then(function () { App.eq.open(anchor); }, function (err) { App.toast(err.message, { kind: 'error', icon: 'warning' }); });
      } }
    ];
    if (!t) { App.menu(anchor, views, { label: 'More player actions' }); return; }
    var liked = App.isLiked(t.id), saved = App.isSaved(t.id);
    App.menu(anchor, [
      { label: liked ? 'Remove from Liked' : 'Like', icon: liked ? 'heart-fill' : 'heart', run: function () { App.toggleLike(t.id); } },
      { label: saved ? 'In your library' : 'Save to library', icon: saved ? 'saved' : 'save', run: function () { if (!saved) App.saves.start([t.id]); } },
      { label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist([t.id]); } },
      { label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } },
      { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + t.artistId); } },
      App.coverItem('track', t.id),
      { label: 'Start radio', icon: 'radio', run: function () { startRadio({ kind: 'track', id: t.id }); } },
      { label: 'Show source', icon: 'soulseek', run: function () { App.hooks.sourceDetails(); } },
      { label: 'Copy link', icon: 'link', run: function () { App.copyShareLink('track', t.id); } },
      'sep'
    ].concat(views), { label: 'More player actions' });
  }
  Object.assign(actions, {
    'noop': function () {},
    'noop-saved': function () { App.toast('Already saved to your library', { icon: 'saved' }); },
    'reload': function () { location.reload(); },
    'play-track': function (el) { var id = el.getAttribute('data-id'); P.playTrack(id, listIds(el)); },
    'play-album': function (el) {
      var id = el.getAttribute('data-id');
      withAlbum(id, function (list) { var a = M.album(id); P.playList(list, 0, { context: App.place('album', id, a.title) }); App.toast('Playing ' + a.title, { icon: 'play' }); });
    },
    'shuffle-album': function (el) {
      var id = el.getAttribute('data-id');
      withAlbum(id, function (list) { P.setShuffle(true); P.playList(list, Math.floor(Math.random() * list.length), { context: App.place('album', id, M.album(id).title) }); });
    },
    'play-playlist': function (el) {
      var p = state.playlists.filter(function (x) { return x.id === el.getAttribute('data-id'); })[0];
      if (!p.items.length) { App.toast('This playlist is empty. Drag songs here or use Add to playlist.', { icon: 'info' }); return; }
      P.playList(p.items.map(function (i) { return i.trackId; }), 0, { context: App.place('playlist', p.id, p.name) });
    },
    'play-ids': function (el) { P.playList(ids(el), 0); },
    'go-playing': function () { App.goPlaying(); },
    'view-cover': function (el) { App.viewCover({ kind: el.getAttribute('data-kind') || 'track', id: el.getAttribute('data-id') }); },
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
      var sel = App.selectedIds(el.closest('.trow'));
      if (sel.length > 1) songsMenu(sel, el); else trackMenu(el.getAttribute('data-id'), el);
    },
    'player-more': function (el) { playerMenu(el); },
    'play': function () { P.toggle(); }, 'next': function () { P.next(); }, 'prev': function () { P.prev(); },
    'shuffle': function () { P.setShuffle(); }, 'repeat': function () { P.cycleRepeat(); }, 'mute': function () { P.toggleMute(); },
    'retry-search': function () { P.retry(); App.toast('Searching Soulseek again', { icon: 'refresh' }); },
    'try-soulseek': function () { P.trySoulseek(); },
    'reload-read': function (el) { var path = el.getAttribute('data-id'); if (path) M.reload(path); else { M.invalidate(); App.refresh(); } },
    'toggle-panel': function (el) { App.togglePanel(el.getAttribute('data-tab')); },
    'close-panel': function () { App.closePanel(); },
    /* a save waiting for a good enough copy: say so, and offer to stop it */
    'save-waiting': function (el) {
      var id = el.getAttribute('data-id'), s = App.saves.get(id);
      App.toast((s && s.error) || 'Waiting for a copy that meets your saving minimum', { icon: 'clock', action: 'Stop waiting', onAction: function () { App.saves.cancel(id); } });
    },
    'panel-expand': function (el) { App.openPanel(el.getAttribute('data-tab')); },
    'open-activity': function () { App.openPanel('activity'); },
    /* the player bar's Device view: the wrapper's mini player window inside the desktop app, Now Playing elsewhere */
    'open-now': function () { if (window.LullDesktop && LullDesktop.inWrapper) LullDesktop.openMini(); else App.go('/now'); },
    'theme': function () { App.toggleTheme(); },
    'status': function (el) { statusPopover(el); },
    'shelf': function (el) { shelfPopover(el); },
    'more-sheet': function () { App.hooks.moreSheet(); },
    'shortcuts': function () { App.hooks.shortcuts(); },
    'palette': function () { App.hooks.palette(); },
    'classic': function () { App.go('/classic'); },
    'open-sources': function () { App.closePopover(); App.go('/settings/sources'); },
    'open-lastfm': function () { App.closePopover(); App.go('/settings/lastfm'); },
    'jellyfin-rescan': function () { serverAction('/api/library/rescan', 'Jellyfin rescan started'); },
    'lastfm-retry': function () { serverAction('/api/lastfm/retry', 'Sending queued scrobbles to Last.fm'); },
    'open-cache': function () { App.closePopover(); App.go('/settings/cache'); },
    'clear-search': function () { var i = $('#header-search'); i.value = ''; i.focus(); toggleClear(); clearedSearch(); },
    'back': function () { App.back(); },
    'skip': function () { $('#view').focus(); }
  });
  /* ---------- copying links ----------
     "Copy link" copies one link that works for anyone: lull's open page (site/, ADR-0032) followed by `#lull?`, the
     song's names and public ids, and its public page (song.link, which opens in every music app, or Last.fm), from
     GET /api/share/{kind}/{id} (kind track | album | artist). Clicked, the open page opens the listener's lull or
     offers the public page; any lull opens it as its own page (App.openLink, on another hub too); never this hub's
     address. A playlist's link is the page it was imported from. The clipboard write
     starts inside the click, with the text still on its way (ClipboardItem takes a promise), so the browser allows
     it once the hub has answered. */
  var SHARE_NOUN = { track: 'song', album: 'album', artist: 'artist' };
  App.copyShareLink = function (kind, id) {
    var share = M.api('/api/share/' + kind + '/' + encodeURIComponent(id));
    App.copyLink(share.then(function (r) { return r.link; }), function (r) {
      return 'Link copied: it opens in lull, and as ' + (r.source === 'lastfm' ? 'the Last.fm page' : 'a page for every music app') + ' without it';
    }, share, function (err) {
      return err.status === 404 ? 'There\'s no link for this ' + SHARE_NOUN[kind] + '.' : 'Couldn\'t make a link: ' + err.message;
    });
  };
  /* link: a promise of the text; said(answer) and failed(err) word the toasts; answer: what said() reads */
  App.copyLink = function (link, said, answer, failed) {
    answer = answer || link;
    clipboardWrite(link).then(function () {
      answer.then(function (r) { App.toast(said(r), { icon: 'link' }); });
    }, function () {
      link.then(copyFailed, function (err) { App.toast(failed(err), { kind: 'error', icon: 'link' }); });
    });
  };
  function clipboardWrite(text) {
    if (!navigator.clipboard) return Promise.reject(new Error('no clipboard'));
    if (window.ClipboardItem && navigator.clipboard.write) {
      var blob = text.then(function (t) { return new Blob([t], { type: 'text/plain' }); });
      blob.catch(function () { /* reported by the caller, whose own promise it is */ });
      return navigator.clipboard.write([new ClipboardItem({ 'text/plain': blob })]);
    }
    return text.then(function (t) { return navigator.clipboard.writeText(t); });
  }
  function copyFailed(url) { App.toast('This browser didn\'t allow copying. The link is ' + url, { kind: 'error', icon: 'link' }); }
  document.addEventListener('click', function (e) {
    var el = e.target.closest('[data-act]'); if (!el || el.disabled) return;
    var fn = actions[el.getAttribute('data-act')];
    if (!fn) { console.warn('No action registered for', el.getAttribute('data-act')); return; }
    if (el.tagName === 'A') e.preventDefault();
    fn(el, e);
    if (el.hasAttribute('data-close')) App.closeTop();
  });
  /* Context menus: right-click, the Menu key (or Shift+F10) and a touch long-press open lull's own "…" menu, at the
     pointer, instead of the browser's (the desktop app's WebView2 draws its own unless the page takes the event).
     Everything that has a "…" opens that same menu, built by the same code: MENU_OF names each item and where its "…"
     is (in it, or elsewhere in a scope around it: the player bar's song opens the bar's). Items without one (tiles, a
     rail playlist, the search's top result, Home's lead) open the menu of what they are (App.itemMenus). Text fields and selected
     text keep the native menu, for copy and paste; anywhere else nothing changes. */
  var MENU_OF = [
    ['.trow, .disc-pick, .lib-hero', null, '[data-act$="menu"][aria-haspopup="menu"]'],
    ['.lib-pl__head', '.lib-pl', '[data-act="lib-pl-menu"]'],
    ['.pbar__now', '.pbar', '[data-act="player-more"]'],
    ['.now-head, .now__dev, .now__devc', '.now', '[data-act="player-more"]']
  ];
  var ITEM_LINK = /^#\/(album|artist|playlist)\/([^/?#]+)/;
  function contextMenuFor(target) {
    for (var i = 0; i < MENU_OF.length; i++) {
      var item = target.closest(MENU_OF[i][0]); if (!item) continue;
      var scope = MENU_OF[i][1] ? item.closest(MENU_OF[i][1]) : item;
      var more = scope && scope.querySelector(MENU_OF[i][2]);
      if (more && actions[more.getAttribute('data-act')]) return function () { actions[more.getAttribute('data-act')](more); };
    }
    var thing = target.closest('.tile, .rail__pl, .disc-top, .disc-lead'); if (!thing) return null;
    var song = thing.matches('.disc-top, .disc-lead') && thing.querySelector('[data-act="play-track"][data-id]');
    if (song) return function () { App.itemMenus.track(song.getAttribute('data-id'), song); };
    var link = thing.matches('a') ? thing : thing.querySelector('.tile__title a[href], .tile__art a[href], a[href^="#/album/"], a[href^="#/artist/"]');
    var m = link && ITEM_LINK.exec(link.getAttribute('href') || '');
    var open = m && App.itemMenus[m[1]];
    return open ? function () { open(decodeURIComponent(m[2]), link); } : null;
  }
  function keepsNativeMenu(e) {
    if (e.target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])')) return true;
    var sel = window.getSelection();
    return !!(sel && !sel.isCollapsed && sel.rangeCount && sel.getRangeAt(0).intersectsNode(e.target));
  }
  document.addEventListener('contextmenu', function (e) {
    if (!(e.target instanceof Element) || keepsNativeMenu(e)) return;
    var open = contextMenuFor(e.target); if (!open) return;
    e.preventDefault();
    App.closePopover();
    // a pointer opens it where it is; the keyboard's menu key (no position) opens it by the item's own button
    contextAt = e.clientX || e.clientY ? { x: e.clientX, y: e.clientY } : null;
    try { open(); } finally { contextAt = null; }
  });
  // a bare <a data-close> inside a sheet closes it after navigating
  document.addEventListener('click', function (e) {
    var a = e.target.closest('a[data-close]'); if (a && !a.hasAttribute('data-act')) setTimeout(App.closeTop, 0);
  });
  /* selecting rows: a click selects one, Ctrl (Cmd on a Mac) adds or drops one, Shift reaches from the last one
     picked; Shift with Up or Down does the same from the keyboard. The "…", a right-click and dragging then act on all
     of them (App.selectedIds) */
  var selFrom = null;
  function selectRows(row, how) {
    var rows = $$('.trow[data-track-id]', row.parentElement);
    if (how === 'range' && selFrom && rows.indexOf(selFrom) >= 0) {
      var a = rows.indexOf(selFrom), b = rows.indexOf(row);
      rows.forEach(function (r, i) { r.classList.toggle('is-selected', i >= Math.min(a, b) && i <= Math.max(a, b)); });
    } else if (how === 'add') { row.classList.toggle('is-selected'); selFrom = row; }
    else { rows.forEach(function (r) { r.classList.toggle('is-selected', r === row); }); selFrom = row; }
    var n = rows.filter(function (r) { return r.classList.contains('is-selected'); }).length;
    if (how !== 'one' && n > 1) App.announce(n + ' songs selected');
  }
  App.selectedIds = function (row) {
    if (!row || !row.classList.contains('is-selected')) return row ? [row.getAttribute('data-track-id')] : [];
    return $$('.trow.is-selected[data-track-id]', row.parentElement).map(function (r) { return r.getAttribute('data-track-id'); });
  };
  document.addEventListener('click', function (e) {
    var r = e.target.closest && e.target.closest('.trow[data-track-id]');
    if (!r || e.target.closest('button, a, input')) return;
    selectRows(r, e.shiftKey ? 'range' : e.ctrlKey || e.metaKey ? 'add' : 'one');
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
    { group: 'Lists', keys: ['Shift+Up', 'Shift+Down'], label: 'Select several songs (or Ctrl+click, Shift+click)' },
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
    if (layers.length && k !== '?') return;            // sheets/dialogs own the keyboard
    var row = a && a.closest && a.closest('.trow');
    if (row) {
      var list = row.parentElement, rows = $$('.trow', list), i = rows.indexOf(row);
      if ((k === 'ArrowDown' || k === 'ArrowUp') && e.altKey) { e.preventDefault(); App.emit('row:move', { row: row, list: list, index: i, dir: k === 'ArrowDown' ? 1 : -1 }); return; }
      if (k === 'ArrowDown' || k === 'ArrowUp') {
        e.preventDefault(); var n = rows[i + (k === 'ArrowDown' ? 1 : -1)];
        if (n) { if (e.shiftKey) { if (!selFrom || rows.indexOf(selFrom) < 0) selFrom = row; selectRows(n, 'range'); } else selectRows(n, 'one'); n.focus(); }
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
      seekto: function (d) { T().seek(d.seekTime); },                     // the engine keeps or clamps it, like the seek bar's
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
      $('.drop-overlay__label', o).textContent = App.current.path === '/classic' ? 'Drop .wsz to apply skin' : 'Drop a playlist to import';
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
      App.openFiles([].slice.call(e.dataTransfer.files));
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
     15. Server events and boot: pairing (GET /api/ping), /health, the event stream, collections,
         service worker. Each source of truth is fetched once here and kept current by events.
     ===================================================================== */
  /* `service`, `stream`, `save`, `job`, `transfer` drive the engines here; every topic is also re-emitted as
     App.emit('server:<topic>', payload) for screens (search.results -> discover.js). */
  var serverHandlers = {
    service: function (e) { applyService(e.name, e.service); },
    stream: onStreamEvent,
    save: onSaveEvent,
    job: onJobEvent,
    /* bare songs the hub resolved (app.js has absorbed their entities): their covers fill in */
    'catalog.tracks': function () { repaintCovers(); },
    transfer: onTransferEvent,
    /* likes, follows and settings changed elsewhere (another device or tab) */
    like: function (e) { if (App.isLiked(e.trackId) !== e.liked) { App.store.update('liked', function (l) { if (e.liked) l[e.trackId] = true; else delete l[e.trackId]; }); App.syncRows(); } },
    follow: function (e) { if (App.isFollowed(e.id) !== e.followed) App.store.update('followed', function (f) { if (e.followed) f[e.id] = true; else delete f[e.id]; }); },
    settings: function (s) { applyPlaybackSettings(s); },
    resync: function () { refetchAll(); }
  };
  Object.keys(serverHandlers).forEach(M.listen);
  function onServerEvent(topic, payload) {
    if (payload && payload.entities) M.absorb(payload.entities);
    if (serverHandlers[topic]) serverHandlers[topic](payload);
    App.emit('server:' + topic, payload);
  }
  /* App.server: { reachable, version, authorized, checked } -- read by the banners, the pairing gate (setup.js) and
     Settings > App. `checked`: /api/ping has answered once, so `authorized` is known. */
  App.server = { reachable: true, version: null, authorized: true, checked: false };
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
      loadPlaylists(), loadJobs(), loadSettings(), loadPrivacy()
    ]).then(function () { App.syncRows(); });
  }
  /* private listening is kept by the hub per device (GET /api/devices: `me` and each device's `private`), so it
     survives reloads and the hub enforces it; this reads it, and App.setPrivateListening changes it */
  function loadPrivacy() {
    return M.api('/api/devices').then(function (r) {
      var me = (r.items || []).filter(function (d) { return d.id === r.me; })[0];
      App.store.set('privateListening', !!(me && me.private));
    }, function (err) { console.warn('Private listening unknown:', err.message); });
  }
  App.setPrivateListening = function (on) {
    return M.api('/api/devices/me', { method: 'PATCH', body: { private: !!on } }).then(function () {
      App.store.set('privateListening', !!on);
      App.announce(on ? 'Private listening on' : 'Private listening off');
    });
  };
  function renderPrivate() { $$('#mini [data-bind="private"]').forEach(function (el) { el.hidden = !state.privateListening; }); }
  /* Settings > Playback values the engine needs; a value may come back as { value, source: 'config' } */
  function loadSettings() {
    return M.api('/api/settings').then(applyPlaybackSettings, function (err) { console.warn('Settings unavailable, playback uses defaults:', err.message); });
  }
  function applyPlaybackSettings(s) {
    var pb = (s && s.playback) || {}, v = function (x) { return x && typeof x === 'object' && 'value' in x ? x.value : x; };
    App.setPlayback({ gapless: v(pb.gapless) !== false, crossfade: +v(pb.crossfade) || 0, normalisation: v(pb.normalisation) || 'off',
      loudnessTarget: +v(pb.loudnessTarget) || -14, radioWhenQueueEnds: v(pb.radioWhenQueueEnds) !== false, volumeStep: +v(pb.volumeStep) || 5,
      prefetchCount: v(pb.prefetchCount) == null ? 2 : +v(pb.prefetchCount), switchToBetter: !s || !s.sources || v(s.sources.switchToBetter) !== false,
      jumpWhenDownloaded: v(pb.jumpWhenDownloaded) !== false });
  }
  App.reloadCollections = loadCollections;
  /* after a reconnect or a resync: everything the events would have told us */
  function refetchAll() {
    forgetArtWaits();
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
      P.resyncStream();
      setServer({ reachable: true });
    }, function () { loadHealth(); });
  }
  /* the pairing gate (setup.js) calls App.start once the hub has set this device's cookie */
  App.checkHealth = loadHealth;
  App.start = function () {
    var wasLocked = !App.server.authorized;
    setServer({ authorized: true, checked: true });
    if (wasLocked) { M.invalidate(); renderRoute(true); }       // reads made behind the gate failed with 401
    connectEvents(); loadCollections();
  };
  /* 204: paired. 401: not paired (or unpaired since), M.onUnauthorized opens the gate. Unreachable: start offline,
     the shell, downloads and the cached UI still work. */
  function checkPairing() {
    return M.api('/api/ping').then(App.start, function (err) {
      if (err.status === 401) { if (events) { events.close(); events = null; } setServer({ checked: true }); return; }
      setServer({ reachable: false }); App.start();
    });
  }
  App.checkPairing = checkPairing;
  function startServer() {
    loadHealth().then(registerWorker);
    checkPairing();
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

  /* clearing the header field on the search page clears the search too, as the page's own field does */
  function clearedSearch() { if (App.current.path === '/search' && App.current.query.q) App.go('/search'); }
  function toggleClear() { var i = $('#header-search'), c = $('#header-search-clear'); if (c) c.hidden = !i.value; }
  function wireHeader() {
    var input = $('#header-search'); if (!input) return;
    var t = null;
    input.addEventListener('input', function () {
      toggleClear(); clearTimeout(t);
      t = setTimeout(function () {
        var q = input.value.trim(), target = '/search' + (q ? '?q=' + encodeURIComponent(q) : '');
        if (App.current.path === '/search') { history.replaceState(history.state, '', '#' + target); renderRoute(true); } else App.go(target);
      }, 350);   // discover.js TYPE_PAUSE_MS
    });
    input.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); clearTimeout(t); var q = input.value.trim(); App.go('/search' + (q ? '?q=' + encodeURIComponent(q) : '')); }
      if (e.key === 'Escape') { clearTimeout(t); input.value = ''; toggleClear(); input.blur(); clearedSearch(); }
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

  /* loops on one beat (docs/themes.md, Drawing cheaply). Each loop is stepped by its own CSS from its first frame; this
     only lines the steps up: as a loop starts, its delay moves by less than one step so its changes fall on whole steps
     of the page's clock. Loops at 10 and 20 changes a second then change together, and the compositor draws for the
     faster alone, not for both. A late or missing event costs a few frames, never an unpaced loop */
  var delays = new WeakMap();
  function stepMs(a) {                             // the time between a stepped loop's changes, 0 if not stepped
    var e = a.effect, t = e.getTiming(), n = 0;
    e.getKeyframes().slice(0, -1).forEach(function (k) {
      var m = /^steps\((\d+)/.exec(k.easing || ''), lin = /^linear\((.*)\)$/.exec(k.easing || '');
      if (m) n += +m[1];
      else if (lin) { var v = lin[1].split(',').map(function (x) { return x.trim().split(/\s+/)[0]; }); for (var i = 1, c = 1; i < v.length; i++) if (v[i] !== v[i - 1]) c++; n += c; }
      else n = Infinity;
    });
    return n > 0 && n < Infinity ? t.duration / n : 0;
  }
  function align(a) {
    if (!(a instanceof window.CSSAnimation) || a.effect.getTiming().iterations !== Infinity) return;
    var step = stepMs(a); if (!step) return;
    if (!delays.has(a)) delays.set(a, a.effect.getTiming().delay);
    a.ready.then(function () {
      if (a.startTime == null) return;
      var d = delays.get(a), off = ((a.startTime + d) % step + step) % step;
      a.effect.updateTiming({ delay: d - off });
    }, function () { /* cancelled before it started */ });
  }
  function alignAll() { if (window.CSSAnimation && document.getAnimations) document.getAnimations().forEach(align); }
  document.addEventListener('animationstart', function (ev) {
    if (window.CSSAnimation && ev.target.getAnimations) ev.target.getAnimations({ subtree: true }).forEach(function (a) { if (a.effect && a.effect.target === ev.target) align(a); });
  });
  App.on('ready', alignAll);                       // loops already running before this page's script
  document.addEventListener('visibilitychange', function () { if (!document.hidden) alignAll(); });   // resumed: realign

  /* decorative loops hold still (:root.is-still) while any reason asks: App.still(reason, on), each reason named by
     what asks (the listener's "pause when lull isn't the active window" below; a design or a later setting adds its own).
     The holding is CSS's, declared with each loop (animation-play-state: var(--loop-play); components.css, Keyframes),
     so a loop that starts while still is held from its first frame, its own rules (a ticker paused on hover) keep the
     say, and a design's entrances still finish (docs/themes.md, Drawing cheaply) */
  var stillFor = {};
  App.still = function (reason, on) {
    if (on) stillFor[reason] = true; else delete stillFor[reason];
    var was = root.classList.contains('is-still');
    root.classList.toggle('is-still', Object.keys(stillFor).length > 0);
    if (was && !root.classList.contains('is-still')) alignAll();   // resumed: realign
  };
  /* "Pause animations when lull isn't the active window" (Settings › Appearance, this device, off by default): the
     window losing focus, to another app or another window, in the desktop app and in a browser. Not on Android, where
     an app in sight is the active one */
  function inactive() { App.still('inactive', !!prefs.pauseInactive && !window.LullNative && !document.hasFocus()); }
  window.addEventListener('blur', inactive);
  window.addEventListener('focus', inactive);
  App.on('prefs', inactive);
  inactive();

  /* a link shared to the Android app ("Share", lull://open?link=…) waits in the shell until this page takes it: at boot,
     and when the shell nudges a running page. It opens as #/open (discover.js). */
  App.takeSharedLink = function () {
    var link = window.lullLinks && typeof window.lullLinks.take === 'function' ? window.lullLinks.take() : '';
    if (link) App.go('/open?link=' + encodeURIComponent(link));
  };

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
    App.on('store', function (k) { if (k.indexOf('privateListening') >= 0) { renderPrivate(); renderBar(); } });
    renderPrivate();
    App.on('store', function (k) { if (k.indexOf('saves') >= 0 || k.indexOf('activity') >= 0) { updateActivityBadge(); renderStrip(); if (state.ctxTab === 'activity') renderCtx(); } });
    booted = true;
    renderRoute();
    startServer();
    App.emit('ready');
    App.takeSharedLink();
  }
  /* boot once the DOM is parsed and the design's design.js (if any) has registered its slots */
  document.addEventListener('DOMContentLoaded', function () { Designs.ready(boot); });
})();
