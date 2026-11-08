/* live.js: lull's live model, published for a theme (ADR-0101; docs/themes.md section 14).
   The model is web/designs/live.json: values by dotted path (clock.minute), each with a type and a pace. A design or
   theme names the values it shows, and the hub lists them in its manifest (`live`); designs.js loads this file only
   when that list isn't empty, and hands it the list (Live.use). Each value named is published in every form its type
   has, and only when it changes:
   - words in every [data-bind="<path>"] (and in a slot's {{<path>}} field as the slot is drawn);
   - a custom property --<path with dashes> on the live hosts (HOSTS: lull's own, every region, and any [data-live] a
     theme marks), registered inherits: false (base.css), so a write restyles the host alone;
   - for a state or a flag, data-<path with dashes> on :root, so any rule can follow it.
   A value nobody names is never computed. The clock is one timer to the next minute's edge (to the next second only
   when clock.second is named), stopped while the page is hidden. Runs in lull's page and in the desktop app's own
   pages (the mini player windows), without App. */
(function () {
  'use strict';
  var L = window.Live = {};
  var root = document.documentElement;

  /* web/designs/live.json, one line (path: [type, pace, states]); written from it by
     LULL_WRITE_PACKAGES=1 cargo test -p lull-themes */
  var MODEL = {"app.ctx":["state","press",["docked","open","closed"]],"app.screen":["state","press"],"app.size":["state","resize",["compact","regular","wide","xwide","ultra"]],"app.view":["state","press",["page","now"]],"clock.date":["number","minute"],"clock.daypart":["state","minute",["morning","afternoon","evening","night"]],"clock.hour":["number","minute"],"clock.minute":["number","minute"],"clock.month":["number","minute"],"clock.second":["number","second"],"clock.time":["text","minute"],"clock.weekday":["state","minute",["sun","mon","tue","wed","thu","fri","sat"]],"clock.year":["number","minute"],"next.album":["text","song"],"next.artist":["text","song"],"next.saved":["flag","song"],"next.title":["text","song"],"player.crossfading":["flag","song"],"player.muted":["flag","press"],"player.playing":["flag","press"],"player.private":["flag","press"],"player.repeat":["state","press",["off","all","one"]],"player.shuffle":["flag","press"],"player.status":["state","song",["idle","finding","queued","buffering","streaming","stalled","choose","library","failed","jellyfin-error","ended"]],"player.status-text":["text","song"],"player.volume":["number","press"],"queue.1.artist":["text","song"],"queue.1.title":["text","song"],"queue.2.artist":["text","song"],"queue.2.title":["text","song"],"queue.3.artist":["text","song"],"queue.3.title":["text","song"],"queue.4.artist":["text","song"],"queue.4.title":["text","song"],"queue.5.artist":["text","song"],"queue.5.title":["text","song"],"queue.6.artist":["text","song"],"queue.6.title":["text","song"],"queue.7.artist":["text","song"],"queue.7.title":["text","song"],"queue.8.artist":["text","song"],"queue.8.title":["text","song"],"queue.count":["number","song"],"queue.left":["text","song"],"region.height":["number","resize"],"region.width":["number","resize"],"services.cache":["state","press",["ok","warn","down"]],"services.catalog":["state","press",["ok","warn","down"]],"services.lastfm":["state","press",["ok","warn","down"]],"services.library":["state","press",["ok","warn","down"]],"services.soulseek":["state","press",["ok","warn","down"]],"song.album":["text","song"],"song.artist":["text","song"],"song.buffered":["number","tick"],"song.duration":["text","song"],"song.elapsed":["text","second"],"song.format":["text","song"],"song.from":["text","song"],"song.hue":["number","song"],"song.liked":["flag","press"],"song.lossless":["flag","song"],"song.pos":["number","tick"],"song.quality":["text","song"],"song.received":["number","tick"],"song.saved":["flag","press"],"song.seed":["number","song"],"song.source":["state","song",["none","library","device","soulseek","youtube"]],"song.title":["text","song"],"song.title-length":["number","song"],"song.year":["text","song"],"week.artists":["number","song"],"week.peak":["number","song"],"week.plays":["number","song"],"week.time":["text","song"]};
  L.model = MODEL;

  /* the parts a value is drawn on: lull's own hosts (as --song-pos has them), every region, any element a theme marks */
  var HOSTS = '#pbar, #mini, .sky-specks, .now__dev, .now__devc, .disc-lead, [data-dk-player], [data-region], [data-live]';
  L.hosts = HOSTS;
  function dashed(p) { return p.replace(/\./g, '-'); }
  L.cssName = function (p) { return '--' + dashed(p); };
  L.attrName = function (p) { return ['data', dashed(p)].join('-'); };   // only the model's: data-clock-daypart
  /* a path of the model's, or a dotted path in one of its branches (which the hub refuses in a slot unless listed) */
  L.known = function (p) { return Object.prototype.hasOwnProperty.call(MODEL, p); };

  /* ---------- what is named, and its values now ---------- */
  var used = {}, any = false, vals = {}, own = {};
  function cssString(t) { return '"' + String(t).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\A ') + '"'; }
  function pad(n) { return (n < 10 ? '0' : '') + n; }
  /* words a value shows (data-bind, {{field}}); a value's own words where its number or state isn't them */
  var WORDS = {
    'clock.hour': pad, 'clock.minute': pad, 'clock.second': pad,
    'clock.weekday': function (v, raw) { return fmt('weekday', { weekday: 'long' }).format(raw.date); },
    'clock.month': function (v, raw) { return fmt('month', { month: 'long' }).format(raw.date); },
    'song.pos': pct, 'song.buffered': pct, 'song.received': pct, 'player.volume': pct
  };
  function pct(v) { return Math.round(v * 100) + '%'; }
  var fmts = {};
  function fmt(k, o) { return fmts[k] || (fmts[k] = new Intl.DateTimeFormat(undefined, o)); }
  function words(p) {
    var v = vals[p]; if (v == null) return '';
    var t = MODEL[p][0];
    if (WORDS[p]) return String(WORDS[p](v, raws));
    if (t === 'flag') return v ? 'true' : 'false';
    return String(v);
  }
  L.text = function (p) { return L.known(p) ? (vals[p] == null ? compute(p) : words(p)) : ''; };
  function cssValue(p) {
    var v = vals[p], t = MODEL[p][0];
    if (t === 'number') return String(v);
    if (t === 'flag') return v ? '1' : '0';
    return cssString(words(p));
  }

  /* ---------- writes: each skips an unchanged value (a rewrite still costs a restyle) ---------- */
  function setText(el, v) {
    var n = el.firstChild;
    if (n && n.nodeType === 3 && !n.nextSibling) { if (n.data !== v) n.data = v; }
    else if (el.textContent !== v) el.textContent = v;
  }
  function setVar(el, name, v) { if (el.style.getPropertyValue(name) !== v) el.style.setProperty(name, v); }
  /* lookups over the whole page, kept until its structure or a hook attribute changes (app.js's $$kept does the same) */
  var kept = {}, watch = new MutationObserver(function () { kept = {}; });
  watch.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'id', 'data-bind', 'data-live', 'data-region', 'data-dk-player'] });
  function all(sel) {
    if (watch.takeRecords().length) kept = {};
    return kept[sel] || (kept[sel] = Array.prototype.slice.call(document.querySelectorAll(sel)));
  }
  function bound(p) { return '[data-bind="' + p + '"]'; }
  /* one value's forms, on the page as it is */
  function write(p) {
    var t = MODEL[p][0];
    if (p.indexOf('region.') === 0) return;      // each region's own size, written by sizes() on that region alone
    if (t !== 'flag') { var w = words(p); all(bound(p)).forEach(function (el) { setText(el, w); }); }
    if (!own[p]) { var name = L.cssName(p), css = cssValue(p); all(HOSTS).forEach(function (el) { setVar(el, name, css); }); }
    if (t === 'state' || t === 'flag') {
      var a = L.attrName(p), s = t === 'flag' ? (vals[p] ? 'true' : 'false') : String(vals[p]);
      if (!s) root.removeAttribute(a);           // a state with nothing to say (no screen yet) is no attribute
      else if (root.getAttribute(a) !== s) root.setAttribute(a, s);
    }
  }
  /* a value is set: written only when named and changed */
  function set(p, v) {
    if (!used[p] || vals[p] === v) return;
    vals[p] = v;
    write(p);
  }

  /* ---------- the clock ---------- */
  var raws = {}, timer = 0, clockOn = false, seconds = false;
  function daypart(h) { return h >= 5 && h < 12 ? 'morning' : h >= 12 && h < 18 ? 'afternoon' : h >= 18 && h < 22 ? 'evening' : 'night'; }
  var DAYS = MODEL['clock.weekday'][2];
  var CLOCK = {
    'clock.time': function (d) { return fmt('time', { hour: 'numeric', minute: '2-digit' }).format(d); },
    'clock.hour': function (d) { return d.getHours(); },
    'clock.minute': function (d) { return d.getMinutes(); },
    'clock.second': function (d) { return d.getSeconds(); },
    'clock.weekday': function (d) { return DAYS[d.getDay()]; },
    'clock.date': function (d) { return d.getDate(); },
    'clock.month': function (d) { return d.getMonth() + 1; },
    'clock.year': function (d) { return d.getFullYear(); },
    'clock.daypart': function (d) { return daypart(d.getHours()); }
  };
  /* a value as it is now, before it was ever set (a slot drawn before the first publish) */
  function compute(p) {
    if (!CLOCK[p]) return '';
    var d = new Date(); raws.date = d; vals[p] = CLOCK[p](d);
    var w = words(p);
    if (!used[p]) delete vals[p];               // only a named value is kept, and written
    return w;
  }
  function tick() {
    var d = new Date(); raws.date = d;
    Object.keys(CLOCK).forEach(function (p) { if (used[p]) set(p, CLOCK[p](d)); });
    arm(d);
  }
  /* the next edge: of the minute, or of the second when clock.second is named; none while hidden */
  function arm(d) {
    clearTimeout(timer); timer = 0;
    if (!clockOn || document.hidden) return;
    d = d || new Date();
    var ms = seconds ? 1000 - d.getMilliseconds() : (60 - d.getSeconds()) * 1000 - d.getMilliseconds();
    timer = setTimeout(tick, ms + 20);
  }
  document.addEventListener('visibilitychange', function () {
    if (document.hidden) { clearTimeout(timer); timer = 0; } else if (clockOn) tick();   // shown again: catch up once
  });

  /* a value's custom property doesn't inherit (as --song-pos, components.css): written on a host, it restyles the host
     alone, never the page under it; a part inside draws with it through `inherit`. Registered here, for the names in
     use, so a page whose design names none carries none of it */
  var registered = {};
  function register(p) {
    if (registered[p] || !window.CSS || !CSS.registerProperty) return;
    registered[p] = true;
    var t = MODEL[p][0], num = t === 'number' || t === 'flag';
    try { CSS.registerProperty(num ? { name: L.cssName(p), syntax: '<number>', inherits: false, initialValue: '0' } : { name: L.cssName(p), syntax: '*', inherits: false }); }
    catch (e) { /* registered already (a page that loaded live.js twice) */ }
  }

  /* ---------- lull's page: the song, the queue, the player, the app, the services, the week ----------
     Each read from what the page already holds (App), at the events that change it: a song, a press, a state (App's
     'player', 'like', 'save', 'store'), the song's place at App's 'tick' (written only when a hundredth or a second
     moved), the app's attributes as .app changes them. Only the branches a theme names are read at all. */
  function named(prefix) { for (var p in used) if (p.indexOf(prefix) === 0) return true; return false; }
  function fnv(s) { var h = 2166136261; for (var i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); } return h >>> 0; }
  function hundredths(f) { return Math.floor(Math.max(0, Math.min(1, f || 0)) * 100) / 100; }
  function songValues(o) {
    var P = App.player, t = P.current(), M = App.data;
    var copy = t ? P.playingCopy() : null, f = copy && copy.format ? (M.formats[copy.format] || { label: copy.format }) : null;
    var album = t && M.album ? M.album(t.albumId) : null, from = t ? P.playingFrom() : null, h = t ? fnv(String(t.id)) : 0;
    o['song.title'] = t ? t.title : ''; o['song.artist'] = t ? t.artist : ''; o['song.album'] = t ? t.album || '' : '';
    o['song.year'] = album && album.year ? String(album.year) : '';
    o['song.from'] = from ? App.placeName(from) : '';
    o['song.source'] = copy ? copy.where : 'none';
    o['song.quality'] = f ? f.label : ''; o['song.format'] = t ? App.fmt.formatLine(t) : ''; o['song.lossless'] = !!(f && f.lossless);
    o['song.liked'] = !!t && App.isLiked(t.id); o['song.saved'] = !!t && App.isSaved(t.id);
    o['song.duration'] = t ? App.fmt.time(P.duration() || 0) : '';
    o['song.hue'] = h % 360; o['song.seed'] = (h >>> 9) % 1000 / 1000; o['song.title-length'] = t ? t.title.length : 0;
  }
  function placeValues(o) {
    var P = App.player, ps = P.state, t = P.current(), dur = P.duration() || 0, b = ps.buffered || [];
    o['song.pos'] = t && dur ? hundredths(ps.pos / dur) : 0;
    o['song.elapsed'] = t ? App.fmt.time(ps.pos) : '';
    o['song.buffered'] = t && b.length ? hundredths(b[b.length - 1][1]) : 0;
    o['song.received'] = t ? hundredths(ps.received == null ? 1 : ps.received) : 0;
  }
  function queueValues(o) {
    var P = App.player, ps = P.state, q = P.upNext(8), n = q[0], left = (ps.queue || []).slice((ps.index || 0) + 1), secs = 0;
    o['next.title'] = n ? n.title : ''; o['next.artist'] = n ? n.artist : ''; o['next.album'] = n ? n.album || '' : '';
    o['next.saved'] = !!n && App.isSaved(n.id);
    for (var i = 1; i <= 8; i++) { o['queue.' + i + '.title'] = q[i - 1] ? q[i - 1].title : ''; o['queue.' + i + '.artist'] = q[i - 1] ? q[i - 1].artist : ''; }
    left.forEach(function (x) { var s = App.data.track(x.trackId); secs += s && s.duration || 0; });
    o['queue.count'] = left.length; o['queue.left'] = left.length ? App.fmt.length(secs) : '';
  }
  function playerValues(o) {
    var P = App.player, ps = P.state, idle = !P.current() || ps.status === 'idle';
    o['player.status'] = idle ? 'idle' : ps.status; o['player.status-text'] = idle ? '' : P.stateWords() || '';
    o['player.playing'] = !!ps.playing; o['player.shuffle'] = !!ps.shuffle; o['player.repeat'] = ps.repeat || 'off';
    o['player.volume'] = Math.round((ps.volume == null ? 1 : ps.volume) * 100) / 100; o['player.muted'] = !!ps.muted;
    o['player.crossfading'] = !!ps.crossfade; o['player.private'] = !!App.store.get('privateListening');
  }
  function appValues(o) {
    var app = document.getElementById('app'); if (!app) return;
    /* the attribute names whole, so the hook check (designs/hooks.js) never reads a data-* prefix lull builds names from */
    [['screen', 'data-screen'], ['view', 'data-view'], ['ctx', 'data-ctx'], ['size', 'data-size']].forEach(function (k) { o['app.' + k[0]] = app.getAttribute(k[1]) || ''; });
  }
  function serviceValues(o) {
    var s = App.store.get('services') || {};
    ['soulseek', 'library', 'lastfm', 'catalog', 'cache'].forEach(function (k) { o['services.' + k] = s[k] ? s[k].state : 'ok'; });
  }
  function weekValues(o) {
    var w = App.data.stats && App.data.stats.byPeriod && App.data.stats.byPeriod['7d'], ok = !!(w && w.clock && w.clock.length);
    o['week.time'] = ok ? Math.floor(w.minutes / 60) + ' h ' + w.minutes % 60 + ' min' : '';
    o['week.plays'] = ok ? w.plays : 0; o['week.artists'] = ok ? w.artists : 0;
    o['week.peak'] = ok ? w.clock.indexOf(Math.max.apply(null, w.clock)) : 0;
  }
  var READ = [['song.', songValues], ['next.', queueValues], ['queue.', queueValues], ['player.', playerValues], ['app.', appValues],
    ['services.', serviceValues], ['week.', weekValues]];
  /* the named branches among `which` (all when absent), read and set */
  function fromApp(which) {
    if (!window.App || !App.player) return;
    if (which && which.indexOf('next.') >= 0) which = which.concat('queue.');   // one read for both
    var o = {}, done = [];
    READ.forEach(function (r) {
      if ((which && which.indexOf(r[0]) < 0) || done.indexOf(r[1]) >= 0 || !named(r[0])) return;
      done.push(r[1]); r[1](o);
    });
    if (!which || which.indexOf('song.') >= 0) placeValues(o);
    Object.keys(o).forEach(function (p) { set(p, o[p]); });
  }
  /* the named values but the clock's and the regions', for a window that can't read them itself: the desktop app's mini
     player windows get them with each player snapshot (desktop.js), and set them with feed() */
  L.values = function () {
    var o = {};
    Object.keys(used).forEach(function (p) { if (vals[p] != null && p.indexOf('clock.') && p.indexOf('region.')) o[p] = vals[p]; });
    return o;
  };
  L.feed = function (o) { Object.keys(o || {}).forEach(function (p) { if (L.known(p)) set(p, o[p]); }); };

  /* each region's own size, written on it alone, when it resizes (only while a theme names region.width or .height) */
  var sizer = null;
  function sizes() {
    if (!named('region.') || !window.ResizeObserver) { if (sizer) { sizer.disconnect(); sizer = null; } return; }
    if (!sizer) sizer = new ResizeObserver(function (entries) {
      entries.forEach(function (e) {
        var b = e.borderBoxSize && e.borderBoxSize[0], w = Math.round(b ? b.inlineSize : e.contentRect.width), h = Math.round(b ? b.blockSize : e.contentRect.height);
        if (used['region.width'] && !own['region.width']) setVar(e.target, '--region-width', String(w));
        if (used['region.height'] && !own['region.height']) setVar(e.target, '--region-height', String(h));
      });
    });
    all('[data-region]').forEach(function (el) { sizer.observe(el); });
  }

  /* ---------- what designs.js asks for ---------- */
  /* use(names): the values the design or theme in use names (its manifest's `live`, with what it extends); `owns`: the
     custom properties it sets itself, which stay its own (the hub leaves them out of `live` already) */
  L.use = function (names, owns) {
    var was = used;
    used = {}; own = {}; vals = {};
    (names || []).forEach(function (p) { if (L.known(p)) { used[p] = true; register(p); } });
    (owns || []).forEach(function (p) { own[p] = true; });
    any = Object.keys(used).length > 0;
    /* a state or flag no longer named leaves :root */
    Object.keys(was).forEach(function (p) {
      var t = MODEL[p][0];
      if (!used[p] && (t === 'state' || t === 'flag')) root.removeAttribute(L.attrName(p));
    });
    clockOn = Object.keys(CLOCK).some(function (p) { return used[p]; });
    seconds = !!used['clock.second'];
    if (clockOn) tick(); else { clearTimeout(timer); timer = 0; }
    fromApp(); sizes();
  };
  L.used = function () { return Object.keys(used); };

  /* every named value on the page again: after lull draws a screen or the shell, whose hosts are new */
  L.paint = function () { if (any) { Object.keys(vals).forEach(write); sizes(); } };
  /* a slot's markup as it is drawn (designs.js renderSlot), before it is on the page: its bindings' words and its
     hosts' properties, so it never shows empty until the next change */
  L.fill = function (box) {
    if (!any) return;
    Object.keys(used).forEach(function (p) { if (vals[p] == null) compute(p); });
    Array.prototype.forEach.call(box.querySelectorAll('[data-bind]'), function (el) {
      var p = el.getAttribute('data-bind');
      if (used[p] && MODEL[p][0] !== 'flag') setText(el, words(p));
    });
    var hosts = box.querySelectorAll(HOSTS);
    Object.keys(vals).forEach(function (p) {
      if (own[p]) return;
      var name = L.cssName(p), css = cssValue(p);
      Array.prototype.forEach.call(hosts, function (el) { setVar(el, name, css); });
    });
  };
  /* lull's page draws its screens and shell again on these (none in the desktop app's own pages). A theme's live.js
     can load before app.js has run (it loads with the theme, in the page's head): then it hooks in once the page has */
  function hook() {
    if (!window.App || !App.on) return;
    App.on('route', function () { fromApp(['app.']); L.paint(); });
    App.on('design', L.paint);
    /* the player's kinds of change (app.js changed()): the queue is read again only when it or the song changed */
    App.on('player', function (k) {
      if (!any) return;
      fromApp(k === 'track' || k === 'queue' || k === 'ended' ? ['song.', 'next.', 'player.'] : k === 'source' || k === 'status' ? ['song.', 'player.'] : ['player.']);
    });
    App.on('tick', function () { if (any && (used['song.pos'] || used['song.elapsed'] || used['song.buffered'] || used['song.received'])) { var o = {}; placeValues(o); Object.keys(o).forEach(function (p) { set(p, o[p]); }); } });
    App.on('like', function () { if (any) fromApp(['song.']); });
    App.on('save', function () { if (any) fromApp(['song.', 'next.']); });
    App.on('store', function () { if (any) fromApp(['song.', 'player.', 'services.', 'week.']); });
    /* the app's attributes change with a route, a panel, a resize: follow .app itself, only while a theme names them */
    var app = document.getElementById('app');
    if (app) new MutationObserver(function () { if (named('app.')) fromApp(['app.']); })
      .observe(app, { attributes: true, attributeFilter: ['data-screen', 'data-view', 'data-ctx', 'data-size'] });
    fromApp(); L.paint();
  }
  if (window.App && App.on) hook();
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', hook);
})();
