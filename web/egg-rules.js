/* The easter eggs' conditions (ADR-0081, docs/themes.md section 13): when each declared condition holds, given what the
   page knows at that moment. Pure functions over plain values, so node can test them (tests/web/egg-rules.test.mjs);
   web/eggs.js evaluates them on the page's own events, never by polling. */
(function (root) {
  'use strict';
  var E = {};

  var DAYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
  /* scripts written without spaces between words: a word there is found inside the text */
  var UNSPACED = /[฀-໿က-႟ក-៿぀-ヿ㐀-䶿一-鿿豈-﫿]/;

  /* text compared the same way on both sides: case and accents set aside (in every script, identically) */
  E.fold = function (text) {
    return String(text || '').normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
  };
  /* the first of `words` (any language or script) found in `text`, as declared, or null */
  E.findWord = function (words, text) {
    var t = E.fold(text);
    if (!t) return null;
    for (var i = 0; i < words.length; i++) {
      var w = E.fold(words[i]);
      if (!w) continue;
      if (UNSPACED.test(w)) { if (t.indexOf(w) >= 0) return words[i]; continue; }
      var at = -1;
      while ((at = t.indexOf(w, at + 1)) >= 0) {
        if (!isWordChar(t, at - 1) && !isWordChar(t, at + w.length)) return words[i];
      }
    }
    return null;
  };
  function isWordChar(t, i) { return i >= 0 && i < t.length && /[\p{L}\p{N}]/u.test(t.charAt(i)); }

  /* ---------- clock ---------- */
  /* "HH:MM[:SS]" (or "24:00") to seconds since midnight */
  E.seconds = function (hms) {
    var p = String(hms).split(':').map(Number);
    return p[0] * 3600 + (p[1] || 0) * 60 + (p[2] || 0);
  };
  function daySeconds(d) { return d.getHours() * 3600 + d.getMinutes() * 60 + d.getSeconds(); }
  function mmdd(d) { return pad(d.getMonth() + 1) + '-' + pad(d.getDate()); }
  function pad(n) { return (n < 10 ? '0' : '') + n; }
  function list(v) { return v == null ? null : Array.isArray(v) ? v : [v]; }
  /* Does a clock condition hold at `now` (a Date, the device's time)? { on, count } where count is the seconds left
     until `to` (a countdown). A window that wraps past midnight belongs to the date it starts on. */
  E.clock = function (c, now) {
    var from = c.from != null ? E.seconds(c.from) : 0, to = c.to != null ? E.seconds(c.to) : 86400, s = daySeconds(now);
    var wraps = to <= from, inside = wraps ? s >= from || s < to : s >= from && s < to;
    if (!inside) return { on: false };
    var start = new Date(now.getTime());
    if (wraps && s < to) start.setDate(start.getDate() - 1);          // after midnight: the window began yesterday
    var dates = list(c.date), days = list(c.weekday);
    if (dates && dates.indexOf(mmdd(start)) < 0) return { on: false };
    if (days && days.indexOf(DAYS[start.getDay()]) < 0) return { on: false };
    var left = (wraps && s >= from ? to + 86400 : to) - s;
    return { on: true, count: c.countdown ? left : null };
  };
  /* ms from `now` until any of these clock conditions can change: its next start or end, or the next second while a
     countdown runs. The page sets one timer for this, never a poll. */
  E.nextClockChange = function (conds, now) {
    var s = daySeconds(now) + now.getMilliseconds() / 1000, best = Infinity;
    conds.forEach(function (c) {
      if (c.countdown && E.clock(c, now).on) best = Math.min(best, 1 - now.getMilliseconds() / 1000);
      [c.from != null ? E.seconds(c.from) : 0, c.to != null ? E.seconds(c.to) : 86400].forEach(function (edge) {
        var d = edge - s;
        if (d <= 0) d += 86400;
        best = Math.min(best, d);
      });
    });
    return best === Infinity ? null : Math.ceil(best * 1000);
  };

  /* ---------- once ---------- */
  /* the period an egg fired in: it fires again only in another one. `song` is the current queue entry's id. */
  E.period = function (once, now, song, session) {
    switch (once || 'song') {
      case 'always': return null;
      case 'song': return 'song:' + song;
      case 'session': return 'session:' + session;
      case 'day': return 'day:' + now.getFullYear() + '-' + mmdd(now);
      case 'night': {
        // 18:00 to 06:00 is one night, named by the evening it starts on
        var d = new Date(now.getTime());
        if (d.getHours() < 6) d.setDate(d.getDate() - 1);
        return 'night:' + d.getFullYear() + '-' + mmdd(d);
      }
      case 'week': {
        var w = new Date(now.getFullYear(), now.getMonth(), now.getDate() - ((now.getDay() + 6) % 7));
        return 'week:' + w.getFullYear() + '-' + mmdd(w);
      }
      case 'year': return 'year:' + now.getFullYear();
      default: return 'song:' + song;
    }
  };

  /* ---------- the rest ---------- */
  /* the milestone reached as a count goes from `before` to `after`, or null */
  E.milestone = function (at, before, after) {
    var hit = null;
    list(at).forEach(function (n) { if (before < n && after >= n) hit = n; });
    return hit;
  };

  /* Do an egg's conditions hold? `s` is what the page knows now:
       now (Date), track ({ title, artist, album }), repeat (times the current song played through in a row),
       status, playing (sound coming out), screen, idleS (seconds since the last input),
       moment ({ kind: 'gesture'|'milestone'|'event', on, count }) for a moment's conditions, roll (0-1, for chance)
     `held` is not checked here: the page times it once the rest hold. Returns { on, count, word } . */
  E.holds = function (egg, s) {
    var out = { on: true, count: null, word: null };
    for (var i = 0; i < egg.when.length && out.on; i++) {
      var c = egg.when[i], kind = Object.keys(c)[0], p = c[kind];
      switch (kind) {
        case 'clock': {
          var k = E.clock(p, s.now);
          if (!k.on) out.on = false; else if (k.count != null) out.count = k.count;
          break;
        }
        case 'words': {
          var w = null;
          if (s.track) list(p.in).forEach(function (f) { if (!w) w = E.findWord(list(p.any), s.track[f]); });
          if (!w) out.on = false; else out.word = w;
          break;
        }
        case 'repeat': if (!(s.repeat >= p.times)) out.on = false; break;
        case 'gesture':
          if (!s.moment || s.moment.kind !== 'gesture' || s.moment.on !== p.on || s.moment.taps < p.taps) out.on = false;
          break;
        case 'milestone': {
          if (!s.moment || s.moment.kind !== 'milestone' || s.moment.on !== p.of) { out.on = false; break; }
          var hit = E.milestone(p.at, s.moment.before, s.moment.after);
          if (hit == null) out.on = false; else out.count = hit;
          break;
        }
        case 'event': if (!s.moment || s.moment.kind !== 'event' || s.moment.on !== p.on) out.on = false; break;
        case 'player':
          if (p.status != null && list(p.status).indexOf(s.status) < 0) out.on = false;
          else if (p.playing != null && !!s.playing !== p.playing) out.on = false;
          else if (p.screen != null && list(p.screen).indexOf(s.screen) < 0) out.on = false;
          else if (p.idle != null && !(s.idleS >= p.idle)) out.on = false;
          break;
        case 'chance': if (!(s.roll < 1 / p.one_in)) out.on = false; break;
        default: out.on = false;      // a condition this page doesn't know: the hub refuses it, so never here
      }
    }
    return out;
  };
  /* the egg's `held` (seconds its `player` conditions must have held), or 0 */
  E.held = function (egg) {
    var h = 0;
    egg.when.forEach(function (c) { if (c.player && c.player.held) h = Math.max(h, c.player.held); });
    return h;
  };
  /* is the egg one of a moment (a tap, a milestone, an event, a song played through), not a state that holds? */
  E.moment = function (egg) {
    return egg.when.some(function (c) { return c.gesture || c.milestone || c.event || c.repeat; });
  };
  /* the egg's words with its fields put in: {{count}} and {{word}} */
  E.line = function (line, count, word) {
    return String(line || '').replace(/\{\{count\}\}/g, count == null ? '' : String(count)).replace(/\{\{word\}\}/g, word || '');
  };

  root.LullEggRules = E;
  if (typeof module === 'object' && module.exports) module.exports = E;
})(typeof self !== 'undefined' ? self : this);
