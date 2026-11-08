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
  var MODEL = {"clock.date":["number","minute"],"clock.daypart":["state","minute",["morning","afternoon","evening","night"]],"clock.hour":["number","minute"],"clock.minute":["number","minute"],"clock.month":["number","minute"],"clock.second":["number","second"],"clock.time":["text","minute"],"clock.weekday":["state","minute",["sun","mon","tue","wed","thu","fri","sat"]],"clock.year":["number","minute"]};
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
    'clock.month': function (v, raw) { return fmt('month', { month: 'long' }).format(raw.date); }
  };
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
    if (t !== 'flag') { var w = words(p); all(bound(p)).forEach(function (el) { setText(el, w); }); }
    if (!own[p]) { var name = L.cssName(p), css = cssValue(p); all(HOSTS).forEach(function (el) { setVar(el, name, css); }); }
    if (t === 'state' || t === 'flag') {
      var a = L.attrName(p), s = t === 'flag' ? (vals[p] ? 'true' : 'false') : String(vals[p]);
      if (root.getAttribute(a) !== s) root.setAttribute(a, s);
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
  };
  L.used = function () { return Object.keys(used); };

  /* every named value on the page again: after lull draws a screen or the shell, whose hosts are new */
  L.paint = function () { if (any) Object.keys(vals).forEach(write); };
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
    if (window.App && App.on) { App.on('route', L.paint); App.on('design', L.paint); L.paint(); }
  }
  if (window.App && App.on) hook();
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', hook);
})();
