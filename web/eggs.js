/* Easter eggs (ADR-0081, docs/themes.md section 13): the eggs the packs that are on declare, checked on what already
   happens (a song or status change, a like, a tap on the logo, the cover or the title, a moment from sounds.js) and on
   one timer for the next clock edge, never by polling. A live egg is :root[data-egg~="<name>"] (with --egg-count and
   data-egg-word), its sound, its slot and its line in .egg-stage, or a place it opens. Rare and never in the way: each
   fires once per its `once`, never over a dialog or sheet, and ends at the next input. Loaded once the page is idle
   (sounds.js, App.needEggs); App.eggs is the API. */
(function () {
  'use strict';
  var App = window.App, E = window.LullEggRules, S = window.LullSoundRules, P = App.player, ps = P.state, D = window.Designs;
  var root = document.documentElement, $ = function (s) { return document.querySelector(s); };
  var GESTURES = { brand: '[data-brand]', cover: '.cover-zoom, [data-now="c-art"]', title: '.pbar__title, [data-now="title"]' };

  var eggs = {};                 // name -> declaration, from the packs that are on
  var live = {};                 // name -> { timer, count, word, egg }
  var holding = {};              // name -> ms since its conditions (all but `held`) began to hold, for state eggs
  var insteadFor = {};           // sound event -> the egg that takes its sound's place this time
  var lastInput = performance.now(), idleTimer = 0, clockTimer = 0, heldTimers = {};
  var session = Math.random().toString(36).slice(2);
  var repeat = { trackId: null, times: 0 }, taps = {};
  var fired = (function () { try { return JSON.parse(localStorage.getItem('lull:eggs')) || {}; } catch (e) { return {}; } })();

  function refresh() {
    var st = App.sounds.settings();
    eggs = st.on || st.surprises ? S.eggs(App.sounds.stack(), st) : {};
    if (!st.surprises) eggs = {};
    Object.keys(live).forEach(function (n) { if (!eggs[n]) end(n); });
    check();
  }

  /* ---------- what the page knows now ---------- */
  function screen() { var a = $('#app'); return a ? a.getAttribute('data-screen') : null; }
  function state(moment) {
    return {
      now: new Date(), track: P.current(), repeat: repeat.times, status: ps.status, playing: P.heard(), screen: screen(),
      idleS: (performance.now() - lastInput) / 1000, moment: moment || null, roll: Math.random()
    };
  }
  /* is anything open that an egg must not get in the way of? */
  function busy() { return !!document.querySelector('.dialog:not([hidden]), .sheet:not([hidden])') || root.getAttribute('data-classic') === 'on'; }

  /* ---------- checking ---------- */
  /* the state eggs: those that hold start (after `held`), those that stop holding end (after `for`) */
  function check() {
    var s = state(null), t = performance.now();
    Object.keys(eggs).forEach(function (name) {
      var egg = eggs[name];
      if (E.moment(egg)) return;
      var h = E.holds(egg, s);
      if (!h.on) {
        delete holding[name]; clearTimeout(heldTimers[name]); delete heldTimers[name];
        if (live[name] && !live[name].moment) endSoon(name);
        return;
      }
      if (live[name]) { show(name, h); return; }
      var held = E.held(egg) * 1000;
      if (holding[name] == null) holding[name] = t;
      var wait = held - (t - holding[name]);
      if (wait > 0) {
        if (!heldTimers[name]) heldTimers[name] = setTimeout(function () { delete heldTimers[name]; check(); }, wait);
        return;
      }
      fire(name, h, false);
    });
    scheduleClock();
    scheduleIdle();
  }
  /* a moment: every egg of that moment whose conditions hold now fires */
  function moment(m) {
    var s = state(m);
    Object.keys(eggs).forEach(function (name) {
      var egg = eggs[name];
      if (!E.moment(egg)) return;
      if (m.kind === 'gesture') s.moment = { kind: 'gesture', on: m.on, taps: tapsWithin(m.on, gestureWithin(egg)) };
      var h = E.holds(egg, s);
      if (h.on) fire(name, h, true);
    });
  }
  function gestureWithin(egg) {
    var g = egg.when.filter(function (c) { return c.gesture; })[0];
    return g ? g.gesture.within : 0;
  }
  function tapsWithin(on, sec) {
    var t = performance.now();
    return (taps[on] || []).filter(function (x) { return t - x <= sec * 1000; }).length;
  }

  /* ---------- firing ---------- */
  function fire(name, h, isMoment) {
    var egg = eggs[name], period = E.period(egg.once, new Date(), P.currentId(), session);
    if (period && fired[name] === period) return false;
    if (busy()) return false;
    if (period) { fired[name] = period; try { localStorage.setItem('lull:eggs', JSON.stringify(fired)); } catch (e) { /* private mode */ } }
    live[name] = { moment: isMoment, egg: egg, count: h.count, word: h.word };
    show(name, h);
    if (egg.go) App.go(egg.go.replace(/^#/, ''));
    if (egg.sound) {
      var entry = S.normal(egg.sound);
      if (entry.loop) App.sounds.loopInstead(name);
      else if (egg.instead) { var ev = eventOf(egg); if (ev) insteadFor[ev] = name; }
      else App.sounds.egg(name);
    }
    slot(name, h);
    if (isMoment) endSoon(name);
    App.emit('egg', name);
    return true;
  }
  function eventOf(egg) { var c = egg.when.filter(function (x) { return x.event; })[0]; return c ? c.event.on : null; }
  /* the egg's look: its name on the root, with its count and word */
  function show(name, h) {
    var names = Object.keys(live).join(' ');
    App.sounds.cssFor();
    if (root.getAttribute('data-egg') !== names) root.setAttribute('data-egg', names);
    if (h.count != null) { root.style.setProperty('--egg-count', String(h.count)); live[name].count = h.count; }
    if (h.word) root.setAttribute('data-egg-word', h.word);
    line(name);
  }
  function endSoon(name) {
    var l = live[name]; if (!l) return;
    var sec = l.egg['for'] != null ? l.egg['for'] : App.sounds.vocab().limits.forDefault;
    clearTimeout(l.timer);
    l.timer = setTimeout(function () { end(name); }, sec * 1000);
  }
  function end(name) {
    var l = live[name]; if (!l) return;
    clearTimeout(l.timer); delete live[name];
    var names = Object.keys(live);
    if (names.length) root.setAttribute('data-egg', names.join(' ')); else root.removeAttribute('data-egg');
    if (!names.some(function (n) { return live[n].count != null; })) root.style.removeProperty('--egg-count');
    if (!names.some(function (n) { return live[n].word; })) root.removeAttribute('data-egg-word');
    if (App.sounds && l.egg.sound && S.normal(l.egg.sound).loop) App.sounds.loopInstead(null);
    var s = document.querySelector('.egg-stage [data-egg-of="' + name + '"]'); if (s) s.remove();
    line(null);
    App.emit('egg', null);
  }
  function endAll() { Object.keys(live).forEach(end); }

  /* ---------- the stage: its line of words and the eggs' slots, above the page, never under the pointer ---------- */
  /* its look, in the components layer as if it were components.css's: it comes with this file, as only a live egg
     needs it, so no page pays for it at start. A design's pack or a theme restyles it as any hook (.egg-stage,
     .egg-line); a pack's slot boxes ([data-egg-of]) are placed by the pack's own CSS. */
  var STAGE_CSS = '@layer components {' +
    '.egg-stage { position: fixed; inset: 0; z-index: var(--z-toast); pointer-events: none; }' +
    '.egg-line { position: absolute; left: 50%; bottom: calc(var(--bar-h) + 40px); transform: translateX(-50%); max-width: calc(100vw - 32px);' +
    ' margin: 0; padding: 8px 16px; border: var(--stroke) solid var(--line); border-radius: var(--r-pill);' +
    ' background: var(--panel); color: var(--fg-panel); box-shadow: var(--stack-sm); font: 700 .9375rem/1.35 var(--f-ui); }' +
    '.egg-line[hidden] { display: none; } }';
  function stage() {
    var el = $('.egg-stage');
    if (!el) {
      if (!document.getElementById('egg-css')) {
        var css = document.createElement('style'); css.id = 'egg-css'; css.textContent = STAGE_CSS; document.head.appendChild(css);
      }
      el = document.createElement('div'); el.className = 'egg-stage'; el.setAttribute('aria-hidden', 'false');
      el.innerHTML = '<p class="egg-line" role="status" aria-live="polite"></p>';
      document.body.appendChild(el);
    }
    return el;
  }
  function line() {
    var names = Object.keys(live).filter(function (n) { return live[n].egg.line; });
    var n = names[names.length - 1], words = n ? E.line(live[n].egg.line, live[n].count, live[n].word) : '';
    var el = $('.egg-stage');
    if (!words && !el) return;
    var p = stage().querySelector('.egg-line');
    if (p.textContent !== words) p.textContent = words;
    p.hidden = !words;
    tidy();
  }
  function slot(name, h) {
    var egg = live[name] && live[name].egg, pack = egg && App.sounds.known()[egg.pack];
    if (!pack || pack.slots.indexOf(name) < 0) return;
    var url = pack.source === 'theme' ? '/api/themes/' + encodeURIComponent(pack.theme) + '/slots/egg.' + name + '.html'
      : pack.source === 'built-in' ? new URL('packs/' + pack.id + '/slots/egg.' + name + '.html', document.baseURI).href   // a static file of the UI
      : '/api/packs/' + encodeURIComponent(pack.id) + '/slots/egg.' + name + '.html';
    var t = P.current(), d = new Date();
    fetch(url, { credentials: 'same-origin' }).then(function (r) { return r.ok ? r.text() : ''; }).then(function (tpl) {
      return tpl ? eggSlot(tpl, { count: h.count == null ? '' : h.count, word: h.word || '', title: t ? t.title : '', artist: t ? t.artist : '', hour: String(d.getHours()) }) : '';
    }).then(function (markup) {
      if (!markup || !live[name]) return;
      var box = document.createElement('div'); box.setAttribute('data-egg-of', name); box.innerHTML = markup;
      stage().appendChild(box);
    }, function (err) { console.warn('An easter egg\'s slot didn\'t load', name, err); });
  }
  /* an egg's slot (`slots/egg.<name>.html`): fields only ({{count}}, {{word}}, {{title}}, {{artist}}, {{hour}},
     escaped), sanitized as any listener's slot is (designs.js), and of the parts only `icon`. Resolves to markup. */
  function eggSlot(tpl, fields) {
    return D.purify().then(function () {
      var f = Object.assign({ product: D.product }, fields), esc = function (v) { return App.esc(v == null ? '' : v); };
      var filled = String(tpl).replace(/<!--[\s\S]*?-->/g, '').replace(/>\s*\n\s*</g, '><').trim()
        .replace(/\{\{\s*([a-zA-Z][a-zA-Z0-9]*)\s*\}\}/g, function (all, k) { return Object.prototype.hasOwnProperty.call(f, k) ? esc(f[k]) : ''; });
      var body = D.sanitize(filled), parts = body.querySelectorAll('lull-part');
      for (var i = 0; i < parts.length; i++) {
        var el = parts[i], cls = el.getAttribute('cls'), out = '';
        if (el.getAttribute('name') === 'icon') {
          out = String(App.icon(el.getAttribute('icon') || 'sparkle', null, el.getAttribute('size') === '16' ? 'ic-16' : ''));
          if (cls) out = out.replace(/^(<[a-z]+[^>]*?\sclass=")/, '$1' + esc(cls) + ' ');
        }
        el.outerHTML = out;
      }
      return body.innerHTML;
    });
  }
  function tidy() {
    var el = $('.egg-stage');
    if (el && !Object.keys(live).length) el.remove();
  }

  /* ---------- timers: the next clock edge, and being left alone ---------- */
  function scheduleClock() {
    clearTimeout(clockTimer);
    var conds = [];
    Object.keys(eggs).forEach(function (n) { eggs[n].when.forEach(function (c) { if (c.clock) conds.push(c.clock); }); });
    var ms = conds.length ? E.nextClockChange(conds, new Date()) : null;
    if (ms != null) clockTimer = setTimeout(check, ms + 20);
  }
  function scheduleIdle() {
    clearTimeout(idleTimer);
    var next = Infinity, idle = (performance.now() - lastInput) / 1000;
    Object.keys(eggs).forEach(function (n) {
      eggs[n].when.forEach(function (c) { if (c.player && c.player.idle != null && c.player.idle > idle) next = Math.min(next, c.player.idle - idle); });
    });
    if (next !== Infinity) idleTimer = setTimeout(check, next * 1000 + 20);
  }

  /* ---------- what happens ---------- */
  function onInput(e) {
    lastInput = performance.now();
    // a tap on a gesture's target counts before the next input ends the live eggs
    var target = e.type === 'pointerdown' && e.target.closest ? Object.keys(GESTURES).filter(function (k) { return e.target.closest(GESTURES[k]); })[0] : null;
    Object.keys(live).forEach(function (n) { if (!live[n].moment || n !== target) end(n); });
    if (target) {
      var list = (taps[target] = (taps[target] || []).concat(performance.now()).slice(-12));
      moment({ kind: 'gesture', on: target, count: list.length });
    }
    check();
  }
  window.addEventListener('pointerdown', onInput, { capture: true, passive: true });
  window.addEventListener('keydown', onInput, { capture: true, passive: true });
  window.addEventListener('wheel', function () { lastInput = performance.now(); }, { capture: true, passive: true });

  var lastTrack = P.currentId(), lastQueueUid = null;
  App.on('player', function (kind) {
    if (kind === 'track') {
      var id = P.currentId(), q = ps.queue[ps.index], uid = q ? q.uid : null;
      // played through again: the same song back by itself (repeat one), not by the listener's choice
      if (id && id === lastTrack && uid !== lastQueueUid && ps.repeat === 'one') {
        repeat.times += 1;
        moment({ kind: 'repeat' });
      } else if (id !== lastTrack) repeat = { trackId: id, times: 0 };
      lastTrack = id; lastQueueUid = uid;
    }
    check();
  });
  App.on('heard', check);
  App.on('route', function () { check(); });
  App.on('sounds', refresh);
  App.on('moment', function (event) { moment({ kind: 'event', on: event }); });
  var counts = { likes: count('liked'), follows: count('followed') };
  function count(key) { var o = App.store.get ? App.store.get(key) : null; return o ? Object.keys(o).length : 0; }
  App.store.subscribe(function (keys) {
    [['liked', 'likes'], ['followed', 'follows']].forEach(function (k) {
      if (keys.indexOf(k[0]) < 0) return;
      var before = counts[k[1]], after = count(k[0]);
      counts[k[1]] = after;
      if (after > before) moment({ kind: 'milestone', on: k[1], before: before, after: after });
    });
  });

  /* ---------- the API ---------- */
  App.eggs = {
    /* the egg that takes this event's sound's place this time (sounds.js asks right after the moment), or null */
    instead: function (event) { var n = insteadFor[event] || null; delete insteadFor[event]; return n; },
    /* fire an egg by hand, whatever its conditions and `once` (Theme Studio and Settings let an author try one) */
    fire: function (name) {
      if (!eggs[name]) return false;
      if (live[name]) end(name);
      delete fired[name];
      return fire(name, { on: true, count: null, word: null }, true);
    },
    live: function () { return Object.keys(live); },
    list: function () { return eggs; }
  };
  /* the credits, the `credits` egg's place */
  App.route('/credits', { title: 'Credits', render: credits });
  function credits() {
    var html = App.html, known = App.sounds.known();
    var packs = Object.keys(known).map(function (k) { return known[k]; });
    return html`<div class="sys-page lull-credits"><h1 class="t-display">credits</h1>
      <div class="card sys-card"><p class="sys-lede">lull is made in the open by the people who write it, and stands on the work of others.</p>
      <h2 class="t-h2">Sounds and surprises</h2><ul>${packs.map(function (p) { return html`<li><b>${p.name}</b>${p.author ? html` by ${p.author}` : ''}</li>`; })}</ul>
      <h2 class="t-h2">Open parts</h2><ul>${OPEN.map(function (o) { return html`<li><b>${o[0]}</b>: ${o[1]}</li>`; })}</ul></div></div>`;
  }
  var OPEN = [
    ['Webamp', 'Classic mode, MIT licence'], ['DOMPurify', 'keeps themes safe, Apache 2.0 or MPL 2.0'],
    ['qrcode-generator', 'pairing codes, MIT licence'], ['slskd', 'the way to Soulseek, AGPL 3.0'],
    ['yt-dlp', 'YouTube as a second source, the Unlicense'], ['Deno', 'yt-dlp\'s runtime, MIT licence'], ['Tauri', 'the app, MIT or Apache 2.0']
  ];

  refresh();
})();
