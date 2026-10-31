/* screens/system.js.
   Stats (/stats), Settings (/settings/:section?), first-run welcome (/welcome), the shortcuts overlay (?),
   the command palette (Ctrl/Cmd+K), the token gate + desktop connect dialog, PWA install / update,
   the degraded-service banners demo and the prototype-only state switcher.
   Sections: 0 helpers + generic controls, 1 stats, 2 settings, 3 shortcuts overlay, 4 command palette,
   5 welcome, 6 token gate, 7 PWA, 8 banners, 9 states, 10 state switcher. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var P = App.player;

  /* =====================================================================
     0. Prototype state (in memory) and helpers
     ===================================================================== */
  function copy(o) { return JSON.parse(JSON.stringify(o)); }
  var cfg = copy(M.settings);
  cfg.playback.loudnessTarget = -14;
  cfg.sources.formatOrder = ['FLAC 16/44.1', 'FLAC 24/96', 'MP3 320', 'MP3 V0', 'OPUS 160'];
  cfg.sources.formatPreset = 'lossless';
  cfg.lastfm.state = 'connected';                // off | waiting | connected | error
  cfg.app.install = detectInstall();             // available | ios | installed
  cfg.appearance.defaultSkin = App.storage('defaultSkin') || cfg.appearance.defaultSkin;
  /* desktop wrapper options only show inside the wrapper (the prototype previews them with ?titlebar=win|gnome) */
  function inWrapper() { return !!window.__TAURI__ || !!(App.current.query && App.current.query.titlebar); }
  if (window.__TAURI__ || /[?&]titlebar=/.test(location.hash)) App.shortcuts.push({ group: 'Panels', keys: ['Ctrl+Shift+M'], label: 'Mini player (desktop app)' });

  function num(x) { return Number(x).toLocaleString('en'); }
  function hours(min) {
    var h = Math.floor(min / 60), m = Math.round(min % 60);
    return h ? num(h) + ' h' + (m && h < 100 ? ' ' + m + ' min' : '') : m + ' min';
  }
  function pct(x) { return Math.round(x * 100) + '%'; }
  function getCfg(path) { return path.split('.').reduce(function (o, k) { return o == null ? o : o[k]; }, cfg); }
  function setCfg(path, v) {
    var ks = path.split('.'), o = cfg;
    for (var i = 0; i < ks.length - 1; i++) o = o[ks[i]];
    var old = o[ks[ks.length - 1]];
    o[ks[ks.length - 1]] = typeof old === 'number' ? +v : v;
    if (effects[path]) effects[path](o[ks[ks.length - 1]]);
  }
  /* replace a part of the page and put focus back on an equivalent control */
  function repaint(sel, markup, focusSel) {
    var el = $(sel); if (!el) return;
    el.innerHTML = String(markup);
    if (focusSel) { var f = $(focusSel, el); if (f) f.focus(); }
  }

  /* ---------- generic controls. Each writes to cfg by its data-key path; side effects live in `effects`. ---------- */
  var effects = {};
  var fmtRange = {};                              // path -> fn(value) -> visible text / aria-valuetext

  /* one-of-N choice: a segmented pill with role=radiogroup; arrows move and select */
  function choice(path, label, options, cls) {
    var cur = String(getCfg(path)), i = options.map(function (o) { return String(o[0]); }).indexOf(cur);
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
    setCfg(group.getAttribute('data-key'), btn.getAttribute('data-value'));
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

  /* switch row: the whole row is the label; help text is the description */
  function switchRow(path, label, help, opts) {
    opts = opts || {};
    var id = App.uid('sys-sw'), on = !!getCfg(path);
    return html`<label class="switch-row sys-row ${opts.disabled ? 'is-disabled' : ''}">
      <span class="sys-row__text"><span class="sys-row__label" id="${id}">${label}${opts.status || ''}</span>${help ? html`<span class="sys-row__help" id="${id}-h">${help}</span>` : ''}</span>
      <button type="button" class="switch" role="switch" aria-checked="${on ? 'true' : 'false'}" aria-labelledby="${id}" ${help ? raw('aria-describedby="' + id + '-h"') : ''} data-switch="${path}" ${opts.disabled ? raw('aria-disabled="true" data-locked-msg="' + App.esc(opts.disabled) + '"') : ''}></button></label>`;
  }
  document.addEventListener('click', function (e) {
    var s = e.target.closest && e.target.closest('[data-switch]'); if (!s) return;
    if (s.getAttribute('aria-disabled') === 'true') { App.toast(s.getAttribute('data-locked-msg') || 'This one is set in server config', { icon: 'lock' }); return; }
    var on = s.getAttribute('aria-checked') !== 'true';
    s.setAttribute('aria-checked', on ? 'true' : 'false');
    setCfg(s.getAttribute('data-switch'), on);
  });

  /* range with a visible value; arrows work because role=slider stops the player's seek shortcut */
  function range(path, label, min, max, step) {
    var v = getCfg(path), text = fmtRange[path] ? fmtRange[path](v) : String(v);
    return html`<div class="sys-range"><input class="range" type="range" role="slider" min="${min}" max="${max}" step="${step}" value="${v}" style="--fill:${(v - min) / (max - min) * 100}%" aria-label="${label}" aria-valuetext="${text}" data-range="${path}"><output class="sys-range__out num" data-out="${path}" aria-hidden="true">${text}</output></div>`;
  }
  document.addEventListener('input', function (e) {
    var r = e.target.closest && e.target.closest('[data-range]'); if (!r) return;
    var path = r.getAttribute('data-range'), v = +r.value;
    r.style.setProperty('--fill', (v - r.min) / (r.max - r.min) * 100 + '%');
    setCfg(path, v);
    var text = fmtRange[path] ? fmtRange[path](v) : String(v);
    r.setAttribute('aria-valuetext', text);
    $$('[data-out="' + path + '"]').forEach(function (o) { o.textContent = text; });
  });

  /* stepper: minus / value / plus */
  function stepper(path, label, min, max, step, unit) {
    var v = getCfg(path);
    return html`<div class="sys-stepper" role="group" aria-label="${label}" data-stepper="${path}" data-min="${min}" data-max="${max}" data-step="${step}" data-unit="${unit || ''}">
      <button type="button" class="btn-icon btn-icon--sm" data-dir="-1" aria-label="Decrease ${label.toLowerCase()}" ${v <= min ? raw('disabled') : ''}>${icon('minus', null, 'ic-16')}</button>
      <output class="sys-stepper__val num" aria-live="polite">${v}${unit ? ' ' + unit : ''}</output>
      <button type="button" class="btn-icon btn-icon--sm" data-dir="1" aria-label="Increase ${label.toLowerCase()}" ${v >= max ? raw('disabled') : ''}>${icon('plus', null, 'ic-16')}</button></div>`;
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('[data-stepper] [data-dir]'); if (!b) return;
    var g = b.closest('[data-stepper]'), path = g.getAttribute('data-stepper');
    var min = +g.getAttribute('data-min'), max = +g.getAttribute('data-max'), unit = g.getAttribute('data-unit');
    var v = App.clamp(getCfg(path) + (+b.getAttribute('data-dir')) * +g.getAttribute('data-step'), min, max);
    setCfg(path, v);
    $('output', g).textContent = v + (unit ? ' ' + unit : '');
    var btns = $$('[data-dir]', g); btns[0].disabled = v <= min; btns[1].disabled = v >= max;
    if (b.disabled) btns[b === btns[0] ? 1 : 0].focus();
  });

  /* a settings row: label + help left, control right */
  function row(label, help, control, opts) {
    opts = opts || {};
    return html`<div class="sys-row ${opts.cls || ''}" ${opts.when ? raw('data-when="' + opts.when + '"') : ''} ${opts.hidden ? raw('hidden') : ''}>
      <div class="sys-row__text"><span class="sys-row__label">${label}</span>${help ? html`<span class="sys-row__help">${help}</span>` : ''}</div>
      <div class="sys-row__ctl">${control}</div></div>`;
  }
  /* config-sourced value: read-only, lock icon, never editable; secrets are never shown */
  function locked(label, value, help) {
    return row(label, help || 'Set in server config. Change it there and restart the server.',
      html`<span class="input input--readonly sys-locked" tabindex="0" aria-label="${label}: ${value || 'hidden'}. Set in server config, read only." data-tip="Set in server config">${icon('lock', null, 'ic-16')}<span class="ellipsis">${value || 'hidden'}</span></span>`, { cls: 'sys-row--locked' });
  }
  /* follow toggle chip; the check is always in the markup and CSS shows it only when pressed (.chip__check) */
  function followChip(id, name, act) {
    return html`<button type="button" class="chip" data-act="${act || 'follow-tag'}" data-id="${id}" data-name="${name}" aria-pressed="${App.isFollowed(id) ? 'true' : 'false'}">${icon('check', null, 'chip__check')}${name}</button>`;
  }
  function group(title) { return html`<div class="sys-group">${ui.labelRule(title, { level: 'h3', sm: true })}</div>`; }
  function statusLine(st, text) { return html`<span class="sys-status sys-status--${st}">${ui.statusGlyph(st, st === 'ok' ? 'OK' : st === 'warn' ? 'Warning' : 'Down')}<span>${text}</span></span>`; }

  /* =====================================================================
     1. Stats (/stats?period=7d|30d|12m|all)
     ===================================================================== */
  var PERIODS = [['7d', '7 days', 'in the last 7 days', 1], ['30d', '30 days', 'in the last 30 days', 30 / 7], ['12m', '12 months', 'in the last 12 months', 52], ['all', 'All time', 'since you started', 76]];
  /* weekday rows of the listening clock: [short, long, factor, hour shift]; the mock only has one 24-hour curve */
  var DAYS = [['Mon', 'Monday', .8, 0], ['Tue', 'Tuesday', .85, 0], ['Wed', 'Wednesday', .9, 0], ['Thu', 'Thursday', .95, 0], ['Fri', 'Friday', 1.15, 1], ['Sat', 'Saturday', 1.3, 1], ['Sun', 'Sunday', 1.05, 0]];
  var HEAT = ['--lavender', '--lav', '--peri', '--peri-deep', '--indigo'];
  var statsUi = { period: '7d', focusTab: false, reset: null };
  var taste = copy(M.stats.taste);

  function periodOf(id) { return PERIODS.filter(function (p) { return p[0] === id; })[0] || PERIODS[0]; }
  function hh(h) { return (h < 10 ? '0' : '') + h + ':00'; }
  function heatCells(d, per) {
    var cells = [], max = 0, total = 0;
    DAYS.forEach(function (day, r) {
      for (var h = 0; h < 24; h++) {
        var v = d.clock[(h - day[3] + 24) % 24] * day[2];
        cells.push({ r: r, h: h, v: v }); max = Math.max(max, v); total += v;
      }
    });
    cells.forEach(function (c) {
      c.min = Math.round(c.v / total * d.minutes / per[3]);
      c.level = c.v < max * .12 ? 0 : Math.min(4, Math.ceil(c.v / max * 4));
    });
    return { cells: cells, max: max };
  }
  function peakRange(clock) {
    var best = 0, at = 0;
    for (var h = 0; h < 24; h++) { var s = clock[h] + clock[(h + 1) % 24]; if (s > best) { best = s; at = h; } }
    return hh(at) + ' and ' + hh((at + 2) % 24);
  }
  function bar(value, max, label) {
    return html`<span class="sys-bar" aria-hidden="true"><i style="--p:${Math.max(.02, value / max)}"></i></span>${label ? html`<span class="vh">${label}</span>` : ''}`;
  }

  function statsTabs(cur, disabled) {
    var i = PERIODS.map(function (p) { return p[0]; }).indexOf(cur);
    return html`<div class="seg sys-period" role="tablist" aria-label="Period" style="--n:4;--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${PERIODS.map(function (p, j) {
      return html`<button type="button" class="seg__btn" role="tab" id="sys-tab-${p[0]}" aria-selected="${j === i ? 'true' : 'false'}" aria-controls="sys-stats-panel" tabindex="${j === i ? '0' : '-1'}" data-period="${p[0]}" ${disabled ? raw('aria-disabled="true"') : ''}>${p[1]}</button>`;
    })}</div>`;
  }
  function tile(ic, label, value, sub) {
    return html`<div class="card sys-tile"><span class="sys-tile__label">${icon(ic, null, 'ic-16')}${label}</span><span class="sys-tile__value num">${value}</span><span class="sys-tile__sub">${sub}</span></div>`;
  }

  function renderStats(ctx) {
    var per = periodOf(ctx.query.period || statsUi.period), d = M.stats.byPeriod[per[0]];
    statsUi.period = per[0];
    var head = function (lede, disabled) {
      return html`<header class="sys-head"><div><h1 class="t-display">stats</h1><p class="sys-lede">${lede}</p></div>${statsTabs(per[0], disabled)}</header>`;
    };
    /* PROTO: demo state */ if (ctx.state === 'system-stats-loading') {
      return html`<div class="sys-page sys-stats">${head('Counting your plays', true)}<div id="sys-stats-panel" role="tabpanel" aria-labelledby="sys-tab-${per[0]}" aria-busy="true">
        <div class="sys-tiles">${[1, 2, 3, 4, 5].map(function () { return raw('<div class="card sys-tile" aria-hidden="true"><span class="skel skel--text" style="width:60%"></span><span class="skel skel--title" style="width:45%;height:26px;margin-top:8px"></span></div>'); })}</div>
        <div class="sys-grid">${ui.skeleton('card', 2)}</div></div></div>`;
    }
    /* PROTO: demo state */ if (ctx.state === 'system-stats-empty') {
      return html`<div class="sys-page sys-stats">${head('Stats fill in once you have listened for a while.', true)}<div id="sys-stats-panel" role="tabpanel" aria-labelledby="sys-tab-${per[0]}">
        <div class="card sys-empty-stats">${ui.emptyState({ art: 'wave', title: 'Not enough listening yet', text: 'Stats and your taste profile need about two hours of music. Play something and check back.', action: { label: 'Find something to play', href: '#/home' }, secondary: { label: 'Start Velvet Orchard radio', act: 'start-radio', id: 'ar-velvet-orchard' } })}
          <div class="sys-empty-stats__progress"><div class="sys-meter-label"><span>26 min of 2 h</span><span class="fg-2">22%</span></div><div class="progress" role="progressbar" aria-label="Listening needed for stats" aria-valuenow="22" aria-valuemin="0" aria-valuemax="100" style="--p:.22"><i></i></div></div></div>
      </div></div>`;
    }
    var maxA = d.topArtists[0][1], maxT = d.topTracks[0][1], maxG = d.topTags[0][1];
    var heat = heatCells(d, per), daysIn = per[0] === '7d' ? 7 : per[0] === '30d' ? 30 : per[0] === '12m' ? 365 : 532;
    var hr = d.hitRate, likedP = hr.liked / hr.recommended, savedP = hr.saved / hr.recommended;
    var newArtists = M.artists.slice(6, 9);

    return html`<div class="sys-page sys-stats">
      ${head(hours(d.minutes) + ' across ' + num(d.plays) + ' plays ' + per[2] + ', mostly between ' + peakRange(d.clock) + '.')}
      <div id="sys-stats-panel" role="tabpanel" aria-labelledby="sys-tab-${per[0]}">
      <div class="sys-tiles">
        ${tile('clock', 'Listening', (d.minutes / 60 < 100 ? (d.minutes / 60).toFixed(1) : num(Math.round(d.minutes / 60))) + ' h', 'about ' + hours(Math.round(d.minutes / daysIn)) + ' a day')}
        ${tile('play', 'Plays', num(d.plays), num(Math.round(d.plays / daysIn)) + ' a day')}
        ${tile('peer', 'Artists', num(d.artists), num(d.newArtists) + ' new to you')}
        ${tile('saved', 'Saves', num(d.saves), 'to your library')}
        ${tile('next', 'Skip rate', pct(d.skipRate), 'skipped before 30 s')}
      </div>
      <div class="sys-grid">
        <section class="card sys-card" aria-labelledby="sys-h-artists">${ui.labelRule('top artists', { id: 'sys-h-artists', action: { label: 'Library', href: '#/library/artists' } })}
          <ol class="sys-bars">${d.topArtists.map(function (x, i) {
            var a = M.artist(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(a, 40, { round: true })}
              <span class="sys-bars__main"><a class="sys-bars__name" href="#/artist/${a.id}">${a.name}</a>${bar(x[1], maxA)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card" aria-labelledby="sys-h-tracks">${ui.labelRule('top songs', { id: 'sys-h-tracks', action: { label: 'Play all', act: 'sys-play-top' } })}
          <ol class="sys-bars" data-ids="${d.topTracks.map(function (x) { return x[0]; }).join(',')}">${d.topTracks.map(function (x, i) {
            var t = M.track(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(t, 40)}
              <span class="sys-bars__main"><button class="sys-bars__name" data-act="play-track" data-id="${t.id}" aria-label="Play ${t.title} by ${t.artist}">${t.title}</button><span class="sys-bars__by"><a href="#/artist/${t.artistId}">${t.artist}</a></span>${bar(x[1], maxT)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card sys-span" aria-labelledby="sys-h-clock">${ui.labelRule('listening clock', { id: 'sys-h-clock' })}
          <p class="sys-card__lede">When you listen, by weekday and hour. ${per[3] > 1 ? 'Average minutes per week.' : 'Minutes this week.'} Darker squares mean more listening.</p>
          <div class="sys-heat" role="img" aria-label="${'Listening clock. Most listening on ' + DAYS[heat.cells.reduce(function (a, c) { return c.v > a.v ? c : a; }).r][1] + 's between ' + peakRange(d.clock) + '. Least between 03:00 and 07:00.'}">
            <span></span>${[0, 6, 12, 18].map(function (h) { return html`<span class="sys-heat__hour" style="grid-column:${h + 2} / span 6">${hh(h)}</span>`; })}
            ${DAYS.map(function (day, r) {
              return html`<span class="sys-heat__day">${day[0]}</span>${heat.cells.slice(r * 24, r * 24 + 24).map(function (c) {
                return html`<i class="sys-heat__cell" style="--c:var(${HEAT[c.level]})" data-l="${c.level}" data-tip="${day[0]} ${hh(c.h)}: ${c.min} min"></i>`;
              })}`;
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
          <div class="inset sys-newart"><span class="sys-newart__n num">${num(d.newArtists)}</span><span>new artists ${per[2]}, including ${newArtists.map(function (a, i) { return html`${i ? (i === newArtists.length - 1 ? ' and ' : ', ') : ''}<a href="#/artist/${a.id}">${a.name}</a>`; })}.</span></div>
        </section>
        ${tasteCard()}
      </div></div></div>`;
  }

  function tasteCard() {
    var body;
    if (statsUi.reset) {
      body = ui.emptyState({ art: 'moon', title: 'Taste profile reset', text: 'Home forgot the weights and learns again from what you play, like and save next.', action: { label: 'Undo reset', act: 'sys-taste-undo' } });
    } else {
      var col = function (title, list, kind) {
        return html`<div class="sys-taste__col">${ui.labelRule(title, { level: 'h3', sm: true })}<ol class="sys-bars sys-bars--taste">${list.map(function (x) {
          var name = kind === 'artist' ? M.artist(x[0]).name : x[0];
          return html`<li class="sys-bars__row"><span class="sys-bars__main">${kind === 'artist' ? html`<a class="sys-bars__name" href="#/artist/${x[0]}">${name}</a>` : html`<a class="sys-bars__name" href="#/tag/${encodeURIComponent(x[0])}">${name}</a>`}${bar(x[1], 1)}</span>
            <span class="sys-bars__val num">${Math.round(x[1] * 100)}<span class="vh"> out of 100</span></span>
            <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="sys-taste-why" data-kind="${kind}" data-id="${x[0]}" aria-haspopup="dialog" aria-label="Why ${name} weighs ${Math.round(x[1] * 100)}" data-tip="Why this weight">${icon('info', null, 'ic-16')}</button></li>`;
        })}</ol></div>`;
      };
      body = html`<p class="sys-card__lede">This is what Home leans on. Weights come from plays, likes, saves and follows over the last 90 days; 100 is your strongest signal. Use the info buttons to see why, or turn something down.</p>
        <div class="sys-taste">${col('artists', taste.artists, 'artist')}${col('tags', taste.tags, 'tag')}</div>`;
    }
    return html`<section class="card sys-card sys-span" aria-labelledby="sys-h-taste" id="sys-taste">
      ${ui.labelRule('your taste profile', { id: 'sys-h-taste', action: statsUi.reset ? null : { label: 'Reset taste', act: 'sys-taste-reset' } })}${body}</section>`;
  }

  function mountStats(ctx) {
    var tl = $('.sys-period', ctx.el); if (!tl) return;
    function choose(btn) {
      if (btn.getAttribute('aria-disabled') === 'true') return;
      statsUi.period = btn.getAttribute('data-period'); statsUi.focusTab = true;
      /* PROTO: demo state */ var q = ctx.state ? '&state=' + ctx.state : '';
      history.replaceState(null, '', '#/stats?period=' + statsUi.period + q);
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

  App.action('sys-play-top', function () { P.playList(M.stats.byPeriod[statsUi.period].topTracks.map(function (x) { return x[0]; }), 0); App.toast('Playing your top songs', { icon: 'play' }); });
  App.action('sys-taste-why', function (el) {
    var kind = el.getAttribute('data-kind'), id = el.getAttribute('data-id');
    var list = taste[kind === 'artist' ? 'artists' : 'tags'], item = list.filter(function (x) { return x[0] === id; })[0];
    var d = M.stats.byPeriod['30d'], w = Math.round(item[1] * 100), text, name;
    if (kind === 'artist') {
      var a = M.artist(id), plays = (d.topArtists.filter(function (x) { return x[0] === id; })[0] || [0, 12])[1];
      var liked = M.tracksBy(id).filter(function (t) { return App.isLiked(t.id); }).length;
      name = a.name;
      text = num(plays) + ' plays in the last 30 days, ' + liked + ' liked song' + (liked === 1 ? '' : 's') + (App.isFollowed(id) ? ', and you follow them.' : '.') + ' Shows up in Home as "Because you like ' + a.name + '" and seeds ' + a.name + ' radio.';
    } else {
      var share = (d.topTags.filter(function (x) { return x[0] === id; })[0] || [0, 6])[1];
      name = id;
      text = share + '% of your plays in the last 30 days are tagged ' + id + (App.isFollowed(id) ? ', and you follow the tag.' : '.') + ' Home adds fresh releases and picks from this tag.';
    }
    App.popover(el, html`<div class="popover__head">${name}: ${w}</div><div class="popover__body"><p>${text}</p><p class="t-micro fg-2" style="margin-top:6px">via your listening history and ${kind === 'artist' ? 'ListenBrainz' : 'MusicBrainz tags'}</p></div>
      <div class="popover__actions"><button class="btn btn--sm" data-act="sys-taste-less" data-kind="${kind}" data-id="${id}">${icon('minus')}Less of this</button><a class="btn btn--sm" href="${kind === 'artist' ? '#/artist/' + id : '#/tag/' + encodeURIComponent(id)}">Open ${kind}</a></div>`, { label: 'Why ' + name + ' has this weight' });
  });
  App.action('sys-taste-less', function (el) {
    var kind = el.getAttribute('data-kind'), id = el.getAttribute('data-id'), list = taste[kind === 'artist' ? 'artists' : 'tags'];
    var item = list.filter(function (x) { return x[0] === id; })[0], before = item[1];
    item[1] = Math.max(.05, Math.round((item[1] - .2) * 100) / 100);
    list.sort(function (a, b) { return b[1] - a[1]; });
    App.closePopover();
    repaintTaste();
    App.toast('Turned down ' + (kind === 'artist' ? M.artist(id).name : id) + ' to ' + Math.round(item[1] * 100), { icon: 'minus', action: 'Undo', onAction: function () {
      item[1] = before; list.sort(function (a, b) { return b[1] - a[1]; }); repaintTaste();
    } });
  });
  App.action('sys-taste-reset', function () {
    App.confirm({ title: 'reset your taste profile?', text: 'Home forgets the weights for every artist and tag and relearns from what you play next. Your plays, likes, saves and follows stay as they are.', confirm: 'Reset taste', danger: true }, function () {
      statsUi.reset = copy(taste);
      taste = { artists: [], tags: [] };
      repaintTaste('[data-act="sys-taste-undo"]');
      App.toast('Taste profile reset', { icon: 'refresh', action: 'Undo', onAction: undoReset });
    });
  });
  function undoReset() { if (!statsUi.reset) return; taste = statsUi.reset; statsUi.reset = null; repaintTaste('[data-act="sys-taste-reset"]'); }
  App.action('sys-taste-undo', undoReset);
  function repaintTaste(focusSel) {
    var el = $('#sys-taste'); if (!el) return;
    el.outerHTML = String(tasteCard());
    if (focusSel) { var f = $('#sys-taste ' + focusSel); if (f) f.focus(); }
  }

  App.route('/stats', { render: renderStats, mount: mountStats, nav: 'stats', title: 'Stats' });

  /* =====================================================================
     2. Settings (/settings/:section?)
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

  /* ---------- playback ---------- */
  fmtRange['playback.crossfade'] = function (v) { return v ? v + ' s' : 'Off'; };
  effects['playback.crossfade'] = function (v) { App.store.set('crossfadeSec', v); };
  effects['playback.normalisation'] = function (v) { $$('[data-when="loudness"]').forEach(function (r) { r.hidden = v !== 'loudness'; }); };
  function secPlayback() {
    return html`
      ${switchRow('playback.gapless', 'Gapless playback', 'Where the source allows. Lossy files from Soulseek may still have a short gap.')}
      ${row('Crossfade', 'Blend the end of one song into the next. Albums marked gapless never crossfade.', range('playback.crossfade', 'Crossfade, seconds', 0, 12, 1))}
      ${row('Loudness normalisation', 'ReplayGain reads tags in the file. Loudness measures files that have no tags.', choice('playback.normalisation', 'Loudness normalisation', [['off', 'Off'], ['replaygain-track', 'Track'], ['replaygain-album', 'Album'], ['loudness', 'Loudness']], 'sys-seg--4'))}
      ${row('Target loudness', 'Quieter targets leave more headroom for dynamic music.', choice('playback.loudnessTarget', 'Target loudness', [[-18, '-18 LUFS'], [-14, '-14 LUFS'], [-11, '-11 LUFS']]), { when: 'loudness', hidden: cfg.playback.normalisation !== 'loudness', cls: 'sys-row--sub' })}
      ${row('Volume step', 'How far Up, Down and the D-pad move the volume.', choice('playback.volumeStep', 'Volume step', [[2, '2%'], [5, '5%'], [10, '10%']]))}
      ${switchRow('playback.radioWhenQueueEnds', 'Continue with radio when the queue ends', 'Starts a radio from the last song so the music keeps going.')}`;
  }

  /* ---------- sources ---------- */
  var PRESETS = {
    lossless: ['FLAC 16/44.1', 'FLAC 24/96', 'MP3 320', 'MP3 V0', 'OPUS 160'],
    bitrate: ['MP3 320', 'MP3 V0', 'OPUS 160', 'FLAC 16/44.1', 'FLAC 24/96'],
    small: ['OPUS 160', 'MP3 V0', 'MP3 320', 'FLAC 16/44.1', 'FLAC 24/96']
  };
  fmtRange['sources.stallTimeout'] = function (v) { return v + ' s'; };
  effects['sources.formatPreset'] = function (v) { cfg.sources.formatOrder = PRESETS[v].slice(); repaint('#sys-order', formatItems()); };
  effects['sources.autoPick'] = function (v) { App.announce(v ? 'Best source is picked automatically' : 'You will choose the source each time'); };
  function formatItems() {
    var o = cfg.sources.formatOrder;
    return html`${o.map(function (f, i) {
      var fm = M.formats[f];
      return html`<li class="sys-order__item" data-format="${f}"><span class="sys-order__n num">${i + 1}</span>${ui.qualityChip(f)}<span class="sys-order__note">${fm.detail}, about ${Math.round(fm.mbPerMin * 4)} MB a song</span>
        <span class="sys-order__btns"><button type="button" class="btn-icon btn-icon--sm btn-icon--bare" data-act="sys-format-move" data-dir="-1" aria-label="Move ${f} up" data-tip="Move up" ${i === 0 ? raw('disabled') : ''}>${icon('chevron-up', null, 'ic-16')}</button><button type="button" class="btn-icon btn-icon--sm btn-icon--bare" data-act="sys-format-move" data-dir="1" aria-label="Move ${f} down" data-tip="Move down" ${i === o.length - 1 ? raw('disabled') : ''}>${icon('chevron-down', null, 'ic-16')}</button></span></li>`;
    })}`;
  }
  App.action('sys-format-move', function (el) {
    var f = el.closest('[data-format]').getAttribute('data-format'), dir = +el.getAttribute('data-dir'), o = cfg.sources.formatOrder;
    var i = o.indexOf(f), j = i + dir; if (j < 0 || j >= o.length) return;
    o.splice(i, 1); o.splice(j, 0, f);
    var preset = Object.keys(PRESETS).filter(function (k) { return PRESETS[k].join() === o.join(); })[0];
    cfg.sources.formatPreset = preset || 'custom'; showChoice('sources.formatPreset', cfg.sources.formatPreset);
    var at = j === 0 || j === o.length - 1 ? -dir : dir;
    repaint('#sys-order', formatItems(), '[data-format="' + f + '"] [data-dir="' + at + '"]');
    App.announce(f + ' moved to position ' + (j + 1));
  });
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
      ${group('soulseek account')}
      ${locked('Username', 'night-owl-radio')}
      ${locked('Password', '')}`;
  }

  /* ---------- cache ---------- */
  fmtRange['cache.capGB'] = function (v) { return v + ' GB'; };
  effects['cache.capGB'] = function () { repaintUsage(); };
  function usageBar() {
    var c = cfg.cache, b = c.breakdown, cap = c.capGB, used = Math.min(cap, c.usedGB);
    var full = used >= cap - .05;
    var segs = [['playing', 'Playing and queued', b.playing], ['recent', 'Recently played', b.recent], ['evictable', 'Can be evicted', b.evictable]];
    return html`<div class="sys-usage__head"><span class="sys-usage__big num">${used.toFixed(1)} <span class="fg-2">of ${cap} GB used</span></span>${full ? statusLine('warn', 'Full') : statusLine('ok', (cap - used).toFixed(1) + ' GB free')}</div>
      <div class="sys-usage__bar" role="img" aria-label="${'Cache: ' + segs.map(function (s) { return s[1] + ' ' + s[2].toFixed(1) + ' GB'; }).join(', ') + ', ' + Math.max(0, cap - used).toFixed(1) + ' GB free'}">${segs.map(function (s) {
        return html`<i class="sys-usage__seg sys-usage__seg--${s[0]}" style="--w:${s[2] / cap * 100}%"></i>`;
      })}</div>
      <ul class="sys-usage__legend">${segs.map(function (s) { return html`<li><i class="sys-usage__seg sys-usage__seg--${s[0]}" aria-hidden="true"></i>${s[1]}<span class="num">${s[2].toFixed(1)} GB</span></li>`; })}<li><i class="sys-usage__seg sys-usage__seg--free" aria-hidden="true"></i>Free<span class="num">${Math.max(0, cap - used).toFixed(1)} GB</span></li></ul>
      ${full ? html`<p class="sys-note sys-note--warn">${icon('warning', null, 'ic-16')}<span>Cache full and every item is in use, so playback may pause. Raise the size or clear the cache.</span></p>` : ''}`;
  }
  function repaintUsage() { repaint('#sys-usage', usageBar()); }
  App.action('sys-cache-clear', function () {
    App.confirm({ title: 'clear the cache?', text: 'This removes ' + (cfg.cache.usedGB - cfg.cache.breakdown.playing).toFixed(1) + ' GB of streamed audio. Your saved library is not touched, and the song that is playing stays.', confirm: 'Clear cache', danger: true }, function () {
      var c = cfg.cache, freed = c.usedGB - c.breakdown.playing;
      c.usedGB = c.breakdown.playing; c.breakdown.recent = 0; c.breakdown.evictable = 0;
      App.setService('cache', { state: 'ok', usedGB: c.usedGB, capGB: c.capGB, detail: c.usedGB.toFixed(1) + ' of ' + c.capGB + ' GB used' });
      App.banner.hide('sys-cache'); repaintUsage();
      App.toast('Cleared ' + freed.toFixed(1) + ' GB from the cache', { kind: 'ok', icon: 'trash' });
    });
  });
  function secCache() {
    return html`<div class="sys-usage" id="sys-usage">${usageBar()}</div>
      ${row('Cache size', 'Streamed audio is kept up to this size so replays start instantly.', range('cache.capGB', 'Cache size, gigabytes', 1, 50, 1))}
      ${row('Clear cache', 'Removes streamed audio only. Your saved library is not touched.', html`<button class="btn" data-act="sys-cache-clear">${icon('trash')}Clear cache</button>`)}
      <p class="sys-note">${icon('info', null, 'ic-16')}<span>When the cache is full, the oldest audio that is not playing or queued goes first. Songs in your queue are always kept.</span></p>`;
  }

  /* ---------- library and saving ---------- */
  var TOKENS = ['artist', 'album', 'year', 'disc', 'track:02', 'title', 'ext'];
  function namingPreview(tpl) {
    var t = M.track('t-lantern-season-1'), al = M.album(t.albumId), f = M.formats[t.format];
    var vals = { artist: t.artist, album: t.album, year: al.year, disc: t.disc || 1, 'track:02': String(t.no).padStart(2, '0'), track: t.no, title: t.title, ext: f.codec === 'OPUS' ? 'opus' : f.codec.toLowerCase() };
    var bad = [];
    var out = tpl.replace(/\{([^}]*)\}/g, function (m, k) { if (k in vals) return vals[k]; bad.push(m); return m; });
    return { path: out, bad: bad };
  }
  function namingOut() {
    var p = namingPreview(cfg.library.naming);
    return html`${p.bad.length ? html`<span class="field__error" id="sys-naming-err">${icon('warning', null, 'ic-16')}Unknown ${p.bad.length > 1 ? 'tokens' : 'token'} ${p.bad.join(', ')}. Use the buttons below.</span>` : ''}
      <div class="inset sys-preview"><span class="sys-preview__label">Saves as</span><span class="sys-preview__path">${cfg.library.musicFolder}/${p.path}</span></div>`;
  }
  App.action('sys-token-insert', function (el) {
    var input = $('#sys-naming'), tok = '{' + el.getAttribute('data-token') + '}';
    var s = input.selectionStart == null ? input.value.length : input.selectionStart, e = input.selectionEnd == null ? s : input.selectionEnd;
    input.value = input.value.slice(0, s) + tok + input.value.slice(e);
    input.focus(); input.setSelectionRange(s + tok.length, s + tok.length);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  document.addEventListener('input', function (e) {
    if (e.target.id !== 'sys-naming') return;
    cfg.library.naming = e.target.value;
    var bad = namingPreview(cfg.library.naming).bad.length;
    e.target.setAttribute('aria-invalid', bad ? 'true' : 'false');
    repaint('#sys-naming-out', namingOut());
  });
  App.action('sys-naming-reset', function () { var i = $('#sys-naming'); i.value = M.settings.library.naming; i.dispatchEvent(new Event('input', { bubbles: true })); i.focus(); });
  App.action('sys-jellyfin-test', function (el) {
    var out = $('#sys-jf-status');
    el.disabled = true; out.innerHTML = String(html`<span class="sys-status"><span class="spin-ring" aria-hidden="true"></span><span>Testing connection</span></span>`);
    /* PROTO: simulated delay */ setTimeout(function () {
      el.disabled = false;
      var down = App.store.get('services').jellyfin.state === 'down';
      out.innerHTML = String(down ? statusLine('down', 'No answer from Jellyfin after 5 s') : statusLine('ok', 'Connected in 42 ms, 1,204 songs'));
      App.announce(down ? 'Jellyfin did not answer' : 'Jellyfin connected');
    }, 1200);
  });
  App.action('sys-rescan', function (el) {
    var box = $('#sys-rescan'); el.disabled = true; var n = 0;
    /* PROTO: simulated delay */ var t = setInterval(function () {
      n = Math.min(1204, n + 301);
      box.innerHTML = String(html`<div class="sys-meter-label"><span>Scanning ${num(n)} of 1,204 files</span></div><div class="progress" role="progressbar" aria-label="Rescan" aria-valuenow="${Math.round(n / 12.04)}" aria-valuemin="0" aria-valuemax="100" style="--p:${n / 1204}"><i></i></div>`);
      if (n >= 1204) { clearInterval(t); el.disabled = false; box.innerHTML = String(statusLine('ok', 'Rescanned just now, 3 new songs found')); App.toast('Library rescan finished: 3 new songs', { kind: 'ok', icon: 'refresh' }); }
    }, 450);
  });
  function secLibrary() {
    var st = App.store.get('services').jellyfin;
    return html`${row('Jellyfin', 'Where saved songs live and play from.', html`<div class="sys-inline"><span id="sys-jf-status" aria-live="polite">${st.state === 'down' ? statusLine('down', 'Not responding') : statusLine('ok', 'Online, 1,204 songs')}</span><button class="btn btn--sm" data-act="sys-jellyfin-test">Test connection</button></div>`)}
      ${locked('Server URL', cfg.library.jellyfinUrl)}
      ${locked('API key', '')}
      ${locked('Music folder', cfg.library.musicFolder)}
      ${group('saving')}
      <div class="sys-row sys-row--stack">
        <div class="sys-row__text"><label class="sys-row__label" for="sys-naming">Folder and file names</label><span class="sys-row__help" id="sys-naming-help">Saved songs are named from this template. The preview uses a real song from your queue.</span></div>
        <div class="sys-row__ctl">
          <div class="sys-naming"><input class="input sys-naming__input" id="sys-naming" value="${cfg.library.naming}" spellcheck="false" autocomplete="off" aria-describedby="sys-naming-help sys-naming-out" aria-invalid="${namingPreview(cfg.library.naming).bad.length ? 'true' : 'false'}"><button class="btn btn--quiet btn--sm" data-act="sys-naming-reset">Reset</button></div>
          <div class="sys-tokens" role="group" aria-label="Insert a token">${TOKENS.map(function (k) { return html`<button type="button" class="chip" data-act="sys-token-insert" data-token="${k}">{${k}}</button>`; })}</div>
          <div id="sys-naming-out" aria-live="polite">${namingOut()}</div>
        </div></div>
      ${row('When a song is already saved', 'What Save does when the library has the same song.', choice('library.duplicatePolicy', 'Duplicate policy', [['ask', 'Ask each time'], ['keep', 'Keep existing'], ['replace', 'Replace if better']], 'sys-seg--wrap'))}
      ${row('Rescan library', 'Picks up files added to the music folder outside lull.', html`<div class="sys-inline sys-inline--col"><button class="btn" data-act="sys-rescan">${icon('refresh')}Rescan library now</button><div id="sys-rescan" class="sys-rescan" aria-live="polite"></div></div>`)}`;
  }

  /* ---------- sharing ---------- */
  fmtRange['sharing.uploadLimitKBs'] = function (v) { return v ? num(v) + ' KB/s' : 'No limit'; };
  effects['sharing.mode'] = function () { repaint('#sys-sharing', sharingBody(), '[data-key="sharing.mode"] [aria-checked="true"]'); };
  function sharingBody() {
    var on = cfg.sharing.mode === 'library';
    return html`${row('Sharing now', '', on ? statusLine('ok', 'Sharing ' + num(cfg.sharing.sharedFiles) + ' files, read-only') : statusLine('warn', 'Sharing nothing'), { cls: 'sys-row--status' })}
      ${row('Share with Soulseek peers', 'Default: your saved library, read-only. Peers can download songs; they can never change or delete anything.', choice('sharing.mode', 'What to share', [['library', 'Saved library, read-only'], ['none', 'Nothing']], 'sys-seg--wrap'))}
      <p class="sys-note ${on ? '' : 'sys-note--warn'}">${icon(on ? 'info' : 'warning', null, 'ic-16')}<span>Soulseek expects sharing. Many peers queue or refuse people who share nothing, so finding sources gets slower${on ? '.' : ' while this is off.'}</span></p>
      <div class="${on ? '' : 'sys-off'}" ${on ? '' : raw('inert')}>
        ${row('Upload slots', 'How many peers can download from you at once.', stepper('sharing.uploadSlots', 'Upload slots', 1, 10, 1))}
        ${row('Upload speed limit', 'Keeps uploads from slowing your own streams.', range('sharing.uploadLimitKBs', 'Upload speed limit', 0, 2000, 100))}
      </div>`;
  }
  function secSharing() { return html`<div id="sys-sharing">${sharingBody()}</div>`; }

  /* ---------- Last.fm: not connected -> waiting for approval -> connected | error ---------- */
  function lfmSet(st) {
    cfg.lastfm.state = st; cfg.lastfm.connected = st === 'connected';
    if (st === 'connected') App.setService('lastfm', { state: cfg.lastfm.pending ? 'warn' : 'ok', queued: cfg.lastfm.pending, detail: cfg.lastfm.pending ? cfg.lastfm.pending + ' scrobbles waiting' : 'Connected as ' + cfg.lastfm.user });
    else if (st === 'off') App.setService('lastfm', { state: 'down', queued: 0, detail: 'Not connected' });
    repaint('#sys-lastfm', lastfmBody(), '[data-lfm-focus]');
    repaint('#sys-w-lastfm', welcomeLastfm(), '[data-lfm-focus]');
    var w = $('#sys-welcome-steps'); if (w) updateWelcomeProgress();
  }
  App.action('sys-lfm-connect', function () { lfmSet('waiting'); App.toast('Opened Last.fm in a new window', { icon: 'external' }); });
  App.action('sys-lfm-reopen', function () { App.toast('Opened Last.fm again', { icon: 'external' }); });
  App.action('sys-lfm-approved', function () { lfmSet('connected'); App.toast('Connected to Last.fm as ' + cfg.lastfm.user, { kind: 'ok', icon: 'check' }); });
  App.action('sys-lfm-cancel', function () { lfmSet('off'); });
  App.action('sys-lfm-disconnect', function () {
    App.confirm({ title: 'disconnect Last.fm?', text: 'Scrobbling stops and ' + cfg.lastfm.pending + ' queued scrobbles are dropped. Recommendations stop using your Last.fm history.', confirm: 'Disconnect', danger: true }, function () { lfmSet('off'); App.toast('Last.fm disconnected', { icon: 'radio' }); });
  });
  App.action('sys-lfm-retry', function (el) {
    var n = cfg.lastfm.pending; el.disabled = true;
    /* PROTO: simulated delay */ setTimeout(function () {
      cfg.lastfm.pending = 0;
      App.setService('lastfm', { state: 'ok', queued: 0, detail: 'Connected as ' + cfg.lastfm.user + ', nothing waiting' });
      App.banner.hide('sys-lastfm');
      repaint('#sys-lastfm', lastfmBody());
      App.toast(n + ' scrobble' + (n === 1 ? '' : 's') + ' sent to Last.fm', { kind: 'ok', icon: 'check' });
    }, 700);
  });
  effects['lastfm.scrobbling'] = function (v) { App.announce(v ? 'Scrobbling on' : 'Scrobbling paused'); };
  function lastfmBody() {
    var s = cfg.lastfm, st = s.state, box;
    if (st === 'off') box = html`<div class="inset sys-connect">${icon('radio', null, 'ic-24')}<div><p class="sys-connect__title">Not connected</p><p class="sys-connect__text">Connect to scrobble what you play and let your Last.fm history shape recommendations.</p></div>
      <div class="sys-connect__actions"><button class="btn btn--primary" data-act="sys-lfm-connect" data-lfm-focus>${icon('external')}Connect Last.fm</button></div></div>`;
    else if (st === 'waiting') box = html`<div class="inset sys-connect" role="status">${raw('<span class="spin-ring sys-connect__spin" aria-hidden="true"></span>')}<div><p class="sys-connect__title">Waiting for approval</p><p class="sys-connect__text">Approve lull in the Last.fm window, then come back here. This page checks every few seconds.</p></div>
      <div class="sys-connect__actions"><button class="btn btn--primary" data-act="sys-lfm-approved" data-lfm-focus>I approved it</button><button class="btn" data-act="sys-lfm-reopen">Open Last.fm again</button><button class="btn btn--quiet" data-act="sys-lfm-cancel">Cancel</button></div></div>`;
    else if (st === 'error') box = html`<div class="inset sys-connect sys-connect--error" role="alert">${ui.statusGlyph('down', 'Error')}<div><p class="sys-connect__title">Couldn't connect to Last.fm</p><p class="sys-connect__text">The approval window closed before Last.fm confirmed, or the request was declined. Nothing was changed.</p></div>
      <div class="sys-connect__actions"><button class="btn btn--primary" data-act="sys-lfm-connect" data-lfm-focus>Try again</button><button class="btn btn--quiet" data-act="sys-lfm-cancel">Cancel</button></div></div>`;
    else box = html`<div class="inset sys-connect sys-connect--ok">${ui.statusGlyph('ok', 'Connected')}<div><p class="sys-connect__title">Connected as <b>${s.user}</b></p><p class="sys-connect__text">Scrobbles go to Last.fm after half a song or 4 minutes, whichever comes first.</p></div>
      <div class="sys-connect__actions"><button class="btn" data-act="sys-lfm-disconnect" data-lfm-focus>Disconnect</button></div></div>`;
    var on = st === 'connected';
    return html`${box}
      ${on ? html`${switchRow('lastfm.scrobbling', 'Scrobble what I play', 'Library and Soulseek plays both count.')}
        ${row('Queued scrobbles', s.pending ? 'Waiting because Last.fm did not answer. They retry on their own.' : 'Nothing waiting.', s.pending ? html`<div class="sys-inline">${ui.countBadge(s.pending, s.pending + ' queued')}<button class="btn btn--sm" data-act="sys-lfm-retry">Retry now</button></div>` : statusLine('ok', 'All sent'))}
        ${switchRow('lastfm.useHistory', 'Use my Last.fm history for recommendations', 'Adds years of listening to the taste profile on Stats.')}` : ''}
      ${group('app credentials')}
      ${locked('API key', '')}
      ${locked('Shared secret', '')}`;
  }
  function secLastfm() { return html`<div id="sys-lastfm">${lastfmBody()}</div>`; }

  /* ---------- discovery ---------- */
  var DSOURCES = [
    ['listenbrainz', 'ListenBrainz', 'Similar artists and what listeners like you play. No account needed.'],
    ['musicbrainz', 'MusicBrainz', 'Release dates, tags and the new-release feed. Saving uses it to match songs either way.'],
    ['lastfm', 'Last.fm', 'Similar artists and tags, plus your history when Last.fm is connected.'],
    ['discogs', 'Discogs', 'Labels and pressings for older releases. Needs a token in server config.']
  ];
  cfg.discovery.on = { listenbrainz: true, musicbrainz: true, lastfm: true, discogs: false };
  fmtRange['discovery.adventurousness'] = function (v) { return v < .34 ? 'Familiar' : v < .67 ? 'Balanced' : 'Adventurous'; };
  DSOURCES.forEach(function (s) { effects['discovery.on.' + s[0]] = function (v) { App.announce(s[1] + (v ? ' on' : ' off')); }; });
  function secDiscovery() {
    var arts = M.artists.filter(function (a) { return App.isFollowed(a.id); });
    return html`${row('Adventurousness', 'How far Home and radio stray from what you already play.', html`${range('discovery.adventurousness', 'Adventurousness', 0, 1, .05)}<span class="sys-range__ends" aria-hidden="true"><span>Familiar</span><span>Adventurous</span></span>`)}
      ${group('sources')}
      ${DSOURCES.map(function (s) {
        var st = s[0] === 'discogs' ? 'down' : s[0] === 'lastfm' && cfg.lastfm.state !== 'connected' ? 'warn' : 'ok';
        var stText = s[0] === 'discogs' ? 'needs config' : st === 'warn' ? 'no history' : 'online';
        return switchRow('discovery.on.' + s[0], s[1], s[2], { disabled: s[0] === 'discogs' && 'Add a Discogs token in server config to turn this on', status: html`<span class="sys-src-status">${ui.statusGlyph(st)}${stText}</span>` });
      })}
      ${group('following')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Tags</span><span class="sys-row__help">Followed tags feed Home with fresh releases. Tap to follow or unfollow.</span></div>
        <div class="sys-row__ctl"><div class="cluster sys-chips">${M.tags.map(function (t) { return followChip(t.id, t.name); })}</div></div></div>
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Artists</span><span class="sys-row__help">${arts.length} followed. New releases from them show first on Home.</span></div>
        <div class="sys-row__ctl"><ul class="sys-follows">${arts.map(function (a) {
          return html`<li>${ui.cover(a, 36, { round: true })}<a href="#/artist/${a.id}">${a.name}</a><button class="btn btn--sm" data-act="sys-follow" data-id="${a.id}" aria-pressed="true">Following</button></li>`;
        })}</ul><a class="btn btn--quiet btn--sm" href="#/following">See everything you follow</a></div></div>`;
  }
  App.action('sys-follow', function (el) {
    var id = el.getAttribute('data-id'); App.toggleFollow(id);
  });
  App.on('follow', function (e) {
    $$('[data-act="sys-follow"][data-id="' + e.id + '"]').forEach(function (b) { b.setAttribute('aria-pressed', e.followed ? 'true' : 'false'); b.textContent = e.followed ? 'Following' : 'Follow'; });
  });

  /* ---------- appearance ---------- */
  var designFocus = false;                         // the picker had focus: give it back after the re-render
  effects['appearance.design'] = function (v) {
    designFocus = !!(document.activeElement && document.activeElement.closest('.sys-designs'));
    App.setPref('design', v);
    App.announce(App.designs[v].label + ' design');
  };
  effects['appearance.theme'] = function (v) { App.setPref('theme', v === 'system' ? null : v); };
  effects['appearance.motion'] = function (v) { App.setPref('motion', v); };
  effects['appearance.textSize'] = function (v) { App.setPref('textSize', v); };
  effects['appearance.defaultSkin'] = function (v) { App.storage('defaultSkin', v); var s = M.skins.filter(function (k) { return k.id === v; })[0]; App.announce(s.name + ' is the default skin'); };
  effects['appearance.showNsfwSkins'] = function (v) { App.storage('showNsfwSkins', v); };
  function skinThumb(s) {
    return raw('<span class="sys-skin__thumb" aria-hidden="true" style="--a:' + s.palette[0] + ';--b:' + s.palette[1] + ';--c:' + s.palette[2] + '"><i></i><i></i><i></i></span>');
  }
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
  function secAppearance() {
    var p = App.prefs();
    cfg.appearance.design = p.design; cfg.appearance.theme = p.theme; cfg.appearance.motion = p.motion; cfg.appearance.textSize = p.textSize;
    var skins = M.skins.filter(function (s) { return s.downloaded || s.favourite; });
    return html`<div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-design-l">Design</span><span class="sys-row__help">The whole look: colours, type, window chrome and layout. Applies at once on this device.</span></div>
        <div class="sys-row__ctl"><div class="sys-designs" role="radiogroup" aria-labelledby="sys-design-l" data-key="appearance.design">${App.designOrder.map(function (n) { return designCard(n, n === p.design); })}</div></div></div>
      ${row('Theme', 'Match system follows your device. T switches day and night anywhere.', choice('appearance.theme', 'Theme', [['system', 'Match system'], ['day', 'Day'], ['night', 'Night']]))}
      ${row('Motion', 'Reduced swaps slides and loops for quick fades. Full ignores the system setting.', choice('appearance.motion', 'Motion', [['system', 'Match system'], ['reduce', 'Reduced'], ['full', 'Full']]))}
      ${row('Text size', 'Scales all text and spacing on this device.', choice('appearance.textSize', 'Text size', [[90, 'Smaller'], [100, 'Default'], [112, 'Larger'], [125, 'Largest']], 'sys-seg--4'))}
      ${group('classic mode')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-skin-l">Default skin</span><span class="sys-row__help">Used when Classic mode opens. Downloaded and favourite skins are listed; find more in Classic mode.</span></div>
        <div class="sys-row__ctl"><div class="sys-skins" role="radiogroup" aria-labelledby="sys-skin-l" data-key="appearance.defaultSkin">${skins.map(function (s) {
          var on = s.id === cfg.appearance.defaultSkin;
          return html`<button type="button" class="sys-skin" role="radio" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-value="${s.id}">${skinThumb(s)}<span class="sys-skin__name">${s.name}</span>${s.favourite ? icon('star-fill', 'Favourite', 'ic-16 sys-skin__fav') : ''}</button>`;
        })}</div><a class="btn btn--quiet btn--sm" href="#/classic">${icon('classic')}Browse skins in Classic mode</a></div></div>
      ${switchRow('appearance.showNsfwSkins', 'Show NSFW skins', 'Hidden by default in the skin browser.')}`;
  }

  /* ---------- shortcuts (read-only list) ---------- */
  function secShortcuts() {
    return html`<p class="sys-card__lede">The same list opens anywhere with ${ui.keycap('?')}. Shortcuts pause while you type in a field and inside Classic mode.</p>
      <p><button class="btn" data-act="shortcuts">${icon('keyboard')}Open the shortcuts overlay</button></p>${shortcutList('')}`;
  }

  /* ---------- app ---------- */
  function detectInstall() {
    try { if (matchMedia('(display-mode: standalone)').matches || navigator.standalone) return 'installed'; } catch (e) { /* old browsers */ }
    return /iPad|iPhone|iPod/.test(navigator.userAgent) ? 'ios' : 'available';
  }
  var installEvent = null;
  window.addEventListener('beforeinstallprompt', function (e) { e.preventDefault(); installEvent = e; if (cfg.app.install !== 'installed') cfg.app.install = 'available'; });
  window.addEventListener('appinstalled', function () { cfg.app.install = 'installed'; repaint('#sys-install', installBody()); });
  function installBody() {
    var s = cfg.app.install;
    if (s === 'installed') return statusLine('ok', 'Installed on this device');
    if (s === 'ios') return html`<button class="btn btn--primary" data-act="sys-ios-sheet">${icon('device')}How to install</button>`;
    return html`<button class="btn btn--primary" data-act="sys-install">${icon('device')}Install app</button>`;
  }
  App.action('sys-install', function () {
    if (installEvent) {
      installEvent.prompt();
      installEvent.userChoice.then(function (r) { if (r.outcome === 'accepted') { cfg.app.install = 'installed'; repaint('#sys-install', installBody()); } installEvent = null; });
      return;
    }
    // prototype: no real prompt from file://, so simulate the accepted path
    cfg.app.install = 'installed'; repaint('#sys-install', installBody());
    App.toast('lull installed. Open it from your apps for a full-screen player.', { kind: 'ok', icon: 'device' });
  });
  App.action('sys-ios-sheet', function () { openIosSheet(); });
  App.action('sys-update-check', function (el) {
    el.disabled = true;
    /* PROTO: simulated delay */ setTimeout(function () { el.disabled = false; updateToast(); }, 900);
  });
  App.action('sys-health', function (el) {
    var out = $('#sys-health'); el.disabled = true;
    out.setAttribute('aria-busy', 'true');
    /* PROTO: simulated delay */ setTimeout(function () { el.disabled = false; out.removeAttribute('aria-busy'); repaint('#sys-health', healthRows(true)); App.announce('Health checked'); }, 800);
  });
  App.action('sys-forget-token', function () {
    App.confirm({ title: 'forget the token on this device?', text: 'lull asks for the access token again before it plays anything here. Other devices stay signed in.', confirm: 'Forget token', danger: true }, function () {
      cfg.app.tokenRemembered = false; setTimeout(function () { openTokenGate({}); }, 280);
    });
  });
  function healthRows(fresh) {
    var s = App.store.get('services');
    var rows = [['ok', 'Server', 'Answering in 18 ms'], ['ok', 'Database', 'Healthy, 6.2 MB'], [s.soulseek.state, 'Soulseek', s.soulseek.detail], [s.jellyfin.state, 'Jellyfin', s.jellyfin.detail], [s.lastfm.state, 'Last.fm', s.lastfm.detail], [s.cache.state, 'Cache', s.cache.detail]];
    return html`<div class="status-list">${rows.map(function (r) {
      return html`<div class="status-row">${ui.statusGlyph(r[0], r[0] === 'ok' ? 'ok' : r[0] === 'warn' ? 'warning' : 'down')}<div><span class="status-row__name">${r[1]}</span><span class="status-row__detail">${r[2]}</span></div></div>`;
    })}</div><p class="sys-card__lede sys-health__when">Checked ${fresh ? 'just now' : '2 min ago'} from <span class="px">/api/health</span></p>`;
  }
  function secApp() {
    return html`${row('Version', '', html`<span class="sys-inline"><span class="px sys-version">${cfg.app.version}</span><button class="btn btn--sm" data-act="sys-update-check">Check for updates</button></span>`)}
      ${row('Install app', 'Adds lull to your home screen or dock with lock-screen controls and its own window.', html`<div id="sys-install">${installBody()}</div>`)}
      ${row('First-run steps', 'Connect Last.fm, follow a few tags, try a search.', html`<a class="btn" href="#/welcome">Show welcome steps</a>`)}
      ${group('server')}
      ${locked('Server URL', cfg.app.server)}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Server health</span><span class="sys-row__help">What the server reports about itself and its services.</span></div>
        <div class="sys-row__ctl"><div id="sys-health" aria-live="polite">${healthRows(false)}</div><button class="btn btn--sm" data-act="sys-health">${icon('refresh')}Check now</button></div></div>
      ${row('Access token', cfg.app.tokenRemembered ? 'Remembered on this device.' : 'Not remembered: asked for on every visit.', html`<button class="btn" data-act="sys-forget-token">Forget on this device</button>`)}
      ${inWrapper() ? html`${group('desktop app')}
        ${row('Server address', 'The desktop app loads lull from this server.', html`<span class="sys-inline"><span class="input input--readonly sys-locked" tabindex="0" aria-label="Server address: ${cfg.app.server}">${icon('link', null, 'ic-16')}<span class="ellipsis">${cfg.app.server}</span></span><button class="btn btn--sm" data-act="sys-desktop-connect">Change server...</button></span>`)}
        ${switchRow('app.closeToTray', 'Close to tray', 'Closing the window keeps the music playing; quit from the tray icon.')}
        ${switchRow('app.launchAtLogin', 'Launch at login', 'Starts with your computer.')}
        ${switchRow('app.startInTray', 'Start in the tray', 'Launches without opening the window.')}
        ${row('Mini player style', 'Opens from the tray, the player bar or Ctrl+Shift+M.', choice('app.miniStyle', 'Mini player style', [['device', 'Device'], ['strip', 'Strip']]))}
        ${switchRow('app.miniOnTop', 'Mini player always on top', 'Some Linux desktops decide this themselves.')}
        ${/Linux/.test(navigator.userAgent) || App.current.query.titlebar === 'gnome' ? switchRow('app.ownTitlebar', 'Use lull title bar', 'Linux only. Replaces the system title bar; takes effect after a restart.') : ''}` : ''}`;
  }
  Object.assign(cfg.app, { closeToTray: true, launchAtLogin: false, startInTray: false, miniStyle: 'device', miniOnTop: true, ownTitlebar: false });
  effects['app.ownTitlebar'] = function () { App.toast('Restart lull to change the title bar', { icon: 'refresh' }); };
  App.action('sys-desktop-connect', function () {
    if (window.LullDesktop) window.LullDesktop.windowAction('change-server');
    else openTokenGate({ desktop: true, url: cfg.app.server });
  });

  var BODY = { playback: secPlayback, sources: secSources, cache: secCache, library: secLibrary, sharing: secSharing, lastfm: secLastfm, discovery: secDiscovery, appearance: secAppearance, shortcuts: secShortcuts, app: secApp };

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
  function mountSettings(ctx) {
    var off = App.on('prefs', function () {
      var p = App.prefs(); showChoice('appearance.theme', p.theme); showChoice('appearance.motion', p.motion); showChoice('appearance.design', p.design);
    });
    var offD = App.on('design', function (name) {
      if (!designFocus) return; designFocus = false;
      var b = $('.sys-design[data-value="' + name + '"]'); if (b) b.focus();
    });
    ctx.onCleanup(function () { off(); offD(); });
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
     4. Command palette (Ctrl/Cmd+K): commands + songs, albums, artists, tags + "search for"
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
  function paletteResults(q) {
    var words = q.toLowerCase().split(/\s+/).filter(Boolean);
    function hit(text) { text = text.toLowerCase(); return words.every(function (w) { return text.indexOf(w) >= 0; }); }
    var groups = [];
    if (!words.length) {
      var all = commands(), pick = ['Go to Home', 'Go to Library', 'Toggle Classic mode', 'Go to Settings', 'Keyboard shortcuts'];
      groups.push({ label: 'suggested', items: all.filter(function (c) { return pick.indexOf(c.label) >= 0 || /theme$/.test(c.label); }) });
      var rec = App.store.get('recentSearches') || M.recentSearches;
      groups.push({ label: 'recent searches', items: rec.slice(0, 3).map(function (r) { return { label: r, icon: 'clock', sub: 'Search', run: go('/search?q=' + encodeURIComponent(r)) }; }) });
      return groups;
    }
    var cmds = commands().filter(function (c) { return hit(c.label); }).slice(0, 6);
    if (cmds.length) groups.push({ label: 'commands', items: cmds });
    var songs = M.tracks.filter(function (t) { return hit(t.title + ' ' + t.artist); }).slice(0, 4).map(function (t) {
      return { label: t.title, sub: t.artist, cover: t, run: function () { P.playTrack(t.id); App.toast('Playing ' + t.title, { icon: 'play' }); } };
    });
    var albums = M.albums.filter(function (a) { return hit(a.title + ' ' + a.artist); }).slice(0, 3).map(function (a) { return { label: a.title, sub: 'Album by ' + a.artist, cover: a, run: go('/album/' + a.id) }; });
    var artists = M.artists.filter(function (a) { return hit(a.name); }).slice(0, 3).map(function (a) { return { label: a.name, sub: 'Artist', cover: a, round: true, run: go('/artist/' + a.id) }; });
    var tags = M.tags.filter(function (t) { return hit(t.name); }).slice(0, 3).map(function (t) { return { label: t.name, sub: 'Tag', icon: 'tag', run: go('/tag/' + encodeURIComponent(t.id)) }; });
    if (songs.length) groups.push({ label: 'songs', items: songs });
    if (albums.length || artists.length) groups.push({ label: 'albums and artists', items: albums.concat(artists) });
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
    var d = App.openDialog({ label: 'Command palette', className: 'sys-palette', closable: false,
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
        function run(i) { var it = flat[i]; if (!it) return; close(); setTimeout(it.run, 0); }
        input.addEventListener('input', function () { list.innerHTML = String(render(input.value)); active = 0; input.setAttribute('aria-activedescendant', 'sys-pal-o0'); list.scrollTop = 0; });
        input.addEventListener('keydown', function (e) {
          if (e.key === 'ArrowDown') { e.preventDefault(); setActive(active + 1); }
          else if (e.key === 'ArrowUp') { e.preventDefault(); setActive(active - 1); }
          else if (e.key === 'Enter') { e.preventDefault(); run(active); }
        });
        list.addEventListener('click', function (e) { var o = e.target.closest('[role="option"]'); if (o) run(+o.getAttribute('data-i')); });
        list.addEventListener('mousemove', function (e) { var o = e.target.closest('[role="option"]'); if (o && +o.getAttribute('data-i') !== active) setActive(+o.getAttribute('data-i')); });
      } });
    return d;
  }
  App.hooks.palette = function () { if ($('.sys-palette')) App.closeTop(); else openPalette(''); };

  /* =====================================================================
     5. First-run welcome (/welcome): 3 real steps
     ===================================================================== */
  var welcome = { searched: false };
  var followed0 = copy(App.store.get('followed') || {});   // restored by resetDemo after the first-run state empties it
  function welcomePicks() {
    var tags = (M.feed.coldStartTags && M.feed.coldStartTags.length ? M.feed.coldStartTags : M.tags.map(function (t) { return t.id; })).slice(0, 6);
    return { tags: tags, artists: M.artists.slice(0, 4) };
  }
  function welcomeLastfm() {
    var st = cfg.lastfm.state;
    if (st === 'connected') return html`<p class="sys-step__done">${icon('check', null, 'ic-16')}Connected as <b>${cfg.lastfm.user}</b></p>`;
    if (st === 'waiting') return html`<div class="sys-inline" role="status">${raw('<span class="spin-ring" aria-hidden="true"></span>')}<span>Waiting for you to approve lull on Last.fm</span></div>
      <div class="sys-inline"><button class="btn btn--primary btn--sm" data-act="sys-lfm-approved" data-lfm-focus>I approved it</button><button class="btn btn--quiet btn--sm" data-act="sys-lfm-cancel">Cancel</button></div>`;
    if (st === 'error') return html`<p class="field__error">${icon('warning', null, 'ic-16')}Last.fm didn't confirm. Nothing was changed.</p><button class="btn btn--sm" data-act="sys-lfm-connect" data-lfm-focus>Try again</button>`;
    return html`<button class="btn btn--sm" data-act="sys-lfm-connect" data-lfm-focus>${icon('external')}Connect Last.fm</button>`;
  }
  function welcomeFollowCount() { var p = welcomePicks(); return p.tags.concat(p.artists.map(function (a) { return a.id; })).filter(App.isFollowed).length; }
  function stepState() { return [cfg.lastfm.state === 'connected', welcomeFollowCount() >= 3, welcome.searched]; }
  function updateWelcomeProgress() {
    var done = stepState();
    $$('.sys-step').forEach(function (li, i) {
      li.classList.toggle('is-done', done[i]);
      $('.sys-step__n', li).innerHTML = String(done[i] ? html`${icon('check', null, 'ic-16')}<span class="vh">Done:</span>` : html`${i + 1}`);
    });
    var n = done.filter(Boolean).length, c = $('#sys-w-count');
    if (c) c.textContent = n + ' of 3 done';
    var f = $('#sys-w-follow-n'); if (f) f.textContent = Math.min(3, welcomeFollowCount()) + ' of 3';
  }
  App.action('sys-w-follow', function (el) {
    var id = el.getAttribute('data-id'), on = el.getAttribute('aria-pressed') !== 'true';
    App.store.update('followed', function (f) { if (on) f[id] = true; else delete f[id]; });
    el.setAttribute('aria-pressed', on ? 'true' : 'false');
    App.announce((on ? 'Following ' : 'Unfollowed ') + el.getAttribute('data-name'));
    updateWelcomeProgress();
  });
  function renderWelcome() {
    var picks = welcomePicks(), tags = picks.tags, arts = picks.artists, done = stepState();
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
            <div class="sys-step__act"><div class="cluster sys-chips" role="group" aria-label="Tags">${tags.map(function (id) {
              var t = M.tag(id) || { id: id, name: id };
              return followChip(t.id, t.name, 'sys-w-follow');
            })}</div>
            <div class="sys-picks" role="group" aria-label="Artists">${arts.map(function (a) {
              var on = App.isFollowed(a.id);
              return html`<button type="button" class="sys-pick" data-act="sys-w-follow" data-id="${a.id}" data-name="${a.name}" aria-pressed="${on ? 'true' : 'false'}">${ui.cover(a, 44, { round: true })}<span>${a.name}</span><span class="sys-pick__state" aria-hidden="true">${icon('plus', null, 'ic-16 sys-pick__plus')}${icon('check', null, 'ic-16 sys-pick__check')}</span></button>`;
            })}</div></div></div></li>
          <li class="sys-step ${done[2] ? 'is-done' : ''}"><span class="sys-step__n">${n(2)}</span><div class="sys-step__body">
            <h3 class="sys-step__title">Try a search</h3><p class="sys-step__text">Searches cover your library and Soulseek at once. Results stream in as peers answer.</p>
            <form class="sys-step__act sys-w-search" id="sys-w-search" role="search"><div class="search-field">${icon('search')}<label class="vh" for="sys-w-q">Search songs, albums, artists</label><input class="input" id="sys-w-q" type="search" placeholder="Try lantern season" autocomplete="off"></div><button class="btn btn--primary" type="submit">Search</button></form>
            <p class="sys-step__try">Or try ${['lantern season', 'city-pop revival'].map(function (s, i) { return html`${i ? ' or ' : ''}<button type="button" class="btn btn--quiet btn--sm" data-act="sys-w-try" data-q="${s}">${s}</button>`; })}</p></div></li>
        </ol>
        <footer class="sys-welcome__foot"><span class="fg-2" id="sys-w-count" aria-live="polite">${done.filter(Boolean).length} of 3 done</span><span class="spacer"></span>
          <a class="btn btn--quiet" href="#/home">Skip for now</a><a class="btn btn--primary" href="#/home" data-act="sys-w-finish">Start listening</a></footer>
      </section></div>`;
  }
  function welcomeSearch(q) { q = (q || '').trim(); if (!q) { $('#sys-w-q').focus(); return; } welcome.searched = true; App.go('/search?q=' + encodeURIComponent(q)); }
  App.action('sys-w-try', function (el) { welcomeSearch(el.getAttribute('data-q')); });
  App.action('sys-w-finish', function () { App.go('/home'); App.toast('The welcome steps live in Settings, App', { icon: 'info' }); });
  App.route('/welcome', { render: renderWelcome, title: 'Welcome', nav: 'home', mount: function (ctx) {
    var f = $('#sys-w-search', ctx.el);
    f.addEventListener('submit', function (e) { e.preventDefault(); welcomeSearch($('#sys-w-q').value); });
  } });

  /* =====================================================================
     6. Token gate (+ desktop wrapper first run: server URL above the token)
     ===================================================================== */
  function openTokenGate(o) {
    o = o || {};
    var titleId = App.uid('sys-gate-t');
    var d = App.openDialog({ className: 'sys-gate', closable: false, dismissible: false, label: o.desktop ? 'Connect to your server' : 'Unlock lull',
      body: html`<form class="sys-gate__form" novalidate>
        <div class="sys-gate__mark">${ui.wordmark('desktop')}</div>
        <h2 class="sys-gate__title" id="${titleId}">${o.desktop ? 'connect to your server' : 'this server needs a token'}</h2>
        <p class="sys-gate__text">${o.desktop ? 'Enter the address of your lull server and its access token. You only do this once on this computer.' : 'Ask whoever runs this server for the access token, or find it in the server config.'}</p>
        ${o.desktop ? html`<div class="field"><label class="field__label" for="sys-gate-url">Server URL</label><div class="sys-naming"><input class="input" id="sys-gate-url" type="url" value="${o.url || ''}" placeholder="https://music.home.arpa" autocomplete="url" spellcheck="false" aria-describedby="sys-gate-url-st"><button type="button" class="btn" data-gate-test>Test</button></div><div id="sys-gate-url-st" class="sys-gate__st" aria-live="polite"></div></div>` : ''}
        <div class="field"><label class="field__label" for="sys-gate-token">Access token</label><input class="input" id="sys-gate-token" type="password" autocomplete="current-password" spellcheck="false" aria-describedby="sys-gate-err" ${o.desktop ? '' : raw('autofocus')}><span class="field__error" id="sys-gate-err" ${o.error ? '' : raw('hidden')}>${icon('warning', null, 'ic-16')}<span data-msg>${o.error || ''}</span></span></div>
        ${switchRow('app.tokenRemembered', 'Remember on this device', 'Off: asked for again when you close the browser.')}
        <div class="sys-gate__foot"><button type="submit" class="btn btn--primary btn--block">${o.desktop ? 'Connect' : 'Unlock'}</button></div>
      </form>`,
      onMount: function (el, close) {
        el.setAttribute('aria-labelledby', titleId); el.removeAttribute('aria-label');
        var form = $('form', el), tok = $('#sys-gate-token', el), err = $('#sys-gate-err', el), url = $('#sys-gate-url', el), st = $('#sys-gate-url-st', el);
        if (o.error) { tok.value = 'wrong-token-1234'; tok.setAttribute('aria-invalid', 'true'); }
        function showErr(m) { $('[data-msg]', err).textContent = m; err.hidden = !m; tok.setAttribute('aria-invalid', m ? 'true' : 'false'); }
        function test(then) {
          if (!url) return then && then(true);
          var v = url.value.trim();
          st.innerHTML = String(html`<span class="sys-status"><span class="spin-ring" aria-hidden="true"></span><span>Testing ${v || 'the address'}</span></span>`);
          /* PROTO: simulated delay */ setTimeout(function () {
            var bad = !/^https?:\/\/[^\s.]+\.[^\s]+/.test(v) || /offline|unreach/.test(v);
            st.innerHTML = String(bad ? statusLine('down', v ? 'Unreachable: nothing answered at ' + v : 'Enter the server address first') : statusLine('ok', 'Found lull 0.1.0'));
            url.setAttribute('aria-invalid', bad ? 'true' : 'false');
            if (then) then(!bad);
          }, o.slow ? 1e9 : 900);
        }
        if (url) {
          $('[data-gate-test]', el).addEventListener('click', function () { test(); });
          if (o.testNow) test();
          setTimeout(function () { (url.value ? tok : url).focus(); }, 40);
        }
        form.addEventListener('submit', function (e) {
          e.preventDefault();
          var v = tok.value.trim();
          if (!v) { showErr('Enter the access token'); tok.focus(); return; }
          test(function (ok) {
            if (!ok) { url.focus(); return; }
            if (/wrong|bad/.test(v)) { showErr('Token rejected. Check for spaces and try again.'); tok.select(); return; }
            showErr(''); close();
            App.toast(o.desktop ? 'Connected to your server' : 'Unlocked' + (cfg.app.tokenRemembered ? '. Remembered on this device.' : ''), { kind: 'ok', icon: 'lock' });
          });
        });
      } });
    return d;
  }

  /* =====================================================================
     7. PWA: install toast, iOS instructions sheet, update toast
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
  function installToast() {
    App.toast('Install lull for a full-screen player with lock-screen controls', { icon: 'device', action: 'Install', sticky: true, onAction: function () {
      if (cfg.app.install === 'ios') openIosSheet(); else App.go('/settings/app');
    } });
  }
  function updateToast() {
    App.toast('New version ready', { icon: 'refresh', action: 'Reload', sticky: true, onAction: function () {
      history.replaceState(null, '', '#' + App.current.path); location.reload();
    } });
  }

  /* =====================================================================
     8. Banners demo (degraded / offline). Server unreachable is not dismissible.
     ===================================================================== */
  var retryTimer = null;
  function setSearchPaused(on) {
    var s = $('#header-search'); if (!s) return;
    s.disabled = on; s.placeholder = on ? 'Search pauses until the server is back' : 'Search songs, albums, artists';
  }
  function bannerUnreachable() {
    var left = 8;
    function paint() {
      var m = $('[data-banner="sys-unreachable"] .banner__msg'); if (!m) { clearInterval(retryTimer); return; }
      m.innerHTML = String(html`Can't reach your server. <span aria-hidden="true">Retrying in <span class="num">${left}</span> s.</span><span class="vh">Retrying automatically.</span> The queue and saved songs still work.`);
    }
    App.banner.show({ id: 'sys-unreachable', kind: 'down', icon: 'offline', text: 'Can\'t reach your server.', dismissible: false, action: { label: 'Retry now', run: function () {
      clearInterval(retryTimer);
      var m = $('[data-banner="sys-unreachable"] .banner__msg'); if (m) m.textContent = 'Reconnecting to your server';
      /* PROTO: simulated delay */ setTimeout(function () { App.banner.hide('sys-unreachable'); setSearchPaused(false); App.toast('Connected to your server again', { kind: 'ok', icon: 'check' }); }, 1200);
    } } });
    setSearchPaused(true); paint();
    clearInterval(retryTimer);
    /* PROTO: simulated delay */ retryTimer = setInterval(function () { left -= 1; if (left < 1) left = 8; paint(); }, 1000);
  }
  var BANNERS = {
    unreachable: bannerUnreachable,
    offline: function () { App.banner.show({ id: 'sys-offline', kind: 'warn', icon: 'offline', text: 'You\'re offline. Saved and cached songs still play; search and streaming wait for the connection.' }); },
    jellyfin: function () {
      App.setService('jellyfin', { state: 'down', detail: 'Not responding since 20:41' });
      App.banner.show({ id: 'sys-jellyfin', kind: 'warn', icon: 'warning', text: 'Jellyfin isn\'t responding. Your library is read-only and saving waits until it\'s back.', action: { label: 'Retry', run: function () {
        App.setService('jellyfin', { state: 'ok', detail: 'Library online, 1,204 songs' }); App.banner.hide('sys-jellyfin'); App.toast('Jellyfin is back', { kind: 'ok', icon: 'check' });
      } } });
    },
    lastfm: function () {
      App.setService('lastfm', { state: 'down', queued: 12, detail: 'Down, 12 scrobbles queued' }); cfg.lastfm.pending = 12;
      App.banner.show({ id: 'sys-lastfm', kind: 'warn', icon: 'warning', text: 'Last.fm is down. Scrobbles are queued (12) and will retry.', action: { label: 'Retry now', run: function () {
        cfg.lastfm.pending = 0; App.setService('lastfm', { state: 'ok', queued: 0, detail: 'Connected as ' + cfg.lastfm.user + ', nothing waiting' });
        App.banner.hide('sys-lastfm'); App.toast('12 scrobbles sent to Last.fm', { kind: 'ok', icon: 'check' });
      } } });
    },
    soulseek: function () {
      App.setService('soulseek', { state: 'down', detail: 'Disconnected, reconnecting in 20 s' });
      App.banner.show({ id: 'sys-soulseek', kind: 'down', icon: 'soulseek', text: 'Soulseek disconnected. Reconnecting in 20 s; saved songs still play.', action: { label: 'Reconnect now', run: function () {
        App.setService('soulseek', { state: 'ok', detail: 'Reconnected just now' }); App.banner.hide('sys-soulseek'); App.toast('Soulseek reconnected', { kind: 'ok', icon: 'soulseek' });
      } } });
    },
    cache: function (warn) {
      var c = cfg.cache; c.usedGB = c.capGB; c.breakdown = warn ? { playing: c.capGB, recent: 0, evictable: 0 } : { playing: .4, recent: c.capGB - 1.2, evictable: .8 };
      App.setService('cache', { state: 'warn', usedGB: c.capGB, capGB: c.capGB, detail: c.capGB + ' of ' + c.capGB + ' GB, ' + (warn ? 'everything in use' : 'evicting oldest audio') });
      App.banner.show({ id: 'sys-cache', kind: warn ? 'warn' : 'info', icon: warn ? 'warning' : 'info', text: warn ? 'Cache full and every item is in use, so playback may pause.' : 'Cache full. The oldest audio is being evicted.', action: { label: 'Cache settings', run: function () { App.go('/settings/cache'); } } });
    }
  };
  /* real browser offline / online events show the same banner */
  window.addEventListener('offline', function () { BANNERS.offline(); });
  window.addEventListener('online', function () { if (App.banner.list().some(function (b) { return b.id === 'sys-offline'; })) { App.banner.hide('sys-offline'); App.toast('Back online', { kind: 'ok', icon: 'check' }); } });

  /* undo everything a demo state changed, so states never leak into each other */
  function resetDemo() {
    while (App.closeTop()) { /* close every sheet and dialog */ }
    App.closePopover();
    clearInterval(retryTimer); setSearchPaused(false);
    App.banner.clear();
    App.store.set('services', copy(M.services));
    cfg.cache = copy(M.settings.cache); cfg.lastfm.pending = M.settings.lastfm.pending;
    cfg.lastfm.state = 'connected'; statsUi.reset = null; taste = copy(M.stats.taste);
    cfg.app.install = detectInstall(); cfg.app.desktop = !!window.__TAURI__;
    cfg.sharing.mode = 'library'; cfg.library.naming = M.settings.library.naming;
    App.store.set('followed', copy(followed0));
  }
  function later(fn) { return function () { setTimeout(fn, 0); }; }

  /* =====================================================================
     9. States (listed by the switcher)
     ===================================================================== */
  App.registerStates('system', [
    { id: 'system-stats', label: 'Stats: 7 days', route: '/stats?period=7d' },
    { id: 'system-stats-12m', label: 'Stats: 12 months', route: '/stats?period=12m' },
    { id: 'system-stats-loading', label: 'Stats: loading', route: '/stats' },
    { id: 'system-stats-empty', label: 'Stats: not enough data yet', route: '/stats' },
    { id: 'system-stats-reset-confirm', label: 'Stats: reset taste (confirm)', route: '/stats', apply: later(function () { var b = $('[data-act="sys-taste-reset"]'); if (b) b.click(); }) },
    { id: 'system-stats-reset', label: 'Stats: taste profile reset', route: '/stats', apply: function () { statsUi.reset = copy(M.stats.taste); taste = { artists: [], tags: [] }; } },
    { id: 'system-settings-index', label: 'Settings: section list (phone)', route: '/settings' },
    { id: 'system-settings-playback', label: 'Settings: playback, loudness target', route: '/settings/playback', apply: function () { cfg.playback.normalisation = 'loudness'; cfg.playback.crossfade = 6; App.store.set('crossfadeSec', 6); } },
    { id: 'system-settings-sources', label: 'Settings: sources', route: '/settings/sources' },
    { id: 'system-cache-full', label: 'Settings: cache full, all in use', route: '/settings/cache', apply: function () { BANNERS.cache(true); } },
    { id: 'system-settings-library', label: 'Settings: library and naming preview', route: '/settings/library' },
    { id: 'system-naming-error', label: 'Settings: naming template error', route: '/settings/library', apply: function () { cfg.library.naming = '{artist}/{year} - {albm}/{track:02} - {title}.{ext}'; } },
    { id: 'system-sharing-none', label: 'Settings: sharing nothing', route: '/settings/sharing', apply: function () { cfg.sharing.mode = 'none'; } },
    { id: 'system-lastfm-off', label: 'Last.fm: not connected', route: '/settings/lastfm', apply: function () { cfg.lastfm.state = 'off'; } },
    { id: 'system-lastfm-waiting', label: 'Last.fm: waiting for approval', route: '/settings/lastfm', apply: function () { cfg.lastfm.state = 'waiting'; } },
    { id: 'system-lastfm-connected', label: 'Last.fm: connected, scrobbles queued', route: '/settings/lastfm', apply: function () { cfg.lastfm.state = 'connected'; } },
    { id: 'system-lastfm-error', label: 'Last.fm: error', route: '/settings/lastfm', apply: function () { cfg.lastfm.state = 'error'; } },
    { id: 'system-settings-discovery', label: 'Settings: discovery sources', route: '/settings/discovery' },
    { id: 'system-settings-appearance', label: 'Settings: appearance', route: '/settings/appearance' },
    { id: 'system-settings-app-desktop', label: 'Settings: App inside the desktop wrapper', route: '/settings/app?titlebar=win' },
    { id: 'system-settings-shortcuts', label: 'Settings: shortcuts list', route: '/settings/shortcuts' },
    { id: 'system-settings-app', label: 'Settings: app, install available', route: '/settings/app', apply: function () { cfg.app.install = 'available'; } },
    { id: 'system-settings-app-installed', label: 'Settings: app, installed', route: '/settings/app', apply: function () { cfg.app.install = 'installed'; } },
    { id: 'system-settings-app-desktop', label: 'Settings: app, inside the desktop wrapper', route: '/settings/app', apply: function () { cfg.app.desktop = true; } },
    { id: 'system-shortcuts', label: 'Overlay: keyboard shortcuts', route: '/home', apply: later(function () { openShortcuts(''); }) },
    { id: 'system-shortcuts-filter', label: 'Overlay: shortcuts filtered', route: '/home', apply: later(function () { openShortcuts('queue'); }) },
    { id: 'system-palette', label: 'Command palette', route: '/home', apply: later(function () { openPalette(''); }) },
    { id: 'system-palette-query', label: 'Command palette: typing "lan"', route: '/home', apply: later(function () { openPalette('lan'); }) },
    { id: 'system-welcome', label: 'First run: welcome steps', route: '/welcome', apply: function () { welcome = { searched: false }; cfg.lastfm.state = 'off'; App.store.set('followed', {}); } },
    { id: 'system-gate', label: 'Token gate', route: '/home', apply: later(function () { openTokenGate({}); }) },
    { id: 'system-gate-rejected', label: 'Token gate: token rejected', route: '/home', apply: later(function () { openTokenGate({ error: 'Token rejected. Check for spaces and try again.' }); }) },
    { id: 'system-desktop-connect', label: 'Desktop first run: connect', route: '/home', apply: later(function () { openTokenGate({ desktop: true }); }) },
    { id: 'system-desktop-testing', label: 'Desktop first run: testing', route: '/home', apply: later(function () { openTokenGate({ desktop: true, url: 'https://music.home.arpa', testNow: true, slow: true }); }) },
    { id: 'system-desktop-unreachable', label: 'Desktop first run: unreachable', route: '/home', apply: later(function () { openTokenGate({ desktop: true, url: 'https://offline.home.arpa', testNow: true }); }) },
    { id: 'system-install-toast', label: 'PWA: install toast', route: '/home', apply: later(installToast) },
    { id: 'system-ios-sheet', label: 'PWA: iOS install instructions', route: '/settings/app', apply: function () { cfg.app.install = 'ios'; setTimeout(openIosSheet, 0); } },
    { id: 'system-update-toast', label: 'PWA: update available toast', route: '/home', apply: later(updateToast) },
    { id: 'system-banner-unreachable', label: 'Banner: server unreachable', route: '/home', apply: function () { BANNERS.unreachable(); } },
    { id: 'system-banner-offline', label: 'Banner: browser offline', route: '/home', apply: function () { BANNERS.offline(); } },
    { id: 'system-banner-jellyfin', label: 'Banner: Jellyfin down', route: '/home', apply: function () { BANNERS.jellyfin(); } },
    { id: 'system-banner-lastfm', label: 'Banner: Last.fm down', route: '/home', apply: function () { BANNERS.lastfm(); } },
    { id: 'system-banner-soulseek', label: 'Banner: Soulseek disconnected', route: '/home', apply: function () { BANNERS.soulseek(); } },
    { id: 'system-banner-cache', label: 'Banner: cache full, evicting', route: '/home', apply: function () { BANNERS.cache(false); } },
    { id: 'system-banner-cache-warn', label: 'Banner: cache full, all in use', route: '/home', apply: function () { BANNERS.cache(true); } },
    { id: 'system-banner-all', label: 'Banners: everything degraded', route: '/home', apply: function () { ['soulseek', 'jellyfin', 'lastfm'].forEach(function (k) { BANNERS[k](); }); BANNERS.cache(false); BANNERS.unreachable(); } }
  ]);

  /* =====================================================================
     10. State switcher (PROTOTYPE ONLY: not part of the app; styles live in system.css)
     ===================================================================== */
  var ORDER = ['player', 'discover', 'now', 'library', 'system', 'classic'];
  var sw = { open: !!App.storage('switcherOpen'), q: '' };
  function stateHref(s) { return '#' + s.route + (s.route.indexOf('?') >= 0 ? '&' : '?') + 'state=' + s.id; }
  function switcherList() {
    var q = sw.q.trim().toLowerCase(), cur = App.current.state, any = false;
    var keys = ORDER.filter(function (k) { return App.states[k]; }).concat(Object.keys(App.states).filter(function (k) { return ORDER.indexOf(k) < 0; }));
    var out = keys.map(function (k) {
      var list = App.states[k].filter(function (s) { return !q || (k + ' ' + s.label + ' ' + s.id).toLowerCase().indexOf(q) >= 0; });
      if (!list.length) return '';
      any = true;
      return html`<section class="sys-sw__group"><h3 class="sys-sw__key">${k} <span class="num">${list.length}</span></h3><ul>${list.map(function (s) {
        return html`<li><a class="sys-sw__link" href="${stateHref(s)}" data-state-id="${s.id}" ${s.id === cur ? raw('aria-current="true"') : ''}>${s.label}</a></li>`;
      })}</ul></section>`;
    });
    return any ? html`${out}` : html`<p class="sys-sw__none">No state matches "${sw.q}".</p>`;
  }
  function switcherMarkup() {
    var p = App.prefs(), night = App.theme() === 'night', reduced = App.reducedMotion();
    return html`<button type="button" class="sys-sw__btn" aria-expanded="${sw.open ? 'true' : 'false'}" aria-controls="sys-sw-panel" data-sw-toggle>${icon('grid', null, 'ic-16')}<span>states</span></button>
      <section class="sys-sw__panel s-panel" id="sys-sw-panel" aria-label="Prototype states" ${sw.open ? '' : raw('hidden')}>
        <header class="sys-sw__head"><div><h2 class="sys-sw__title">states</h2><p class="sys-sw__note">Prototype only, not part of the app</p></div>
          <button type="button" class="btn-icon btn-icon--bare btn-icon--sm" data-sw-toggle aria-label="Close states" data-tip="Close">${icon('close', null, 'ic-16')}</button></header>
        <div class="sys-sw__tools">
          <button type="button" class="pill-filter" data-sw="theme" aria-pressed="${night ? 'true' : 'false'}">${icon(night ? 'moon' : 'sun')}Night</button>
          <button type="button" class="pill-filter" data-sw="motion" aria-pressed="${reduced ? 'true' : 'false'}">${icon('equaliser')}Reduce motion</button>
          <button type="button" class="pill-filter" data-sw="design" aria-label="Design: ${App.designs[p.design].label}. Switch design">${icon('sparkle')}${App.designs[p.design].label}</button>
          <button type="button" class="pill-filter" data-sw="reset">${icon('undo')}Clear state</button>
        </div>
        <div class="search-field sys-sw__filter">${icon('search')}<label class="vh" for="sys-sw-q">Filter states</label><input class="input" id="sys-sw-q" type="search" placeholder="Filter states" autocomplete="off" value="${sw.q}"></div>
        <div class="sys-sw__list" id="sys-sw-list">${switcherList()}</div>
      </section>`;
  }
  function paintSwitcher(focusSel) {
    var host = $('#sys-switcher');
    if (!host) { host = document.createElement('div'); host.id = 'sys-switcher'; host.className = 'sys-sw'; document.body.appendChild(host); wireSwitcher(host); }
    host.innerHTML = String(switcherMarkup());
    if (focusSel) { var f = $(focusSel, host); if (f) f.focus(); }
  }
  function wireSwitcher(host) {
    host.addEventListener('click', function (e) {
      var t = e.target.closest('[data-sw-toggle]');
      if (t) { sw.open = !sw.open; App.storage('switcherOpen', sw.open); paintSwitcher(sw.open ? '#sys-sw-q' : '.sys-sw__btn'); return; }
      var tool = e.target.closest('[data-sw]');
      if (tool) {
        var k = tool.getAttribute('data-sw');
        if (k === 'theme') App.toggleTheme();
        if (k === 'motion') App.setPref('motion', App.reducedMotion() ? 'full' : 'reduce');
        if (k === 'design') { var a = App.designOrder; App.setPref('design', a[(a.indexOf(App.design()) + 1) % a.length]); }
        if (k === 'reset') { resetDemo(); App.go(App.current.path, { replace: true }); }
        paintSwitcher('[data-sw="' + k + '"]');
        return;
      }
      var link = e.target.closest('[data-state-id]');
      if (link) {
        e.preventDefault();
        var href = link.getAttribute('href');
        resetDemo();
        if (location.hash === href) App.go(href.slice(1), { replace: true }); else location.hash = href;
      }
    });
    host.addEventListener('input', function (e) { if (e.target.id === 'sys-sw-q') { sw.q = e.target.value; repaint('#sys-sw-list', switcherList()); } });
    host.addEventListener('keydown', function (e) { if (e.key === 'Escape' && sw.open) { e.stopPropagation(); sw.open = false; App.storage('switcherOpen', false); paintSwitcher('.sys-sw__btn'); } });
  }
  App.on('ready', function () {
    if (/(^|&)switcher=0/.test(location.hash.split('?')[1] || '')) return;
    paintSwitcher();
    App.on('states', function () { if ($('#sys-sw-list')) repaint('#sys-sw-list', switcherList()); });
    App.on('route', function () { if ($('#sys-sw-list')) repaint('#sys-sw-list', switcherList()); });
    App.on('prefs', function () { if ($('#sys-switcher')) { var a = document.activeElement, sel = a && a.getAttribute && a.getAttribute('data-sw') ? '[data-sw="' + a.getAttribute('data-sw') + '"]' : null; paintSwitcher(sel && $('#sys-switcher').contains(a) ? sel : null); } });
  });
})();
