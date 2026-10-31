/* screens/system.js.
   Stats (/stats), Settings (/settings/:section?), first-run welcome (/welcome), the shortcuts overlay (?),
   the command palette (Ctrl/Cmd+K), the token gate, PWA install / update, and the service banners.
   Reads: GET /api/stats, GET + PATCH /api/settings, GET /api/lastfm, store `services` and App.server
   (docs/context/web.md, docs/context/api.md).
   Sections: 0 helpers + generic controls, 1 stats, 2 settings, 3 shortcuts overlay, 4 command palette,
   5 welcome, 6 token gate, 7 PWA, 8 banners. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var P = App.player;
  var SETTINGS = '/api/settings', LASTFM = '/api/lastfm', FEED = '/api/feed', SKIN_FAVS = '/api/skins/favourites';

  /* =====================================================================
     0. Helpers and generic controls
     ===================================================================== */
  /* the mini player and the wrapper's own settings exist only inside the desktop app. desktop.js loads after this
     file, so the shortcut checks for Tauri itself (as desktop.js does) */
  function inWrapper() { return !!(window.LullDesktop && window.LullDesktop.inWrapper); }
  if (window.__TAURI__) App.shortcuts.push({ group: 'Panels', keys: ['Ctrl+Shift+M'], label: 'Mini player (desktop app)' });

  function num(x) { return Number(x).toLocaleString('en'); }
  function hours(min) {
    var h = Math.floor(min / 60), m = Math.round(min % 60);
    return h ? num(h) + ' h' + (m && h < 100 ? ' ' + m + ' min' : '') : m + ' min';
  }
  function pct(x) { return Math.round(x * 100) + '%'; }
  function share(part, whole) { return whole ? part / whole : 0; }
  function uniq(list) { return list.filter(function (x, i) { return x != null && list.indexOf(x) === i; }); }
  function isArtistId(id) { return /^ar-/.test(id); }
  /* the server's own sentence, without a trailing full stop, so it can sit inside ours */
  function clause(text) { return String(text || '').replace(/\.\s*$/, ''); }
  /* replace a part of the page and put focus back on an equivalent control */
  function repaint(sel, markup, focusSel) {
    var el = $(sel); if (!el) return;
    el.innerHTML = String(markup);
    if (focusSel) { var f = $(focusSel, el); if (f) f.focus(); }
  }
  /* the server-side health check (GET /health): services and version land in the store and App.server */
  function checkHealth() { return App.checkHealth(); }

  var STATE_WORD = { ok: 'OK', warn: 'Warning', down: 'Down', off: 'Not reported' };
  function statusLine(st, text) { return html`<span class="sys-status sys-status--${st}">${ui.statusGlyph(st, STATE_WORD[st])}<span>${text}</span></span>`; }
  function spinnerLine(text) { return html`<span class="sys-status"><span class="spin-ring" aria-hidden="true"></span><span>${text}</span></span>`; }
  /* a read that failed inside a card: the server's sentence and a retry of that read */
  function readError(err, path, title) {
    return ui.emptyState({ art: 'star', title: title, text: err.message, action: { label: 'Try again', act: 'reload-read', id: path } });
  }

  /* ---------- settings values ----------
     GET /api/settings: the stored keys plain, values fixed by the server config as { value, source: 'config' }
     (shown read-only, never edited), secrets as { value: null, source: 'config', secret: true, set }
     (only whether they are set; DESIGN.md 14.10). Appearance is per device; Last.fm's switches are /api/lastfm. */
  function isConfig(x) { return !!x && typeof x === 'object' && x.source === 'config'; }
  function val(x) { return isConfig(x) ? x.value : x; }
  function configText(x) {
    if (x && x.secret) return x.set ? 'set in server config' : 'not set';
    var v = val(x); return v == null || v === '' ? 'not set' : String(v);
  }
  function settings() { return M.peek(SETTINGS); }
  function lookup(path) { return path.split('.').reduce(function (o, k) { return o == null ? o : o[k]; }, settings()); }
  function locked(path) { return isConfig(lookup(path)); }

  /* values that are not a plain server setting: per-device prefs (device.*) and derived ones; path -> { get, set } */
  var custom = {};
  /* UI side effects of a change (repaint a dependent part, announce); run at once */
  var effects = {};
  var fmtRange = {};                              // path -> fn(value) -> visible text / aria-valuetext

  function get(path) { var c = custom[path]; return c && c.get ? c.get() : val(lookup(path)); }
  function setVal(path, v) {
    if (typeof get(path) === 'number') v = +v;
    if (custom[path] && custom[path].set) custom[path].set(v); else save(one(path, v));
    if (effects[path]) effects[path](v);
  }
  function one(path, v) { var c = {}; c[path] = v; return c; }
  function assign(obj, path, v) {
    var ks = path.split('.'), last = ks.pop();
    ks.forEach(function (k) { obj = obj[k] = obj[k] || {}; });
    obj[last] = v;
  }
  /* PATCH /api/settings with only the changed keys, as a partial settings object. The change shows at once;
     the server refuses a bad change whole (400), and then it is put back and the server's sentence is shown. */
  function save(changes) {
    var s = settings(), body = {}, before = {}, paths = Object.keys(changes);
    paths.forEach(function (path) {
      before[path] = lookup(path);
      assign(s, path, changes[path]); assign(body, path, changes[path]);
    });
    M.api(SETTINGS, { method: 'PATCH', body: body }).then(function () {
      if (paths.some(function (p) { return p.indexOf('playback.') === 0; })) App.setPlayback(playbackPrefs());
    }, function (err) {
      paths.forEach(function (path) { assign(s, path, before[path]); });
      App.toast('Couldn\'t save that setting: ' + err.message, { kind: 'error' });
      App.refresh();
    });
  }
  /* what the player engine needs from Settings > Playback */
  function playbackPrefs() {
    var pb = settings().playback;
    return { gapless: !!val(pb.gapless), crossfade: +val(pb.crossfade) || 0, normalisation: val(pb.normalisation) || 'off',
      loudnessTarget: +val(pb.loudnessTarget) || -14, radioWhenQueueEnds: !!val(pb.radioWhenQueueEnds), volumeStep: +val(pb.volumeStep) || 5 };
  }
  /* the `settings` event (any device's change, our own included): merge what differs, re-render if anything did */
  App.on('server:settings', function (stored) {
    var s = settings(), changed = false;
    if (!s) return;
    Object.keys(stored).forEach(function (sec) {
      Object.keys(stored[sec]).forEach(function (k) {
        var path = sec + '.' + k;
        if (JSON.stringify(val(lookup(path))) === JSON.stringify(stored[sec][k])) return;
        assign(s, path, stored[sec][k]); changed = true;
      });
    });
    if (changed && App.current.path.indexOf('/settings') === 0) App.refresh();
  });

  /* ---------- generic controls. Each reads and writes its value by its data-key path. ---------- */
  /* a config-fixed value in place of a control: read-only, with a lock */
  function lockedValue(label, text) {
    return html`<span class="input input--readonly sys-locked" tabindex="0" aria-label="${label}: ${text}. Set in server config, read only." data-tip="Set in server config">${icon('lock', null, 'ic-16')}<span class="ellipsis">${text}</span></span>`;
  }

  /* one-of-N choice: a segmented pill with role=radiogroup; arrows move and select */
  function choice(path, label, options, cls) {
    var cur = String(get(path)), i = options.map(function (o) { return String(o[0]); }).indexOf(cur);
    if (locked(path)) return lockedValue(label, i < 0 ? cur : options[i][1]);
    return html`<div class="seg sys-seg ${cls || ''} ${i < 0 ? 'sys-seg--none' : ''}" role="radiogroup" aria-label="${label}" data-key="${path}" style="--n:${options.length};--i:${Math.max(0, i)}"><span class="seg__ind" aria-hidden="true"></span>${options.map(function (o, j) {
      return html`<button type="button" class="seg__btn" role="radio" aria-checked="${j === i ? 'true' : 'false'}" tabindex="${j === Math.max(0, i) ? '0' : '-1'}" data-value="${o[0]}">${o[1]}</button>`;
    })}</div>`;
  }
  /* reflect a value in a choice group without running its effect (e.g. theme changed with T) */
  function showChoice(path, value) {
    $$('[data-key="' + path + '"][role="radiogroup"]').forEach(function (g) {
      var btns = $$('[role="radio"]', g);
      g.classList.toggle('sys-seg--none', !btns.some(function (b) { return b.getAttribute('data-value') === String(value); }));
      btns.forEach(function (b, j) {
        var on = b.getAttribute('data-value') === String(value);
        b.setAttribute('aria-checked', on ? 'true' : 'false'); b.tabIndex = on ? 0 : -1;
        if (on) g.style.setProperty('--i', j);
      });
    });
  }
  function pickChoice(group, btn, focus) {
    var btns = $$('[role="radio"]', group);
    btns.forEach(function (b) { var on = b === btn; b.setAttribute('aria-checked', on ? 'true' : 'false'); b.tabIndex = on ? 0 : -1; });
    group.style.setProperty('--i', btns.indexOf(btn)); group.classList.remove('sys-seg--none');
    if (focus) btn.focus();
    setVal(group.getAttribute('data-key'), btn.getAttribute('data-value'));
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('[data-key][role="radiogroup"] [role="radio"]');
    if (b && b.getAttribute('aria-checked') !== 'true') pickChoice(b.closest('[role="radiogroup"]'), b);
  });
  document.addEventListener('keydown', function (e) {
    var b = e.target.closest && e.target.closest('[data-key][role="radiogroup"] [role="radio"]'); if (!b) return;
    var d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key]; if (!d) return;
    e.preventDefault();
    var g = b.closest('[role="radiogroup"]'), btns = $$('[role="radio"]', g);
    pickChoice(g, btns[(btns.indexOf(b) + d + btns.length) % btns.length], true);
  });

  /* switch row: the whole row is the label; help text is the description. opts.disabled = the reason, shown on press */
  function switchRow(path, label, help, opts) {
    opts = opts || {};
    var id = App.uid('sys-sw'), on = !!get(path), disabled = opts.disabled || (locked(path) && 'This one is set in server config');
    return html`<label class="switch-row sys-row ${disabled ? 'is-disabled' : ''}">
      <span class="sys-row__text"><span class="sys-row__label" id="${id}">${label}</span>${help ? html`<span class="sys-row__help" id="${id}-h">${help}</span>` : ''}</span>
      <button type="button" class="switch" role="switch" aria-checked="${on ? 'true' : 'false'}" aria-labelledby="${id}" ${help ? raw('aria-describedby="' + id + '-h"') : ''} data-switch="${path}" ${disabled ? raw('aria-disabled="true" data-locked-msg="' + App.esc(disabled) + '"') : ''}></button></label>`;
  }
  document.addEventListener('click', function (e) {
    var s = e.target.closest && e.target.closest('[data-switch]'); if (!s) return;
    if (s.getAttribute('aria-disabled') === 'true') { App.toast(s.getAttribute('data-locked-msg'), { icon: 'lock' }); return; }
    var on = s.getAttribute('aria-checked') !== 'true';
    s.setAttribute('aria-checked', on ? 'true' : 'false');
    setVal(s.getAttribute('data-switch'), on);
  });

  /* range with a visible value; saved when let go (change), shown while dragging (input) */
  function range(path, label, min, max, step) {
    var v = get(path), text = fmtRange[path] ? fmtRange[path](v) : String(v);
    if (locked(path)) return lockedValue(label, text);
    return html`<div class="sys-range"><input class="range" type="range" role="slider" min="${min}" max="${max}" step="${step}" value="${v}" style="--fill:${(v - min) / (max - min) * 100}%" aria-label="${label}" aria-valuetext="${text}" data-range="${path}"><output class="sys-range__out num" data-out="${path}" aria-hidden="true">${text}</output></div>`;
  }
  document.addEventListener('input', function (e) {
    var r = e.target.closest && e.target.closest('[data-range]'); if (!r) return;
    var path = r.getAttribute('data-range'), v = +r.value, text = fmtRange[path] ? fmtRange[path](v) : String(v);
    r.style.setProperty('--fill', (v - r.min) / (r.max - r.min) * 100 + '%');
    r.setAttribute('aria-valuetext', text);
    $$('[data-out="' + path + '"]').forEach(function (o) { o.textContent = text; });
  });
  document.addEventListener('change', function (e) {
    var r = e.target.closest && e.target.closest('[data-range]'); if (r) setVal(r.getAttribute('data-range'), +r.value);
  });

  /* stepper: minus / value / plus */
  function stepper(path, label, min, max, step, unit) {
    var v = get(path);
    if (locked(path)) return lockedValue(label, v + (unit ? ' ' + unit : ''));
    return html`<div class="sys-stepper" role="group" aria-label="${label}" data-stepper="${path}" data-min="${min}" data-max="${max}" data-step="${step}" data-unit="${unit || ''}">
      <button type="button" class="btn-icon btn-icon--sm" data-dir="-1" aria-label="Decrease ${label.toLowerCase()}" ${v <= min ? raw('disabled') : ''}>${icon('minus', null, 'ic-16')}</button>
      <output class="sys-stepper__val num" aria-live="polite">${v}${unit ? ' ' + unit : ''}</output>
      <button type="button" class="btn-icon btn-icon--sm" data-dir="1" aria-label="Increase ${label.toLowerCase()}" ${v >= max ? raw('disabled') : ''}>${icon('plus', null, 'ic-16')}</button></div>`;
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('[data-stepper] [data-dir]'); if (!b) return;
    var g = b.closest('[data-stepper]'), path = g.getAttribute('data-stepper');
    var min = +g.getAttribute('data-min'), max = +g.getAttribute('data-max'), unit = g.getAttribute('data-unit');
    var v = App.clamp(get(path) + (+b.getAttribute('data-dir')) * +g.getAttribute('data-step'), min, max);
    setVal(path, v);
    $('output', g).textContent = v + (unit ? ' ' + unit : '');
    var btns = $$('[data-dir]', g); btns[0].disabled = v <= min; btns[1].disabled = v >= max;
    if (b.disabled) btns[b === btns[0] ? 1 : 0].focus();
  });

  /* a settings row: label + help left, control right; opts.when names a group of rows shown together */
  function row(label, help, control, opts) {
    opts = opts || {};
    return html`<div class="sys-row ${opts.cls || ''}" ${opts.when ? raw('data-when="' + opts.when + '"') : ''} ${opts.hidden ? raw('hidden') : ''}>
      <div class="sys-row__text"><span class="sys-row__label">${label}</span>${help ? html`<span class="sys-row__help">${help}</span>` : ''}</div>
      <div class="sys-row__ctl">${control}</div></div>`;
  }
  /* a value only the server config sets (URLs, credentials, naming): read-only */
  function configRow(label, x, help) {
    return row(label, help || 'Set in server config. Change it there and restart the server.', lockedValue(label, configText(x)), { cls: 'sys-row--locked' });
  }
  /* follow toggle chip; the check is always in the markup and CSS shows it only when pressed (.chip__check) */
  function followChip(id, name, act) {
    return html`<button type="button" class="chip" data-act="${act || 'follow-tag'}" data-id="${id}" data-name="${name}" aria-pressed="${App.isFollowed(id) ? 'true' : 'false'}">${icon('check', null, 'chip__check')}${name}</button>`;
  }
  function group(title) { return html`<div class="sys-group">${ui.labelRule(title, { level: 'h3', sm: true })}</div>`; }

  /* =====================================================================
     1. Stats (/stats?period=7d|30d|12m|all): GET /api/stats?period=
     ===================================================================== */
  /* [id, tab label, phrase, days in the period (0: unknown, all time)] */
  var PERIODS = [['7d', '7 days', 'in the last 7 days', 7], ['30d', '30 days', 'in the last 30 days', 30], ['12m', '12 months', 'in the last 12 months', 365], ['all', 'All time', 'since you started', 0]];
  var HEAT = ['--lavender', '--lav', '--peri', '--peri-deep', '--indigo'];
  var NEEDED_MIN = 120;                           // stats and the taste profile need about two hours of listening
  var statsUi = { period: '7d', focusTab: false };

  /* tz = minutes east of UTC, so the clock and the periods follow the listener's own hours */
  function statsPath(p) { return '/api/stats?period=' + p + '&tz=' + -new Date().getTimezoneOffset(); }
  var TASTE = '/api/taste';
  function periodOf(id) { return PERIODS.filter(function (p) { return p[0] === id; })[0] || PERIODS[0]; }
  function hh(h) { return (h < 10 ? '0' : '') + h + ':00'; }
  function topValue(list) { return Math.max.apply(null, [1].concat(list.map(function (x) { return x[1]; }))); }
  /* the clock: plays per local hour, 24 values */
  function heatCells(clock) {
    var max = Math.max.apply(null, clock), total = clock.reduce(function (a, b) { return a + b; }, 0);
    return clock.map(function (v, h) {
      return { h: h, share: share(v, total), level: !max || v < max * .12 ? 0 : Math.min(4, Math.ceil(v / max * 4)) };
    });
  }
  /* the busiest two hours */
  function peakRange(clock) {
    var best = -1, at = 0;
    for (var h = 0; h < 24; h++) { var s = clock[h] + clock[(h + 1) % 24]; if (s > best) { best = s; at = h; } }
    return hh(at) + ' and ' + hh((at + 2) % 24);
  }
  function shareText(x) { return x && x < .005 ? 'under 1%' : pct(x); }
  function bar(value, max) { return html`<span class="sys-bar" aria-hidden="true"><i style="--p:${Math.max(.02, share(value, max))}"></i></span>`; }

  function statsTabs(cur) {
    var i = PERIODS.map(function (p) { return p[0]; }).indexOf(cur);
    return html`<div class="seg sys-period" role="tablist" aria-label="Period" style="--n:4;--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${PERIODS.map(function (p, j) {
      return html`<button type="button" class="seg__btn" role="tab" id="sys-tab-${p[0]}" aria-selected="${j === i ? 'true' : 'false'}" aria-controls="sys-stats-panel" tabindex="${j === i ? '0' : '-1'}" data-period="${p[0]}">${p[1]}</button>`;
    })}</div>`;
  }
  function tile(ic, label, value, sub) {
    return html`<div class="card sys-tile"><span class="sys-tile__label">${icon(ic, null, 'ic-16')}${label}</span><span class="sys-tile__value num">${value}</span><span class="sys-tile__sub">${sub}</span></div>`;
  }

  function renderStats(ctx) {
    var per = periodOf(ctx.query.period || statsUi.period), path = statsPath(per[0]), r = M.get(path), d = r.data;
    statsUi.period = per[0];
    function page(lede, body, busy) {
      return html`<div class="sys-page sys-stats">
        <header class="sys-head"><div><h1 class="t-display">stats</h1><p class="sys-lede">${lede}</p></div>${statsTabs(per[0])}</header>
        <div id="sys-stats-panel" role="tabpanel" aria-labelledby="sys-tab-${per[0]}" ${busy ? raw('aria-busy="true"') : ''}>${body}</div></div>`;
    }
    if (r.loading) {
      return page('Counting your plays', html`<div class="sys-tiles">${[1, 2, 3, 4, 5].map(function () { return raw('<div class="card sys-tile" aria-hidden="true"><span class="skel skel--text" style="width:60%"></span><span class="skel skel--title" style="width:45%;height:26px;margin-top:8px"></span></div>'); })}</div>
        <div class="sys-grid">${ui.skeleton('card', 2)}</div>`, true);
    }
    if (r.error) return page('Couldn\'t count your plays.', App.errorCard(r.error, { path: path, title: 'Couldn\'t load your stats' }));
    if (d.minutes < NEEDED_MIN) return page('Stats fill in once you have listened for a while.', emptyStats(d));
    return page(hours(d.minutes) + ' across ' + num(d.plays) + ' plays ' + per[2] + ', mostly between ' + peakRange(d.clock) + '.', statsBody(d, per));
  }

  function emptyStats(d) {
    var p = Math.min(1, d.minutes / NEEDED_MIN);
    return html`<div class="card sys-empty-stats">${ui.emptyState({ art: 'wave', title: 'Not enough listening yet', text: 'Stats and your taste profile need about two hours of music. Play something and check back.', action: { label: 'Find something to play', href: '#/home' } })}
      <div class="sys-empty-stats__progress"><div class="sys-meter-label"><span>${hours(d.minutes)} of 2 h</span><span class="fg-2">${pct(p)}</span></div><div class="progress" role="progressbar" aria-label="Listening needed for stats" aria-valuenow="${Math.round(p * 100)}" aria-valuemin="0" aria-valuemax="100" style="--p:${p}"><i></i></div></div></div>`;
  }

  function statsBody(d, per) {
    var days = per[3], hr = d.hitRate, likedP = share(hr.liked, hr.recommended), savedP = share(hr.saved, hr.recommended);
    /* ids the cache can't resolve render nothing (docs/context/web.md) */
    var artists = d.topArtists.filter(function (x) { return M.artist(x[0]); });
    var tracks = d.topTracks.filter(function (x) { return M.track(x[0]); });
    var maxA = topValue(artists), maxT = topValue(tracks), maxG = topValue(d.topTags);
    var listening = d.minutes / 60 < 100 ? (d.minutes / 60).toFixed(1) : num(Math.round(d.minutes / 60));

    return html`<div class="sys-tiles">
        ${tile('clock', 'Listening', listening + ' h', days ? 'about ' + hours(Math.round(d.minutes / days)) + ' a day' : 'in total')}
        ${tile('play', 'Plays', num(d.plays), days ? num(Math.round(d.plays / days)) + ' a day' : 'in total')}
        ${tile('peer', 'Artists', num(d.artists), num(d.newArtists) + ' new to you')}
        ${tile('saved', 'Saves', num(d.saves), 'to your library')}
        ${tile('next', 'Skip rate', pct(d.skipRate), 'skipped before 30 s')}
      </div>
      <div class="sys-grid">
        <section class="card sys-card" aria-labelledby="sys-h-artists">${ui.labelRule('top artists', { id: 'sys-h-artists', action: { label: 'Library', href: '#/library/artists' } })}
          <ol class="sys-bars">${artists.map(function (x, i) {
            var a = M.artist(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(a, 40, { round: true })}
              <span class="sys-bars__main"><a class="sys-bars__name" href="#/artist/${a.id}">${a.name}</a>${bar(x[1], maxA)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card" aria-labelledby="sys-h-tracks">${ui.labelRule('top songs', { id: 'sys-h-tracks', action: tracks.length ? { label: 'Play all', act: 'sys-play-top' } : null })}
          <ol class="sys-bars" data-ids="${tracks.map(function (x) { return x[0]; }).join(',')}">${tracks.map(function (x, i) {
            var t = M.track(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(t, 40)}
              <span class="sys-bars__main"><button class="sys-bars__name" data-act="play-track" data-id="${t.id}" aria-label="Play ${t.title} by ${t.artist}">${t.title}</button><span class="sys-bars__by"><a href="#/artist/${t.artistId}">${t.artist}</a></span>${bar(x[1], maxT)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card sys-span" aria-labelledby="sys-h-clock">${ui.labelRule('listening clock', { id: 'sys-h-clock' })}
          <p class="sys-card__lede">When you listen, by hour of the day. Darker squares mean more plays.</p>
          <div class="sys-heat" role="img" aria-label="${'Listening clock. Most listening between ' + peakRange(d.clock) + '.'}">
            <span></span>${[0, 6, 12, 18].map(function (h) { return html`<span class="sys-heat__hour" style="grid-column:${h + 2} / span 6">${hh(h)}</span>`; })}
            <span class="sys-heat__day">All</span>${heatCells(d.clock).map(function (c) {
              return html`<i class="sys-heat__cell" style="--c:var(${HEAT[c.level]})" data-l="${c.level}" data-tip="${hh(c.h)}: ${shareText(c.share)} of your plays"></i>`;
            })}
          </div>
          <div class="sys-legend" aria-hidden="true"><span>less</span>${HEAT.map(function (v, i) { return html`<i class="sys-heat__cell" style="--c:var(${v})" data-l="${i}"></i>`; })}<span>more</span></div>
        </section>
        <section class="card sys-card" aria-labelledby="sys-h-tags">${ui.labelRule('top tags', { id: 'sys-h-tags' })}
          <ol class="sys-bars sys-bars--tags">${d.topTags.map(function (x) {
            return html`<li class="sys-bars__row">${ui.tagChip(x[0])}<span class="sys-bars__main">${bar(x[1], maxG)}</span><span class="sys-bars__val num">${x[1]}%<span class="vh"> of plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card" aria-labelledby="sys-h-hit">${ui.labelRule('discovery hit rate', { id: 'sys-h-hit' })}
          <p class="sys-hit__big"><span class="num">${pct(likedP)}</span> of the picks you played became likes</p>
          <dl class="sys-funnel">
            <div><dt>Recommended and played</dt><dd>${bar(1, 1)}<span class="num">${num(hr.recommended)}</span></dd></div>
            <div><dt>Liked</dt><dd>${bar(likedP, 1)}<span class="num">${num(hr.liked)}</span></dd></div>
            <div><dt>Saved</dt><dd>${bar(savedP, 1)}<span class="num">${num(hr.saved)}</span></dd></div>
          </dl>
          <div class="inset sys-newart"><span class="sys-newart__n num">${num(d.newArtists)}</span><span>new artists ${per[2]}.</span></div>
        </section>
        ${tasteCard()}
      </div>`;
  }

  /* discovery's taste profile (GET /api/taste): { artists: [[id, weight 0-1]], tags: [[name, weight]] }, its own read */
  function tasteCard() {
    var r = M.get(TASTE);
    function card(body, action) {
      return html`<section class="card sys-card sys-span" aria-labelledby="sys-h-taste" id="sys-taste">
        ${ui.labelRule('your taste profile', { id: 'sys-h-taste', action: action })}${body}</section>`;
    }
    if (r.loading) return card(ui.skeleton('row', 3));
    if (r.error) return card(readError(r.error, TASTE, 'Couldn\'t load your taste profile'));
    var taste = r.data, artists = taste.artists.filter(function (x) { return M.artist(x[0]); }), empty = !artists.length && !taste.tags.length;
    function col(title, list, kind) {
      return html`<div class="sys-taste__col">${ui.labelRule(title, { level: 'h3', sm: true })}<ol class="sys-bars sys-bars--taste">${list.map(function (x) {
        var name = kind === 'artist' ? M.artist(x[0]).name : x[0], w = Math.round(x[1] * 100);
        var href = kind === 'artist' ? '#/artist/' + x[0] : '#/tag/' + encodeURIComponent(x[0]);
        return html`<li class="sys-bars__row"><span class="sys-bars__main"><a class="sys-bars__name" href="${href}">${name}</a>${bar(x[1], 1)}</span>
          <span class="sys-bars__val num">${w}<span class="vh"> out of 100</span></span>
          <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="sys-taste-why" data-kind="${kind}" data-id="${x[0]}" data-weight="${w}" aria-haspopup="dialog" aria-label="Why ${name} weighs ${w}" data-tip="Why this weight">${icon('info', null, 'ic-16')}</button></li>`;
      })}</ol></div>`;
    }
    var body = empty
      ? ui.emptyState({ art: 'moon', title: 'No taste profile yet', text: 'Home learns from what you play, like and save next.' })
      : html`<p class="sys-card__lede">This is what Home leans on. Weights come from plays, likes, saves and follows over the last 90 days; 100 is your strongest signal. Use the info buttons to see why, or turn something down.</p>
        <div class="sys-taste">${col('artists', artists, 'artist')}${col('tags', taste.tags, 'tag')}</div>`;
    return card(body, empty ? null : { label: 'Reset taste', act: 'sys-taste-reset' });
  }

  function mountStats(ctx) {
    var tl = $('.sys-period', ctx.el); if (!tl) return;
    function choose(btn) {
      statsUi.period = btn.getAttribute('data-period'); statsUi.focusTab = true;
      history.replaceState(null, '', '#/stats?period=' + statsUi.period);
      App.refresh();
    }
    tl.addEventListener('click', function (e) { var b = e.target.closest('[role="tab"]'); if (b) choose(b); });
    tl.addEventListener('keydown', function (e) {
      var tabs = $$('[role="tab"]', tl), i = tabs.indexOf(document.activeElement);
      var d = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (e.key === 'Home') d = -i; if (e.key === 'End') d = tabs.length - 1 - i;
      if (d == null || i < 0) return;
      e.preventDefault(); choose(tabs[(i + d + tabs.length) % tabs.length]);
    });
    if (statsUi.focusTab) { statsUi.focusTab = false; var sel = $('[aria-selected="true"]', tl); if (sel) sel.focus(); }
  }

  /* weights change with feedback and reset: refetch the taste read in place */
  function refreshTaste() { M.reload(TASTE); }
  App.action('sys-play-top', function (el) {
    var ids = $('[data-ids]', el.closest('.sys-card')).getAttribute('data-ids').split(',');
    P.playList(ids, 0); App.toast('Playing your top songs', { icon: 'play' });
  });
  App.action('sys-taste-why', function (el) {
    var kind = el.getAttribute('data-kind'), id = el.getAttribute('data-id'), w = el.getAttribute('data-weight');
    var per = periodOf(statsUi.period), d = M.peek(statsPath(per[0])), text, name;
    function inTop(list) { return (list.filter(function (x) { return x[0] === id; })[0] || [])[1]; }
    if (kind === 'artist') {
      var plays = inTop(d.topArtists);
      name = M.artist(id).name;
      text = (plays ? num(plays) + ' plays ' + per[2] + '. ' : '') + (App.isFollowed(id) ? 'You follow them. ' : '') + 'Shows up in Home as "Because you like ' + name + '" and seeds ' + name + ' radio.';
    } else {
      var tagShare = inTop(d.topTags);
      name = id;
      text = (tagShare ? tagShare + '% of your plays ' + per[2] + ' are tagged ' + id + '. ' : '') + (App.isFollowed(id) ? 'You follow the tag. ' : '') + 'Home adds fresh releases and picks from this tag.';
    }
    App.popover(el, html`<div class="popover__head">${name}: ${w}</div><div class="popover__body"><p>${text}</p><p class="t-micro fg-2" style="margin-top:6px">via your listening history</p></div>
      <div class="popover__actions"><button class="btn btn--sm" data-act="sys-taste-less" data-id="${id}" data-name="${name}">${icon('minus')}Less of this</button><a class="btn btn--sm" href="${kind === 'artist' ? '#/artist/' + id : '#/tag/' + encodeURIComponent(id)}">Open ${kind}</a></div>`, { label: 'Why ' + name + ' has this weight' });
  });
  /* "less of this" is the same feedback Home sends; the new weight comes back with the taste read */
  App.action('sys-taste-less', function (el) {
    var id = el.getAttribute('data-id'), name = el.getAttribute('data-name');
    App.closePopover();
    M.api('/api/feedback', { method: 'POST', body: { kind: 'less-like', id: id } }).then(function () {
      refreshTaste();
      App.toast('Turned down ' + name, { icon: 'minus' });
    }, function (err) { App.toast('Couldn\'t turn down ' + name + ': ' + err.message, { kind: 'error' }); });
  });
  App.action('sys-taste-reset', function () {
    App.confirm({ title: 'reset your taste profile?', text: 'Home forgets the weights for every artist and tag and relearns from what you play next. Your plays, likes, saves and follows stay as they are.', confirm: 'Reset taste', danger: true }, function () {
      M.api('/api/taste/reset', { method: 'POST' }).then(function () {
        refreshTaste();
        App.toast('Taste profile reset', { icon: 'refresh' });
      }, function (err) { App.toast('Couldn\'t reset the taste profile: ' + err.message, { kind: 'error' }); });
    });
  });

  App.route('/stats', { render: renderStats, mount: mountStats, nav: 'stats', title: 'Stats' });

  /* =====================================================================
     2. Settings (/settings/:section?): GET + PATCH /api/settings; appearance is per device
     ===================================================================== */
  var SECTIONS = [
    ['playback', 'Playback', 'volume-2', 'Gapless, crossfade, loudness'],
    ['sources', 'Sources', 'soulseek', 'Formats, bitrate, fallback'],
    ['cache', 'Cache', 'grid', 'Size, usage, clear'],
    ['library', 'Library and saving', 'library', 'Jellyfin, folder names, duplicates'],
    ['sharing', 'Sharing', 'export', 'What Soulseek peers can download'],
    ['lastfm', 'Last.fm', 'radio', 'Scrobbling and history'],
    ['discovery', 'Discovery', 'sparkle', 'Sources, follows, adventurousness'],
    ['appearance', 'Appearance', 'sun', 'Design, theme, motion, text size'],
    ['shortcuts', 'Shortcuts', 'keyboard', 'Every keyboard shortcut'],
    ['app', 'App', 'device', 'Install, version, server health']
  ];
  function sectionOf(id) { return SECTIONS.filter(function (s) { return s[0] === id; })[0]; }
  /* a section body that needs the server's settings: skeleton, honest error, or the body */
  function withSettings(body) {
    var r = M.get(SETTINGS);
    if (r.loading) return ui.skeleton('text', 6);
    if (r.error) return readError(r.error, SETTINGS, 'Couldn\'t load settings');
    return body();
  }

  /* ---------- playback ---------- */
  fmtRange['playback.crossfade'] = function (v) { return v ? v + ' s' : 'Off'; };
  effects['playback.normalisation'] = function (v) { $$('[data-when="loudness"]').forEach(function (r) { r.hidden = v !== 'loudness'; }); };
  function secPlayback() {
    return html`
      ${switchRow('playback.gapless', 'Gapless playback', 'Where the source allows. Lossy files from Soulseek may still have a short gap.')}
      ${row('Crossfade', 'Blend the end of one song into the next. Albums marked gapless never crossfade.', range('playback.crossfade', 'Crossfade, seconds', 0, 12, 1))}
      ${row('Loudness normalisation', 'ReplayGain reads tags in the file. Loudness measures files that have no tags.', choice('playback.normalisation', 'Loudness normalisation', [['off', 'Off'], ['replaygain-track', 'Track'], ['replaygain-album', 'Album'], ['loudness', 'Loudness']], 'sys-seg--4'))}
      ${row('Target loudness', 'Quieter targets leave more headroom for dynamic music.', choice('playback.loudnessTarget', 'Target loudness', [[-18, '-18 LUFS'], [-14, '-14 LUFS'], [-11, '-11 LUFS']]), { when: 'loudness', hidden: get('playback.normalisation') !== 'loudness', cls: 'sys-row--sub' })}
      ${row('Volume step', 'How far Up, Down and the D-pad move the volume.', choice('playback.volumeStep', 'Volume step', [[2, '2%'], [5, '5%'], [10, '10%']]))}
      ${switchRow('playback.radioWhenQueueEnds', 'Continue with radio when the queue ends', 'Starts a radio from the last song so the music keeps going.')}`;
  }

  /* ---------- sources ---------- */
  /* the format classes the source picker ranks: label -> [what it is, typical kbps for the size estimate] */
  var FORMATS = { 'FLAC 16/44.1': ['16-bit, 44.1 kHz', 1040], 'FLAC 24/96': ['24-bit, 96 kHz', 2800], 'MP3 320': ['320 kbps CBR', 320], 'MP3 V0': ['~245 kbps VBR', 245], 'OPUS 160': ['160 kbps', 160] };
  /* sources.formatPreset -> the order the picker tries formats in */
  var PRESETS = {
    lossless: ['FLAC 16/44.1', 'FLAC 24/96', 'MP3 320', 'MP3 V0', 'OPUS 160'],
    bitrate: ['MP3 320', 'MP3 V0', 'OPUS 160', 'FLAC 16/44.1', 'FLAC 24/96'],
    small: ['OPUS 160', 'MP3 V0', 'MP3 320', 'FLAC 16/44.1', 'FLAC 24/96']
  };
  fmtRange['sources.stallTimeout'] = function (v) { return v + ' s'; };
  effects['sources.formatPreset'] = function () { repaint('#sys-order', formatItems()); };
  effects['sources.autoPick'] = function (v) { App.announce(v ? 'Best source is picked automatically' : 'You will choose the source each time'); };
  function formatItems() {
    return html`${PRESETS[get('sources.formatPreset')].map(function (f, i) {
      var fm = FORMATS[f];
      return html`<li class="sys-order__item" data-format="${f}"><span class="sys-order__n num">${i + 1}</span>${ui.qualityChip(f)}<span class="sys-order__note">${fm[0]}, about ${Math.round(fm[1] * .03)} MB a song</span></li>`;
    })}`;
  }
  function secSources() {
    return html`
      <div class="sys-row sys-row--stack">
        <div class="sys-row__text"><span class="sys-row__label" id="sys-order-l">Preferred formats</span><span class="sys-row__help">The source picker tries these in order, then weighs peer speed, free slots and queue length.</span></div>
        <div class="sys-row__ctl">${choice('sources.formatPreset', 'Format preset', [['lossless', 'Lossless first'], ['bitrate', 'High bitrate first'], ['small', 'Smallest first']], 'sys-seg--wrap')}
          <ol class="sys-order" id="sys-order" aria-labelledby="sys-order-l">${formatItems()}</ol></div></div>
      ${row('Minimum bitrate', 'Lossy files below this are ignored, kbps.', choice('sources.minBitrate', 'Minimum bitrate', [[128, '128'], [192, '192'], [256, '256'], [320, '320']]))}
      ${row('Longest peer queue', 'Skip peers with more people waiting than this.', stepper('sources.maxPeerQueue', 'Longest peer queue', 0, 50, 5, 'waiting'))}
      ${row('Stall timeout', 'If a peer stops sending for this long, switch to the next source.', range('sources.stallTimeout', 'Stall timeout, seconds', 5, 60, 5))}
      ${switchRow('sources.autoPick', 'Pick the best source automatically', 'Off: the source list opens every time you play something new.')}
      ${group('soulseek connection')}
      ${configRow('slskd address', lookup('sources.slskdUrl'), 'lull reaches Soulseek through slskd, which holds your Soulseek account. Set in server config.')}
      ${configRow('slskd API key', lookup('sources.slskdApiKey'))}`;
  }

  /* ---------- cache: size from server config, usage live from the soulseek crate. Nothing here clears it:
     the server has no endpoint for that, and the oldest audio is evicted on its own. ---------- */
  function usageBar() {
    var cap = get('cache.capGB'), used = get('cache.usedGB');
    if (used == null) return html`<div class="sys-usage__head"><span class="sys-usage__big num">${num(cap)} <span class="fg-2">GB cache</span></span>${statusLine('off', 'Usage not reported yet')}</div>`;
    used = Math.min(cap, used);
    var full = used >= cap - .05, free = Math.max(0, cap - used);
    return html`<div class="sys-usage__head"><span class="sys-usage__big num">${used.toFixed(1)} <span class="fg-2">of ${num(cap)} GB used</span></span>${full ? statusLine('warn', 'Full') : statusLine('ok', free.toFixed(1) + ' GB free')}</div>
      <div class="sys-usage__bar" role="img" aria-label="${'Cache: ' + used.toFixed(1) + ' GB of streamed audio, ' + free.toFixed(1) + ' GB free'}"><i class="sys-usage__seg sys-usage__seg--playing" style="--w:${share(used, cap) * 100}%"></i></div>
      <ul class="sys-usage__legend"><li><i class="sys-usage__seg sys-usage__seg--playing" aria-hidden="true"></i>Streamed audio<span class="num">${used.toFixed(1)} GB</span></li><li><i class="sys-usage__seg sys-usage__seg--free" aria-hidden="true"></i>Free<span class="num">${free.toFixed(1)} GB</span></li></ul>`;
  }
  function secCache() {
    return html`<div class="sys-usage" id="sys-usage">${usageBar()}</div>
      ${configRow('Cache size', get('cache.capGB') + ' GB', 'Streamed audio is kept up to this size so replays start instantly. Set in server config.')}
      <p class="sys-note">${icon('info', null, 'ic-16')}<span>When the cache is full, the oldest audio that is not playing or queued goes first. Songs in your queue are always kept, and your saved library is never touched.</span></p>`;
  }

  /* ---------- library and saving ---------- */
  /* the naming template is server config; the preview shows where the playing (or first queued) song would go */
  function previewTrack() { return P.current() || P.state.queue.map(function (q) { return M.track(q.trackId); }).filter(Boolean)[0]; }
  function namingPreview(tpl) {
    var t = previewTrack(), al = t && M.album(t.albumId);
    var vals = t ? { artist: t.artist, album: t.album, year: t.year || (al && al.year) || '', disc: t.disc || 1, 'track:02': String(t.no || 1).padStart(2, '0'), track: t.no || 1, title: t.title, ext: (t.codec || 'flac').toLowerCase() }
      : { artist: 'Artist', album: 'Album', year: 'Year', disc: '1', 'track:02': '01', track: '1', title: 'Title', ext: 'flac' };
    return tpl.replace(/\{([^}]*)\}/g, function (m, k) { return k in vals ? vals[k] : m; });
  }
  function jellyfinStatus() { var st = App.store.get('services').jellyfin; return statusLine(st.state, st.detail); }
  App.action('sys-jellyfin-test', function (el) {
    el.disabled = true; repaint('#sys-jf-status', spinnerLine('Testing connection'));
    checkHealth().then(function () {
      el.disabled = false; repaint('#sys-jf-status', jellyfinStatus());
      App.announce(App.store.get('services').jellyfin.state === 'ok' ? 'Jellyfin connected' : 'Jellyfin did not answer');
    });
  });
  /* the latest rescan, as the Activity list has it (POST /api/library/rescan, then `job` events) */
  function rescanStatus() {
    var job = (App.store.get('activity') || []).filter(function (j) { return j.kind === 'rescan'; })[0];
    if (!job) return '';
    if (job.state === 'failed') return statusLine('down', job.error || job.detail || 'Rescan failed');
    if (job.state === 'done') return statusLine('ok', job.detail || 'Rescan finished');
    return spinnerLine(job.detail || 'Rescanning the library');
  }
  function secLibrary() {
    var naming = get('library.naming'), folder = get('library.musicFolder');
    return html`${row('Jellyfin', 'Where saved songs live and play from.', html`<div class="sys-inline"><span id="sys-jf-status" aria-live="polite">${jellyfinStatus()}</span><button class="btn btn--sm" data-act="sys-jellyfin-test">Test connection</button></div>`)}
      ${configRow('Server URL', lookup('library.jellyfinUrl'))}
      ${configRow('User', lookup('library.jellyfinUser'))}
      ${configRow('API key', lookup('library.jellyfinApiKey'))}
      ${configRow('Music folder', lookup('library.musicFolder'))}
      ${group('saving')}
      ${configRow('Folder and file names', lookup('library.naming'), 'Saved songs are named from this template. Set in server config.')}
      ${naming ? html`<div class="inset sys-preview"><span class="sys-preview__label">Saves as</span><span class="sys-preview__path">${folder || ''}/${namingPreview(naming)}</span></div>` : ''}
      ${row('When a song is already saved', 'What Save does when the library has the same song.', choice('library.duplicatePolicy', 'Duplicate policy', [['ask', 'Ask each time'], ['keep', 'Keep existing'], ['replace', 'Replace if better']], 'sys-seg--wrap'))}
      ${row('Rescan library', 'Picks up files added to the music folder outside lull.', html`<div class="sys-inline sys-inline--col"><button class="btn" data-act="jellyfin-rescan">${icon('refresh')}Rescan library now</button><div id="sys-rescan" class="sys-rescan" aria-live="polite">${rescanStatus()}</div></div>`)}`;
  }

  /* ---------- sharing: what is shared is server config; slots and speed are stored ---------- */
  fmtRange['sharing.uploadLimitKBs'] = function (v) { return v ? num(v) + ' KB/s' : 'No limit'; };
  function secSharing() {
    var on = get('sharing.mode') === 'library';
    return html`${row('Sharing now', '', on ? statusLine('ok', 'Sharing your saved library, read-only') : statusLine('warn', 'Sharing nothing'), { cls: 'sys-row--status' })}
      ${row('Share with Soulseek peers', 'Your saved library, read-only, or nothing. Peers can download songs; they can never change or delete anything.', choice('sharing.mode', 'What to share', [['library', 'Saved library, read-only'], ['none', 'Nothing']], 'sys-seg--wrap'))}
      <p class="sys-note ${on ? '' : 'sys-note--warn'}">${icon(on ? 'info' : 'warning', null, 'ic-16')}<span>Soulseek expects sharing. Many peers queue or refuse people who share nothing, so finding sources gets slower${on ? '.' : ' while this is off.'}</span></p>
      <div class="${on ? '' : 'sys-off'}" ${on ? '' : raw('inert')}>
        ${row('Upload slots', 'How many peers can download from you at once.', stepper('sharing.uploadSlots', 'Upload slots', 1, 20, 1))}
        ${row('Upload speed limit', 'Keeps uploads from slowing your own streams.', range('sharing.uploadLimitKBs', 'Upload speed limit', 0, 2000, 100))}
      </div>`;
  }

  /* ---------- Last.fm: GET /api/lastfm { configured, connected, user, scrobbling, useHistory, queued, error? };
     its two switches are PATCH /api/lastfm. Connecting leaves for Last.fm in this window; the callback comes back
     to #/settings/lastfm (with `error` set when it failed). ---------- */
  ['scrobbling', 'useHistory'].forEach(function (key) {
    custom['lastfm.' + key] = { get: function () { return !!(M.peek(LASTFM) || {})[key]; }, set: function (v) {
      var l = M.peek(LASTFM), patch = {}; patch[key] = v; l[key] = v;
      M.api(LASTFM, { method: 'PATCH', body: patch }).then(null, function (err) {
        l[key] = !v;
        App.toast('Couldn\'t change that: ' + err.message, { kind: 'error' });
        App.refresh();
      });
    } };
  });
  effects['lastfm.scrobbling'] = function (v) { App.announce(v ? 'Scrobbling on' : 'Scrobbling paused'); };
  App.action('sys-lfm-connect', function () { location.href = M.url('/api/lastfm/connect'); });
  App.action('sys-lfm-disconnect', function () {
    var q = (M.peek(LASTFM) || {}).queued || 0;
    App.confirm({ title: 'disconnect Last.fm?', text: 'Scrobbling stops' + (q ? ' and ' + q + ' queued scrobbles are dropped' : '') + '. Recommendations stop using your Last.fm history.', confirm: 'Disconnect', danger: true }, function () {
      M.api(LASTFM, { method: 'DELETE' }).then(function () {
        M.reload(LASTFM);
        App.toast('Last.fm disconnected', { icon: 'radio' });
      }, function (err) { App.toast('Couldn\'t disconnect Last.fm: ' + err.message, { kind: 'error' }); });
    });
  });
  App.action('sys-lfm-retry', function (el) {
    el.disabled = true;
    M.api('/api/lastfm/retry', { method: 'POST' }).then(function () {
      M.reload(LASTFM);
      App.toast('Sending queued scrobbles to Last.fm', { kind: 'ok', icon: 'check' });
    }, function (err) { el.disabled = false; App.toast('Couldn\'t send scrobbles: ' + err.message, { kind: 'error' }); });
  });
  function lastfmBox() {
    var r = M.get(LASTFM), s = r.data;
    if (r.loading) return ui.skeleton('text', 2);
    if (r.error) {
      return html`<div class="inset sys-connect sys-connect--error" role="alert">${ui.statusGlyph('down', 'Error')}<div><p class="sys-connect__title">Couldn't check Last.fm</p><p class="sys-connect__text">${r.error.message}</p></div>
        <div class="sys-connect__actions"><button class="btn btn--primary" data-act="reload-read" data-id="${LASTFM}">Try again</button></div></div>`;
    }
    if (!s.configured) {
      return html`<div class="inset sys-connect">${ui.statusGlyph('off', 'Not set up')}<div><p class="sys-connect__title">Not set up</p><p class="sys-connect__text">Last.fm needs an API key and shared secret in the server config. Add them there and restart the server to connect.</p></div></div>`;
    }
    if (!s.connected && s.error) {
      return html`<div class="inset sys-connect sys-connect--error" role="alert">${ui.statusGlyph('down', 'Error')}<div><p class="sys-connect__title">Couldn't connect to Last.fm</p><p class="sys-connect__text">${s.error}</p></div>
        <div class="sys-connect__actions"><button class="btn btn--primary" data-act="sys-lfm-connect">Try again</button></div></div>`;
    }
    if (!s.connected) {
      return html`<div class="inset sys-connect">${icon('radio', null, 'ic-24')}<div><p class="sys-connect__title">Not connected</p><p class="sys-connect__text">Connect to scrobble what you play and let your Last.fm history shape recommendations. Last.fm opens in this window and brings you back here.</p></div>
        <div class="sys-connect__actions"><button class="btn btn--primary" data-act="sys-lfm-connect">${icon('external')}Connect Last.fm</button></div></div>`;
    }
    return html`<div class="inset sys-connect sys-connect--ok">${ui.statusGlyph('ok', 'Connected')}<div><p class="sys-connect__title">Connected as <b>${s.user}</b></p><p class="sys-connect__text">Scrobbles go to Last.fm after half a song or 4 minutes, whichever comes first.</p></div>
      <div class="sys-connect__actions"><button class="btn" data-act="sys-lfm-disconnect">Disconnect</button></div></div>
      ${switchRow('lastfm.scrobbling', 'Scrobble what I play', 'Library and Soulseek plays both count.')}
      ${row('Queued scrobbles', s.queued ? 'Waiting because Last.fm did not answer. They retry on their own.' : 'Nothing waiting.', s.queued ? html`<div class="sys-inline">${ui.countBadge(s.queued, s.queued + ' queued')}<button class="btn btn--sm" data-act="sys-lfm-retry">Retry now</button></div>` : statusLine('ok', 'All sent'))}
      ${switchRow('lastfm.useHistory', 'Use my Last.fm history for recommendations', 'Adds years of listening to the taste profile on Stats.')}`;
  }
  function secLastfm() {
    return html`<div id="sys-lastfm">${lastfmBox()}</div>
      ${group('app credentials')}
      ${withSettings(function () { return html`${configRow('API key', lookup('lastfm.apiKey'))}${configRow('Shared secret', lookup('lastfm.apiSecret'))}`; })}`;
  }

  /* ---------- discovery ---------- */
  var DSOURCES = [
    ['listenbrainz', 'ListenBrainz', 'Similar artists and what listeners like you play. No account needed.'],
    ['musicbrainz', 'MusicBrainz', 'Release dates, tags and the new-release feed. Saving uses it to match songs either way.'],
    ['lastfm', 'Last.fm', 'Similar artists and tags, plus your history when Last.fm is connected.']
  ];
  fmtRange['discovery.adventurousness'] = function (v) { return v < .34 ? 'Familiar' : v < .67 ? 'Balanced' : 'Adventurous'; };
  DSOURCES.forEach(function (s) { effects['discovery.on.' + s[0]] = function (v) { App.announce(s[1] + (v ? ' on' : ' off')); }; });
  function secDiscovery() {
    var followed = Object.keys(App.store.get('followed') || {});
    var tagIds = followed.filter(function (id) { return !isArtistId(id); }), artistIds = followed.filter(isArtistId);
    var arts = artistIds.map(M.artist).filter(Boolean);
    return html`${row('Adventurousness', 'How far Home and radio stray from what you already play.', html`${range('discovery.adventurousness', 'Adventurousness', 0, 1, .05)}<span class="sys-range__ends" aria-hidden="true"><span>Familiar</span><span>Adventurous</span></span>`)}
      ${group('sources')}
      ${DSOURCES.map(function (s) { return switchRow('discovery.on.' + s[0], s[1], s[2]); })}
      ${configRow('ListenBrainz user', lookup('discovery.listenbrainzUser'))}
      ${group('following')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Tags</span><span class="sys-row__help">Followed tags feed Home with fresh releases. Tap to unfollow; follow more from any tag page.</span></div>
        <div class="sys-row__ctl">${tagIds.length ? html`<div class="cluster sys-chips">${tagIds.map(function (id) { return followChip(id, M.tag(id).name); })}</div>` : html`<p class="sys-row__help">No tags followed yet.</p>`}</div></div>
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Artists</span><span class="sys-row__help">${artistIds.length} followed. New releases from them show first on Home.</span></div>
        <div class="sys-row__ctl"><ul class="sys-follows">${arts.map(function (a) {
          return html`<li>${ui.cover(a, 36, { round: true })}<a href="#/artist/${a.id}">${a.name}</a><button class="btn btn--sm" data-act="sys-follow" data-id="${a.id}" aria-pressed="true">Following</button></li>`;
        })}</ul><a class="btn btn--quiet btn--sm" href="#/following">See everything you follow</a></div></div>`;
  }
  App.action('sys-follow', function (el) { App.toggleFollow(el.getAttribute('data-id')); });
  App.on('follow', function (e) {
    $$('[data-act="sys-follow"][data-id="' + e.id + '"]').forEach(function (b) { b.setAttribute('aria-pressed', e.followed ? 'true' : 'false'); b.textContent = e.followed ? 'Following' : 'Follow'; });
  });

  /* ---------- appearance: all per device (App.setPref, and App.storage for Classic mode's two) ---------- */
  var designFocus = false;                         // the picker had focus: give it back after the re-render
  custom['device.design'] = { get: function () { return App.prefs().design; }, set: function (v) {
    designFocus = !!(document.activeElement && document.activeElement.closest('.sys-designs'));
    App.setPref('design', v);
    App.announce(App.designs[v].label + ' design');
  } };
  custom['device.theme'] = { get: function () { return App.prefs().theme; }, set: function (v) { App.setPref('theme', v === 'system' ? null : v); } };
  custom['device.motion'] = { get: function () { return App.prefs().motion; }, set: function (v) { App.setPref('motion', v); } };
  custom['device.textSize'] = { get: function () { return App.prefs().textSize; }, set: function (v) { App.setPref('textSize', v); } };
  /* Classic mode reads these two from the same storage keys */
  custom['device.defaultSkin'] = { get: function () { return App.storage('defaultSkin'); }, set: function (v) { App.storage('defaultSkin', v); } };
  custom['device.showNsfwSkins'] = { get: function () { return App.storage('showNsfwSkins') === true; }, set: function (v) { App.storage('showNsfwSkins', v); } };
  /* design preview card: the design's screenshot (assets/previews/<name>-{day,night}.webp, when d.preview) over a tiny
     picture of the shell (rail / topnav / dock) in its swatches, which shows while the image loads or when there is none */
  function designCard(name, on) {
    var d = App.designs[name], sw = d.swatches;
    var pv = new URL(Designs.base + '../assets/previews/' + name, location.href).href;   // absolute: a url() in a custom property resolves against the stylesheet that uses it
    var shot = d.preview ? ';--pv-day:url("' + pv + '-day.webp");--pv-night:url("' + pv + '-night.webp")' : '';
    return html`<button type="button" class="sys-design" role="radio" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-value="${name}"
      style="${'--d1:' + sw[0] + ';--d2:' + sw[1] + ';--d3:' + sw[2] + ';--d4:' + sw[3] + shot}">
      <span class="sys-design__pv${d.preview ? ' has-shot' : ''}" data-shell="${d.shell}" aria-hidden="true"><i class="sys-design__nav"></i><i class="sys-design__win"><b></b><b></b><b></b></i><i class="sys-design__win sys-design__win--b"><b></b><b></b></i><i class="sys-design__bar"><b></b></i><i class="sys-design__dock"></i></span>
      <span class="sys-design__name">${d.label}${icon('check', null, 'ic-16 sys-design__check')}</span>
      <span class="sys-design__desc">${d.description}</span></button>`;
  }
  /* favourite skins (GET /api/skins/favourites) to pick the default from */
  function skinPicker() {
    var r = M.get(SKIN_FAVS);
    if (r.loading) return ui.skeleton('chip', 3);
    if (r.error) return html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.error.message}</p><button class="btn btn--sm" data-act="reload-read" data-id="${SKIN_FAVS}">Try again</button>`;
    var skins = r.data.items, cur = get('device.defaultSkin');
    if (!skins.length) return html`<p class="sys-row__help">No favourite skins yet. Star one in Classic mode and it shows up here.</p>`;
    return html`<div class="sys-skins" role="radiogroup" aria-labelledby="sys-skin-l" data-key="device.defaultSkin">${skins.map(function (s) {
      var on = s.md5 === cur;
      return html`<button type="button" class="sys-skin" role="radio" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-value="${s.md5}"><span class="sys-skin__thumb" aria-hidden="true">${s.screenshotUrl ? html`<img src="${s.screenshotUrl}" alt="" loading="lazy" decoding="async">` : ''}</span><span class="sys-skin__name">${s.name}</span>${icon('star-fill', 'Favourite', 'ic-16 sys-skin__fav')}</button>`;
    })}</div>`;
  }
  function secAppearance() {
    var p = App.prefs();
    return html`<div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-design-l">Design</span><span class="sys-row__help">The whole look: colours, type, window chrome and layout. Applies at once on this device.</span></div>
        <div class="sys-row__ctl"><div class="sys-designs" role="radiogroup" aria-labelledby="sys-design-l" data-key="device.design">${App.designOrder.map(function (n) { return designCard(n, n === p.design); })}</div></div></div>
      ${row('Theme', 'Match system follows your device. T switches day and night anywhere.', choice('device.theme', 'Theme', [['system', 'Match system'], ['day', 'Day'], ['night', 'Night']]))}
      ${row('Motion', 'Reduced swaps slides and loops for quick fades. Full ignores the system setting.', choice('device.motion', 'Motion', [['system', 'Match system'], ['reduce', 'Reduced'], ['full', 'Full']]))}
      ${row('Text size', 'Scales all text and spacing on this device.', choice('device.textSize', 'Text size', [[90, 'Smaller'], [100, 'Default'], [112, 'Larger'], [125, 'Largest']], 'sys-seg--4'))}
      ${group('classic mode')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-skin-l">Default skin</span><span class="sys-row__help">Used when Classic mode opens on this device. Your favourite skins are listed; find more in Classic mode.</span></div>
        <div class="sys-row__ctl">${skinPicker()}<a class="btn btn--quiet btn--sm" href="#/classic">${icon('classic')}Browse skins in Classic mode</a></div></div>
      ${switchRow('device.showNsfwSkins', 'Show NSFW skins', 'Hidden by default in the skin browser on this device.')}`;
  }

  /* ---------- shortcuts (read-only list) ---------- */
  function secShortcuts() {
    return html`<p class="sys-card__lede">The same list opens anywhere with ${ui.keycap('?')}. Shortcuts pause while you type in a field and inside Classic mode.</p>
      <p><button class="btn" data-act="shortcuts">${icon('keyboard')}Open the shortcuts overlay</button></p>${shortcutList('')}`;
  }

  /* ---------- app: install, version, server health ---------- */
  function isIos() { return /iPad|iPhone|iPod/.test(navigator.userAgent) || (navigator.platform === 'MacIntel' && navigator.maxTouchPoints > 1); }
  function installBody() {
    if (App.pwa.installed()) return statusLine('ok', 'Installed on this device');
    if (App.pwa.installable()) return html`<button class="btn btn--primary" data-act="sys-install">${icon('device')}Install app</button>`;
    if (isIos()) return html`<button class="btn btn--primary" data-act="sys-ios-sheet">${icon('device')}How to install</button>`;
    return html`<span class="sys-row__help">This browser doesn't offer to install from here. Look for Install or Add to Home Screen in its menu.</span>`;
  }
  App.action('sys-install', function () { installApp(); });
  App.action('sys-ios-sheet', function () { openIosSheet(); });
  App.action('sys-update-check', function (el) {
    var before = App.server.version;
    el.disabled = true;
    checkHealth().then(function () {
      el.disabled = false;
      if (!App.server.reachable) App.toast('Couldn\'t reach your server to check for updates', { kind: 'error' });
      else if (App.server.version === before) App.toast(App.product + ' ' + before + ' is the version your server runs', { icon: 'check' });
      /* a new version raises the update toast through App.on('pwa') */
    });
  });
  var NAMES = { soulseek: 'Soulseek', jellyfin: 'Jellyfin', lastfm: 'Last.fm', cache: 'Cache' };
  function healthRows(checked) {
    var s = App.store.get('services'), srv = App.server;
    var rows = [[srv.reachable ? 'ok' : 'down', 'Server', srv.reachable ? 'Answering' + (srv.version ? ', version ' + srv.version : '') : 'Not answering']]
      .concat(Object.keys(NAMES).map(function (k) { return [s[k].state, NAMES[k], s[k].detail]; }));
    return html`<div class="status-list">${rows.map(function (r) {
      return html`<div class="status-row">${ui.statusGlyph(r[0], STATE_WORD[r[0]])}<div><span class="status-row__name">${r[1]}</span><span class="status-row__detail">${r[2]}</span></div></div>`;
    })}</div><p class="sys-card__lede sys-health__when">${checked ? 'Checked just now from ' : 'Kept current by the server, from '}<span class="px">/health</span></p>`;
  }
  App.action('sys-health', function (el) {
    var out = $('#sys-health'); el.disabled = true; out.setAttribute('aria-busy', 'true');
    checkHealth().then(function () {
      el.disabled = false; out.removeAttribute('aria-busy');
      repaint('#sys-health', healthRows(true)); App.announce('Health checked');
    });
  });
  function serverAddress() { return location.href.split('#')[0].replace(/index\.html$/, ''); }

  /* ---------- the desktop wrapper's own settings (LullDesktop.settings / setSettings), read once per visit ---------- */
  var desk = null;                                 // { loading, error, data }
  function errText(err) { return err && err.message || String(err); }
  function loadDesktop() {
    desk = { loading: true };
    window.LullDesktop.settings().then(function (d) { desk = { data: d }; repaint('#sys-desktop', desktopRows()); },
      function (err) { desk = { error: errText(err) }; repaint('#sys-desktop', desktopRows()); });
  }
  ['closeToTray', 'launchAtLogin', 'startInTray', 'miniStyle', 'miniOnTop', 'ownTitlebar'].forEach(function (key) {
    custom['desktop.' + key] = { get: function () { return desk.data[key]; }, set: function (v) {
      var before = desk.data[key], patch = {}; patch[key] = v; desk.data[key] = v;
      window.LullDesktop.setSettings(patch).then(null, function (err) {
        desk.data[key] = before;
        App.toast('Couldn\'t change that: ' + errText(err), { kind: 'error' });
        repaint('#sys-desktop', desktopRows());
      });
    } };
  });
  effects['desktop.ownTitlebar'] = function () { App.toast('Restart lull to change the title bar', { icon: 'refresh' }); };
  App.action('sys-desktop-connect', function () { window.LullDesktop.windowAction('change-server'); });
  App.action('sys-desktop-reload', function () { loadDesktop(); repaint('#sys-desktop', desktopRows()); });
  function desktopRows() {
    if (desk.loading) return ui.skeleton('text', 4);
    if (desk.error) return ui.emptyState({ art: 'star', title: 'Couldn\'t read the desktop app\'s settings', text: desk.error, action: { label: 'Try again', act: 'sys-desktop-reload' } });
    var d = desk.data, noTray = !d.tray && 'This desktop has no tray icon';
    return html`${row('Server address', 'The desktop app loads lull from this server.', html`<span class="sys-inline">${lockedValue('Server address', d.server || serverAddress())}<button class="btn btn--sm" data-act="sys-desktop-connect">Change server...</button></span>`)}
      ${switchRow('desktop.closeToTray', 'Close to tray', 'Closing the window keeps the music playing; quit from the tray icon.', { disabled: noTray })}
      ${switchRow('desktop.launchAtLogin', 'Launch at login', 'Starts with your computer.')}
      ${switchRow('desktop.startInTray', 'Start in the tray', 'Launches without opening the window.', { disabled: noTray })}
      ${row('Mini player style', 'Opens from the tray, the player bar or Ctrl+Shift+M.', choice('desktop.miniStyle', 'Mini player style', [['device', 'Device'], ['strip', 'Strip']]))}
      ${switchRow('desktop.miniOnTop', 'Mini player always on top', 'Some Linux desktops decide this themselves.')}
      ${d.ownTitlebar == null ? '' : switchRow('desktop.ownTitlebar', 'Use lull title bar', 'Replaces the system title bar; takes effect after a restart.')}`;
  }
  function secApp() {
    if (inWrapper() && !desk) loadDesktop();
    return html`${row('Version', '', html`<span class="sys-inline"><span class="px sys-version" id="sys-version">${App.server.version || 'not reported yet'}</span><button class="btn btn--sm" data-act="sys-update-check">Check for updates</button></span>`)}
      ${row('Install app', 'Adds lull to your home screen or dock with lock-screen controls and its own window.', html`<div id="sys-install">${installBody()}</div>`)}
      ${row('First-run steps', 'Connect Last.fm, follow a few tags, try a search.', html`<a class="btn" href="#/welcome">Show welcome steps</a>`)}
      ${group('server')}
      ${inWrapper() ? '' : configRow('Server address', serverAddress(), 'The address this page was loaded from.')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Server health</span><span class="sys-row__help">What the server reports about itself and its services.</span></div>
        <div class="sys-row__ctl"><div id="sys-health" aria-live="polite">${healthRows(false)}</div><button class="btn btn--sm" data-act="sys-health">${icon('refresh')}Check now</button></div></div>
      ${inWrapper() ? html`${group('desktop app')}<div id="sys-desktop">${desktopRows()}</div>` : ''}`;
  }

  var BODY = {
    playback: function () { return withSettings(secPlayback); }, sources: function () { return withSettings(secSources); },
    cache: function () { return withSettings(secCache); }, library: function () { return withSettings(secLibrary); },
    sharing: function () { return withSettings(secSharing); }, lastfm: secLastfm,
    discovery: function () { return withSettings(secDiscovery); }, appearance: secAppearance, shortcuts: secShortcuts, app: secApp
  };

  function renderSettings(ctx) {
    var sec = ctx.params.section, cur = sectionOf(sec) ? sec : 'playback', s = sectionOf(cur);
    return html`<div class="sys-page sys-settings ${sec ? 'has-section' : 'is-index'}">
      <h1 class="t-display sys-settings__title">settings</h1>
      <div class="sys-settings__grid">
        <nav class="sys-nav s-panel" aria-label="Settings sections"><ul>${SECTIONS.map(function (x) {
          var on = x[0] === cur;   // with no section, Playback shows beside the list (wide) but is not "current" in the phone list
          return html`<li><a class="sys-nav__item ${on && !sec ? 'sys-nav__item--default' : ''}" href="#/settings/${x[0]}" ${on && sec ? raw('aria-current="page"') : ''}>${icon(x[2])}<span class="sys-nav__text"><span class="sys-nav__label">${x[1]}</span><span class="sys-nav__desc">${x[3]}</span></span>${icon('chevron-right', null, 'ic-16 sys-nav__chev')}</a></li>`;
        })}</ul></nav>
        <section class="sys-detail" aria-labelledby="sys-sec-title">
          <a class="btn btn--quiet sys-back" href="#/settings">${icon('chevron-left')}All settings</a>
          <div class="card sys-card sys-section" id="sys-section">${ui.labelRule(s[1].toLowerCase(), { id: 'sys-sec-title' })}${BODY[cur]()}</div>
        </section>
      </div></div>`;
  }
  /* live parts repaint in place: services (Jellyfin, health), jobs (rescan), the server, install state */
  function mountSettings(ctx) {
    var offs = [
      App.on('prefs', function () {
        var p = App.prefs(); showChoice('device.theme', p.theme); showChoice('device.motion', p.motion); showChoice('device.design', p.design);
      }),
      App.on('design', function (name) {
        if (!designFocus) return; designFocus = false;
        var b = $('.sys-design[data-value="' + name + '"]'); if (b) b.focus();
      }),
      App.store.subscribe(function (keys) {
        if (keys.indexOf('services') >= 0) { repaint('#sys-jf-status', jellyfinStatus()); repaint('#sys-health', healthRows(false)); }
        if (keys.indexOf('activity') >= 0) repaint('#sys-rescan', rescanStatus());
      }),
      App.on('server', function () {
        repaint('#sys-health', healthRows(false));
        var v = $('#sys-version'); if (v) v.textContent = App.server.version || 'not reported yet';
      }),
      App.on('pwa', function () { repaint('#sys-install', installBody()); })
    ];
    ctx.onCleanup(function () {
      offs.forEach(function (off) { off(); });
      if (!/^#\/settings\/app\b/.test(location.hash)) desk = null;   // leaving Settings > App: read the wrapper again next visit
    });
  }
  App.route('/settings/:section?', { render: renderSettings, mount: mountSettings, nav: 'settings', title: function (ctx) { var s = sectionOf(ctx.params.section); return s ? s[1] + ' settings' : 'Settings'; } });

  /* =====================================================================
     3. Shortcuts overlay (?): grouped keycaps + filter
     ===================================================================== */
  var MAC = /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  function keysMarkup(k) {
    if (/^G\+[A-Z]$/.test(k)) return html`${ui.keycap('G')}<span class="sys-then">then</span>${ui.keycap(k.slice(2))}`;
    return html`${k.split('+').map(function (p, i) { return html`${i ? html`<span class="sys-plus" aria-hidden="true">+</span>` : ''}${ui.keycap(p === 'Ctrl' && MAC ? 'Cmd' : p)}`; })}`;
  }
  function shortcutList(q) {
    q = (q || '').trim().toLowerCase();
    var groups = {}, order = [];
    App.shortcuts.reduce(function (out, s) {
      var parts = s.label.split(', ');
      if (s.keys.length > 2 && parts.length === s.keys.length) return out.concat(s.keys.map(function (k, i) { return { group: s.group, keys: [k], label: parts[i] }; }));
      return out.concat([s]);
    }, []).forEach(function (s) {
      var hay = (s.group + ' ' + s.label + ' ' + s.keys.join(' ')).toLowerCase();
      if (q && q.split(/\s+/).some(function (w) { return hay.indexOf(w) < 0; })) return;
      if (!groups[s.group]) { groups[s.group] = []; order.push(s.group); }
      groups[s.group].push(s);
    });
    if (!order.length) return html`<p class="sys-nomatch">No shortcut matches "${q}". Try a key like ${ui.keycap('Q')} or a word like queue.</p>`;
    return html`<div class="sys-keys__groups">${order.map(function (g) {
      return html`<section class="sys-keys__group">${ui.labelRule(g.toLowerCase(), { level: 'h3', sm: true })}<dl class="shortcut-list">${groups[g].map(function (s) {
        return html`<div class="shortcut-row"><dt>${s.keys.map(function (k, i) { return html`${i ? html`<span class="sys-then">or</span>` : ''}<span class="sys-combo">${keysMarkup(k)}</span>`; })}</dt><dd>${s.label}</dd></div>`;
      })}</dl></section>`;
    })}</div>`;
  }
  function openShortcuts(q) {
    App.openDialog({ title: 'keyboard shortcuts', wide: true, className: 'sys-keys',
      body: html`<div class="search-field sys-keys__filter">${icon('search')}<label class="vh" for="sys-keys-q">Filter shortcuts</label><input class="input" id="sys-keys-q" type="search" placeholder="Filter, for example queue or Ctrl" autocomplete="off" spellcheck="false" value="${q || ''}" autofocus></div>
        <div class="sys-keys__list" id="sys-keys-list" aria-live="polite">${shortcutList(q)}</div>`,
      onMount: function (el) {
        var input = $('#sys-keys-q', el), body = $('.sheet__body', el);
        function more() { if (body) body.toggleAttribute('data-more', body.scrollTop + body.clientHeight < body.scrollHeight - 4); }   // fade the foot while more list is below
        if (body) body.addEventListener('scroll', more);
        input.addEventListener('input', function () { repaint('#sys-keys-list', shortcutList(input.value)); more(); });
        input.addEventListener('keydown', function (e) { if (e.key === 'Escape' && input.value) { e.stopPropagation(); input.value = ''; repaint('#sys-keys-list', shortcutList('')); more(); } });
        more();
      } });
  }
  App.hooks.shortcuts = function () { openShortcuts(''); };

  /* =====================================================================
     4. Command palette (Ctrl/Cmd+K): commands, what is already known locally, and "search for"
     ===================================================================== */
  function go(path) { return function () { App.go(path); }; }
  function commands() {
    var night = App.theme() === 'night', reduced = App.reducedMotion(), playing = P.state.playing;
    var list = [
      { label: 'Go to Home', icon: 'home', keys: ['G+H'], run: go('/home') },
      { label: 'Go to Search', icon: 'search', keys: ['/'], run: go('/search') },
      { label: 'Go to Library', icon: 'library', keys: ['G+L'], run: go('/library') },
      { label: 'Go to Playlists', icon: 'playlists', keys: ['G+P'], run: go('/playlists') },
      { label: 'Go to Stats', icon: 'stats', run: go('/stats') },
      { label: 'Go to Settings', icon: 'settings', run: go('/settings') },
      { label: 'Toggle Classic mode', icon: 'classic', keys: ['W'], run: function () { App.go(App.current.path === '/classic' ? '/home' : '/classic'); } },
      { label: night ? 'Switch to day theme' : 'Switch to night theme', icon: night ? 'sun' : 'moon', keys: ['T'], run: App.toggleTheme },
      { label: reduced ? 'Turn full motion on' : 'Reduce motion', icon: 'equaliser', run: function () { App.setPref('motion', reduced ? 'full' : 'reduce'); App.toast(reduced ? 'Full motion on' : 'Reduced motion on', { icon: 'equaliser' }); } },
      { label: playing ? 'Pause' : 'Play', icon: playing ? 'pause' : 'play', keys: ['Space'], run: function () { P.toggle(); } },
      { label: 'Next song', icon: 'next', keys: ['N'], run: function () { P.next(); } },
      { label: 'Previous song', icon: 'prev', keys: ['P'], run: function () { P.prev(); } },
      { label: 'Like the playing song', icon: 'heart', keys: ['L'], run: function () { if (P.currentId()) App.toggleLike(P.currentId()); } },
      { label: 'Save the playing song', icon: 'save', keys: ['Shift+S'], run: function () { if (P.currentId()) App.saves.start([P.currentId()]); } },
      { label: 'Start radio from the playing song', icon: 'radio', run: function () { var t = P.current(); if (t) P.startRadio({ kind: 'track', id: t.id }); } },
      { label: 'Open queue', icon: 'queue', keys: ['Q'], run: function () { App.openPanel('queue'); } },
      { label: 'Open lyrics', icon: 'lyrics', keys: ['Y'], run: function () { App.openPanel('lyrics'); } },
      { label: 'Open activity', icon: 'activity', run: function () { App.openPanel('activity'); } },
      { label: 'Keyboard shortcuts', icon: 'keyboard', keys: ['?'], run: function () { openShortcuts(''); } }
    ];
    SECTIONS.forEach(function (s) { list.push({ label: 'Settings: ' + s[1], icon: s[2], run: go('/settings/' + s[0]) }); });
    return list;
  }
  /* what the palette finds without a network search (each /api/search also starts a Soulseek search, so typing
     here never calls it): liked and queued songs, followed artists and their albums, followed tags, playlists.
     Anything else is the "Search for" row, which hands the words to /search. */
  function knownItems() {
    var followed = Object.keys(App.store.get('followed') || {});
    var trackIds = uniq(Object.keys(App.store.get('liked') || {}).concat(P.state.queue.map(function (q) { return q.trackId; })));
    var tracks = trackIds.map(M.track).filter(Boolean);
    var artistIds = uniq(followed.filter(isArtistId).concat(tracks.map(function (t) { return t.artistId; })));
    var albumIds = uniq(tracks.map(function (t) { return t.albumId; }).concat([].concat.apply([], artistIds.map(function (id) {
      return M.albumsBy(id).map(function (a) { return a.id; });
    }))));
    return { tracks: tracks, artists: artistIds.map(M.artist).filter(Boolean), albums: albumIds.map(M.album).filter(Boolean),
      tags: followed.filter(function (id) { return !isArtistId(id); }).map(M.tag), playlists: M.playlists };
  }
  function paletteResults(q) {
    var words = q.toLowerCase().split(/\s+/).filter(Boolean);
    function hit(text) { text = String(text || '').toLowerCase(); return words.every(function (w) { return text.indexOf(w) >= 0; }); }
    var groups = [];
    if (!words.length) {
      var all = commands(), pick = ['Go to Home', 'Go to Library', 'Toggle Classic mode', 'Go to Settings', 'Keyboard shortcuts'];
      groups.push({ label: 'suggested', items: all.filter(function (c) { return pick.indexOf(c.label) >= 0 || /theme$/.test(c.label); }) });
      var rec = (App.store.get('recentSearches') || []).slice(0, 3);
      if (rec.length) groups.push({ label: 'recent searches', items: rec.map(function (r) { return { label: r, icon: 'clock', sub: 'Search', run: go('/search?q=' + encodeURIComponent(r)) }; }) });
      return groups;
    }
    var k = knownItems();
    var cmds = commands().filter(function (c) { return hit(c.label); }).slice(0, 6);
    var songs = k.tracks.filter(function (t) { return hit(t.title + ' ' + t.artist); }).slice(0, 4).map(function (t) {
      return { label: t.title, sub: t.artist, cover: t, run: function () { P.playTrack(t.id); App.toast('Playing ' + t.title, { icon: 'play' }); } };
    });
    var albums = k.albums.filter(function (a) { return hit(a.title + ' ' + a.artist); }).slice(0, 3).map(function (a) { return { label: a.title, sub: 'Album by ' + a.artist, cover: a, run: go('/album/' + a.id) }; });
    var artists = k.artists.filter(function (a) { return hit(a.name); }).slice(0, 3).map(function (a) { return { label: a.name, sub: 'Artist', cover: a, round: true, run: go('/artist/' + a.id) }; });
    var playlists = k.playlists.filter(function (p) { return hit(p.name); }).slice(0, 3).map(function (p) { return { label: p.name, sub: 'Playlist', icon: 'playlists', run: go('/playlist/' + p.id) }; });
    var tags = k.tags.filter(function (t) { return hit(t.name); }).slice(0, 3).map(function (t) { return { label: t.name, sub: 'Tag', icon: 'tag', run: go('/tag/' + encodeURIComponent(t.id)) }; });
    if (cmds.length) groups.push({ label: 'commands', items: cmds });
    if (songs.length) groups.push({ label: 'songs', items: songs });
    if (albums.length || artists.length) groups.push({ label: 'albums and artists', items: albums.concat(artists) });
    if (playlists.length) groups.push({ label: 'playlists', items: playlists });
    if (tags.length) groups.push({ label: 'tags', items: tags });
    groups.push({ label: 'search', items: [{ label: 'Search for "' + q.trim() + '"', sub: 'Library and Soulseek', icon: 'search', run: go('/search?q=' + encodeURIComponent(q.trim())) }] });
    return groups;
  }
  function openPalette(q0) {
    var flat = [], active = 0;
    function render(q) {
      flat = [];
      var groups = paletteResults(q);
      return html`${groups.map(function (g, gi) {
        return html`<div class="sys-pal__group" role="group" aria-labelledby="sys-pal-g${gi}"><div class="sys-pal__glabel" id="sys-pal-g${gi}" role="presentation">${g.label}</div>${g.items.map(function (it) {
          var i = flat.push(it) - 1;
          return html`<div class="sys-pal__opt" role="option" id="sys-pal-o${i}" data-i="${i}" aria-selected="${i === 0 ? 'true' : 'false'}">
            <span class="sys-pal__ic">${it.cover ? ui.cover(it.cover, 32, { round: it.round }) : icon(it.icon || 'sparkle')}</span>
            <span class="sys-pal__text"><span class="sys-pal__label">${it.label}</span>${it.sub ? html`<span class="sys-pal__sub">${it.sub}</span>` : ''}</span>
            ${it.keys ? html`<span class="sys-pal__keys" aria-hidden="true">${keysMarkup(it.keys[0])}</span>` : ''}</div>`;
        })}</div>`;
      })}`;
    }
    return App.openDialog({ label: 'Command palette', className: 'sys-palette', closable: false,
      body: html`<div class="search-field sys-pal__field">${icon('search')}<input class="input" id="sys-pal-q" type="text" role="combobox" aria-expanded="true" aria-controls="sys-pal-list" aria-autocomplete="list" aria-activedescendant="sys-pal-o0" aria-label="Search or run a command" placeholder="Search or type a command" autocomplete="off" spellcheck="false" value="${q0 || ''}" autofocus><span class="search-field__end"><kbd class="kbd" aria-hidden="true">Esc</kbd></span></div>
        <div class="sys-pal__list" id="sys-pal-list" role="listbox" aria-label="Results">${render(q0 || '')}</div>
        <p class="sys-pal__foot" aria-hidden="true">${ui.keycap('Up')}${ui.keycap('Down')} move ${ui.keycap('Enter')} run ${ui.keycap('Esc')} close</p>`,
      onMount: function (el, close) {
        var input = $('#sys-pal-q', el), list = $('#sys-pal-list', el);
        function setActive(i) {
          if (!flat.length) return;
          active = (i + flat.length) % flat.length;
          $$('[role="option"]', list).forEach(function (o) { o.setAttribute('aria-selected', +o.getAttribute('data-i') === active ? 'true' : 'false'); });
          var o = $('#sys-pal-o' + active, list); input.setAttribute('aria-activedescendant', o.id);
          o.scrollIntoView({ block: 'nearest' });
        }
        /* the command runs once this key or click event is done, with the palette closed and focus given back */
        function run(i) { var it = flat[i]; if (!it) return; close(); Promise.resolve().then(it.run); }
        input.addEventListener('input', function () { list.innerHTML = String(render(input.value)); active = 0; input.setAttribute('aria-activedescendant', 'sys-pal-o0'); list.scrollTop = 0; });
        input.addEventListener('keydown', function (e) {
          if (e.key === 'ArrowDown') { e.preventDefault(); setActive(active + 1); }
          else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(active - 1); }
          else if (e.key === 'Enter') { e.preventDefault(); run(active); }
        });
        list.addEventListener('click', function (e) { var o = e.target.closest('[role="option"]'); if (o) run(+o.getAttribute('data-i')); });
        list.addEventListener('mousemove', function (e) { var o = e.target.closest('[role="option"]'); if (o && +o.getAttribute('data-i') !== active) setActive(+o.getAttribute('data-i')); });
      } });
  }
  App.hooks.palette = function () { if ($('.sys-palette')) App.closeTop(); else openPalette(''); };

  /* =====================================================================
     5. First-run welcome (/welcome): connect Last.fm, follow three things, try a search
     ===================================================================== */
  var welcome = { searched: false };
  /* picks come from the feed: its cold-start tags, and the artists behind its picks */
  function welcomePicks(feed) {
    var tags = uniq((feed.coldStartTags || []).concat(feed.followedTags || [])).slice(0, 6);
    var artistIds = uniq((feed.picks || []).map(function (p) { var t = M.track(p.trackId); return t && t.artistId; }));
    return { tags: tags, artists: artistIds.map(M.artist).filter(Boolean).slice(0, 4) };
  }
  function welcomeLastfm() {
    var r = M.get(LASTFM);
    if (r.loading) return ui.skeleton('chip', 1);
    if (r.error) return html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.error.message}</p><button class="btn btn--sm" data-act="reload-read" data-id="${LASTFM}">Try again</button>`;
    if (r.data.connected) return html`<p class="sys-step__done">${icon('check', null, 'ic-16')}Connected as <b>${r.data.user}</b></p>`;
    if (!r.data.configured) return html`<p class="sys-row__help">Last.fm isn't set up in the server config yet. Skip this step for now.</p>`;
    return html`${r.data.error ? html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.data.error}</p>` : ''}<button class="btn btn--sm" data-act="sys-lfm-connect">${icon('external')}${r.data.error ? 'Try again' : 'Connect Last.fm'}</button>`;
  }
  function welcomeFollowCount() {
    var feed = M.peek(FEED); if (!feed) return 0;
    var p = welcomePicks(feed);
    return p.tags.concat(p.artists.map(function (a) { return a.id; })).filter(App.isFollowed).length;
  }
  function stepState() { var l = M.peek(LASTFM); return [!!(l && l.connected), welcomeFollowCount() >= 3, welcome.searched]; }
  function updateWelcomeProgress() {
    var done = stepState();
    $$('.sys-step').forEach(function (li, i) {
      li.classList.toggle('is-done', done[i]);
      $('.sys-step__n', li).innerHTML = String(done[i] ? html`${icon('check', null, 'ic-16')}<span class="vh">Done:</span>` : html`${i + 1}`);
    });
    var c = $('#sys-w-count'); if (c) c.textContent = done.filter(Boolean).length + ' of 3 done';
    var f = $('#sys-w-follow-n'); if (f) f.textContent = Math.min(3, welcomeFollowCount()) + ' of 3';
  }
  App.action('sys-w-follow', function (el) { App.toggleFollow(el.getAttribute('data-id')); });
  function welcomeFollow() {
    var r = M.get(FEED);
    if (r.loading) return ui.skeleton('chip', 6);
    if (r.error) return html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.error.message}</p><button class="btn btn--sm" data-act="reload-read" data-id="${FEED}">Try again</button>`;
    var picks = welcomePicks(r.data);
    return html`<div class="cluster sys-chips" role="group" aria-label="Tags">${picks.tags.map(function (id) { return followChip(id, M.tag(id).name, 'sys-w-follow'); })}</div>
      <div class="sys-picks" role="group" aria-label="Artists">${picks.artists.map(function (a) {
        return html`<button type="button" class="sys-pick" data-act="sys-w-follow" data-id="${a.id}" data-name="${a.name}" aria-pressed="${App.isFollowed(a.id) ? 'true' : 'false'}">${ui.cover(a, 44, { round: true })}<span>${a.name}</span><span class="sys-pick__state" aria-hidden="true">${icon('plus', null, 'ic-16 sys-pick__plus')}${icon('check', null, 'ic-16 sys-pick__check')}</span></button>`;
      })}</div>`;
  }
  function renderWelcome() {
    var done = stepState(), feed = M.peek(FEED), tries = feed ? welcomePicks(feed).tags.slice(0, 2) : [];
    function n(i) { return done[i] ? html`${icon('check', null, 'ic-16')}<span class="vh">Done:</span>` : html`${i + 1}`; }
    return html`<div class="sys-page sys-welcome">
      ${ui.sparkle('--s:22px; right:6%; top:18px')}
      <header class="sys-welcome__head"><h1 class="t-display">welcome to lull</h1>
        <p class="sys-lede">Everything you play streams from Soulseek peers, and anything you save lands in your Jellyfin library. Three steps make Home yours.</p></header>
      <section class="card sys-welcome__card" aria-labelledby="sys-w-h">
        ${ui.labelRule('get started', { id: 'sys-w-h' })}
        <ol class="sys-steps" id="sys-welcome-steps">
          <li class="sys-step ${done[0] ? 'is-done' : ''}"><span class="sys-step__n">${n(0)}</span><div class="sys-step__body">
            <h3 class="sys-step__title">Connect Last.fm</h3><p class="sys-step__text">Scrobble what you play here, and let years of history seed your recommendations.</p>
            <div class="sys-step__act" id="sys-w-lastfm">${welcomeLastfm()}</div></div></li>
          <li class="sys-step ${done[1] ? 'is-done' : ''}"><span class="sys-step__n">${n(1)}</span><div class="sys-step__body">
            <h3 class="sys-step__title">Follow a few artists or tags <span class="sys-step__count num" id="sys-w-follow-n">${Math.min(3, welcomeFollowCount())} of 3</span></h3><p class="sys-step__text">Home fills with new releases and picks from what you follow.</p>
            <div class="sys-step__act">${welcomeFollow()}</div></div></li>
          <li class="sys-step ${done[2] ? 'is-done' : ''}"><span class="sys-step__n">${n(2)}</span><div class="sys-step__body">
            <h3 class="sys-step__title">Try a search</h3><p class="sys-step__text">Searches cover your library and Soulseek at once. Results stream in as peers answer.</p>
            <form class="sys-step__act sys-w-search" id="sys-w-search" role="search"><div class="search-field">${icon('search')}<label class="vh" for="sys-w-q">Search songs, albums, artists</label><input class="input" id="sys-w-q" type="search" placeholder="Try an artist or a song" autocomplete="off"></div><button class="btn btn--primary" type="submit">Search</button></form>
            ${tries.length ? html`<p class="sys-step__try">Or try ${tries.map(function (s, i) { return html`${i ? ' or ' : ''}<button type="button" class="btn btn--quiet btn--sm" data-act="sys-w-try" data-q="${s}">${s}</button>`; })}</p>` : ''}</div></li>
        </ol>
        <footer class="sys-welcome__foot"><span class="fg-2" id="sys-w-count" aria-live="polite">${done.filter(Boolean).length} of 3 done</span><span class="spacer"></span>
          <a class="btn btn--quiet" href="#/home">Skip for now</a><a class="btn btn--primary" href="#/home" data-act="sys-w-finish">Start listening</a></footer>
      </section></div>`;
  }
  function welcomeSearch(q) { q = (q || '').trim(); if (!q) { $('#sys-w-q').focus(); return; } welcome.searched = true; App.go('/search?q=' + encodeURIComponent(q)); }
  App.action('sys-w-try', function (el) { welcomeSearch(el.getAttribute('data-q')); });
  App.action('sys-w-finish', function () { App.go('/home'); App.toast('The welcome steps live in Settings, App', { icon: 'info' }); });
  App.route('/welcome', { render: renderWelcome, title: 'Welcome', nav: 'home', mount: function (ctx) {
    $('#sys-w-search', ctx.el).addEventListener('submit', function (e) { e.preventDefault(); welcomeSearch($('#sys-w-q').value); });
    /* follows change at once and roll back if the server refuses: mirror both on the picks and the progress */
    ctx.onCleanup(App.on('follow', function (e) {
      $$('[data-act="sys-w-follow"][data-id="' + e.id + '"]').forEach(function (b) { b.setAttribute('aria-pressed', e.followed ? 'true' : 'false'); });
      updateWelcomeProgress();
    }));
  } });

  /* =====================================================================
     6. Token gate: opens when the server answers 401 (App.server.authorized = false);
        POST /api/session sets the cookie, then App.start() connects
     ===================================================================== */
  var gate = null;
  function openTokenGate() {
    if (gate) return;
    var titleId = App.uid('sys-gate-t');
    gate = App.openDialog({ className: 'sys-gate', closable: false, dismissible: false, label: 'Unlock ' + App.product,
      onClose: function () { gate = null; },
      body: html`<form class="sys-gate__form" novalidate>
        <div class="sys-gate__mark">${ui.wordmark('desktop')}</div>
        <h2 class="sys-gate__title" id="${titleId}">this server needs a token</h2>
        <p class="sys-gate__text">Ask whoever runs this server for the access token, or find it in the server config.</p>
        <div class="field"><label class="field__label" for="sys-gate-token">Access token</label><input class="input" id="sys-gate-token" type="password" autocomplete="current-password" spellcheck="false" aria-describedby="sys-gate-err" autofocus><span class="field__error" id="sys-gate-err" hidden>${icon('warning', null, 'ic-16')}<span data-msg></span></span></div>
        <div class="sys-gate__foot"><button type="submit" class="btn btn--primary btn--block">Unlock</button></div>
      </form>`,
      onMount: function (el, close) {
        el.setAttribute('aria-labelledby', titleId); el.removeAttribute('aria-label');
        var form = $('form', el), tok = $('#sys-gate-token', el), err = $('#sys-gate-err', el), submit = $('[type="submit"]', el);
        function showErr(m) { $('[data-msg]', err).textContent = m; err.hidden = !m; tok.setAttribute('aria-invalid', m ? 'true' : 'false'); }
        form.addEventListener('submit', function (e) {
          e.preventDefault();
          var token = tok.value.trim();
          if (!token) { showErr('Enter the access token'); tok.focus(); return; }
          showErr(''); submit.disabled = true;
          M.api('/api/session', { method: 'POST', body: { token: token } }).then(function () {
            close(); App.start();
            App.toast('Unlocked', { kind: 'ok', icon: 'lock' });
          }, function (e2) {
            submit.disabled = false;
            showErr(e2.status === 401 ? 'Token rejected. Check for spaces and try again.' : e2.message);
            tok.select();
          });
        });
      } });
  }

  /* =====================================================================
     7. PWA: install (button, one-time toast), iOS instructions sheet, update toast
     ===================================================================== */
  function openIosSheet() {
    App.openSheet({ title: 'install on iPhone or iPad', className: 'sys-ios',
      body: html`<ol class="sys-ios__steps">
        <li><span class="sys-ios__ic">${icon('export', null, 'ic-24')}</span><div><b>Tap Share</b><span>The square with an arrow, in Safari's toolbar.</span></div></li>
        <li><span class="sys-ios__ic">${icon('plus', null, 'ic-24')}</span><div><b>Tap Add to Home Screen</b><span>Scroll the share sheet if you don't see it.</span></div></li>
        <li><span class="sys-ios__ic">${icon('check', null, 'ic-24')}</span><div><b>Tap Add</b><span>lull opens full screen, with controls on the lock screen.</span></div></li></ol>
        <p class="sys-note">${icon('info', null, 'ic-16')}<span>Use Safari. Other browsers on iOS can't add apps to the home screen.</span></p>`,
      foot: html`<button class="btn btn--primary" data-close>Got it</button>` });
  }
  function installApp() {
    App.pwa.install().then(function (accepted) {
      repaint('#sys-install', installBody());
      if (accepted) App.toast('lull installed. Open it from your apps for a full-screen player.', { kind: 'ok', icon: 'device' });
    });
  }
  /* offered once per device, when the browser says it can install (or on iOS, where only the sheet can help) */
  function offerInstall() {
    if (App.storage('installOffered') || App.pwa.installed()) return;
    App.storage('installOffered', true);
    App.toast('Install lull for a full-screen player with lock-screen controls', { icon: 'device', action: 'Install', sticky: true, onAction: function () {
      if (App.pwa.installable()) installApp(); else openIosSheet();
    } });
  }
  var updateToast = null;
  App.on('pwa', function (e) {
    if (e.kind === 'installable') offerInstall();
    if (e.kind === 'update' && !(updateToast && document.contains(updateToast.el))) {
      updateToast = App.toast('New version ready', { icon: 'refresh', action: 'Reload', sticky: true, onAction: App.pwa.update });
    }
  });
  App.on('ready', function () { if (isIos()) offerInstall(); });

  /* =====================================================================
     8. Banners: degraded services (store `services`), server unreachable (not dismissible), browser offline.
        Each is shown while its cause lasts; one the listener dismissed stays away until its state changes.
     ===================================================================== */
  function retryScrobbles() {
    M.api('/api/lastfm/retry', { method: 'POST' }).then(function () { App.toast('Sending queued scrobbles to Last.fm', { kind: 'ok', icon: 'check' }); },
      function (err) { App.toast('Couldn\'t send scrobbles: ' + err.message, { kind: 'error' }); });
  }
  /* the banners the current state calls for; `sig` is what a dismissal is remembered against */
  function wantedBanners() {
    var s = App.store.get('services'), out = [];
    var sk = s.soulseek, jf = s.jellyfin, lf = s.lastfm, ca = s.cache;
    if (sk.state === 'down' || sk.state === 'warn') {
      out.push({ id: 'sys-soulseek', sig: sk.state, kind: sk.state, icon: 'soulseek', text: 'Soulseek: ' + clause(sk.detail) + '. Saved songs still play.', action: { label: 'Check again', run: checkHealth } });
    }
    if (jf.state === 'down') {
      out.push({ id: 'sys-jellyfin', sig: jf.state, kind: 'warn', icon: 'warning', text: 'Jellyfin isn\'t responding. Your library is read-only and saving waits until it\'s back.', action: { label: 'Retry', run: checkHealth } });
    }
    if (lf.state === 'down') {
      out.push({ id: 'sys-lastfm', sig: lf.state, kind: 'warn', icon: 'warning', text: 'Last.fm is down.' + (lf.queued ? ' Scrobbles are queued (' + lf.queued + ') and will retry.' : ''), action: { label: 'Retry now', run: retryScrobbles } });
    }
    if (ca.state === 'warn' || ca.state === 'down') {
      out.push({ id: 'sys-cache', sig: ca.state, kind: 'warn', icon: 'warning', text: ca.state === 'warn' ? 'Cache full and every item is in use, so playback may pause.' : 'Cache: ' + clause(ca.detail) + '.', action: { label: 'Cache settings', run: go('/settings/cache') } });
    }
    if (App.server.reachable === false) {
      out.push({ id: 'sys-unreachable', sig: 'down', kind: 'down', icon: 'offline', dismissible: false, text: 'Can\'t reach your server. What has already loaded keeps playing.', action: { label: 'Retry now', run: checkHealth } });
    }
    if (navigator.onLine === false) {
      out.push({ id: 'sys-offline', sig: 'offline', kind: 'warn', icon: 'offline', text: 'You\'re offline. Saved and cached songs still play; search and streaming wait for the connection.' });
    }
    return out;
  }
  var shown = {}, dismissed = {};                  // id -> { sig, text } this file put up / sig the listener closed
  var BACK = { 'sys-unreachable': 'Connected to your server again', 'sys-offline': 'Back online' };
  function syncBanners() {
    var open = App.banner.list().map(function (b) { return b.id; }), want = wantedBanners(), next = {};
    Object.keys(shown).forEach(function (id) { if (open.indexOf(id) < 0) dismissed[id] = shown[id].sig; });
    want.forEach(function (b) {
      if (dismissed[b.id] === b.sig) { next[b.id] = shown[b.id]; return; }
      delete dismissed[b.id];
      /* re-show only on a change, so live regions don't re-announce on every service event */
      if (!shown[b.id] || shown[b.id].text !== b.text || open.indexOf(b.id) < 0) App.banner.show(b);
      next[b.id] = { sig: b.sig, text: b.text };
    });
    Object.keys(shown).forEach(function (id) {
      if (next[id]) return;
      delete dismissed[id];
      if (open.indexOf(id) >= 0) App.banner.hide(id);
      if (BACK[id]) App.toast(BACK[id], { kind: 'ok', icon: 'check' });
    });
    shown = next;
    setSearchPaused(App.server.reachable === false);
  }
  /* search needs the server: the header field says so while it is unreachable */
  function setSearchPaused(on) {
    var s = $('#header-search'); if (!s) return;
    if (!s.hasAttribute('data-placeholder')) s.setAttribute('data-placeholder', s.placeholder);
    s.disabled = on; s.placeholder = on ? 'Search pauses until the server is back' : s.getAttribute('data-placeholder');
  }
  App.store.subscribe(function (keys) { if (keys.indexOf('services') >= 0) syncBanners(); });
  App.on('server', function (srv) {
    if (srv.authorized === false) openTokenGate(); else if (gate) gate.close();
    syncBanners();
  });
  App.on('ready', syncBanners);
  App.on('design', function () { setSearchPaused(App.server.reachable === false); });   // the header was re-rendered
  window.addEventListener('offline', syncBanners);
  window.addEventListener('online', syncBanners);
})();
