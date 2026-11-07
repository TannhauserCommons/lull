/* screens/system.js.
   Stats (/stats), Settings (/settings/:section?), the shortcuts overlay (?), the command palette (Ctrl/Cmd+K),
   PWA install / update, and the service banners. First-run setup and pairing are screens/setup.js.
   Reads: GET /api/stats, GET + PATCH /api/settings, GET + PATCH /api/config, GET /api/lastfm, store `services`
   and App.server (docs/context/web.md, docs/context/api.md).
   Sections: 0 helpers + generic controls, 1 stats, 2 settings, 3 shortcuts overlay, 4 command palette,
   7 PWA, 8 banners. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var P = App.player;
  var SETTINGS = '/api/settings', CONFIG = '/api/config', LASTFM = '/api/lastfm';

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
  /* a service's detail as one clause of a banner: its first line, at most about 200 characters (the hub already keeps
     it short; an older hub pasted slskd's stack traces, which filled a phone's screen) */
  function clause(text) {
    var t = String(text || '').split('\n')[0].replace(/\.\s*$/, '');
    return t.length > 200 ? t.slice(0, 200).replace(/\s+\S*$/, '') + '…' : t;
  }
  /* replace a part of the page and put focus back on an equivalent control */
  function repaint(sel, markup, focusSel) {
    var el = $(sel); if (!el) return;
    App.patch(el, markup);
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
  function locked(path) { return isCfg(path) ? cfgLocked(path.slice(7)) : isConfig(lookup(path)); }

  /* ---------- hub config (ADR-0002): GET /api/config { items: [{ key, value, set, source: env|stored|default, secret,
     restart }] } (a secret's value is always null: `set` says whether it has one), changed with
     PATCH /api/config { KEY: "value"|null } (values are strings). A control reads it by the path 'config.KEY'.
     Keys the environment sets are read-only; secrets only say whether they are set and are never shown.
     Until the hub answers /api/config, each key shows read-only from /api/settings (CFG_SETTING). ---------- */
  var CFG_SETTING = { LULL_SHARE: 'sharing.mode', LULL_CACHE_MAX_GB: 'cache.capGB', LULL_MUSIC_DIR: 'library.musicFolder',
    LULL_NAMING: 'library.naming', LULL_SLSKD_URL: 'sources.slskdUrl', LULL_SLSKD_API_KEY: 'sources.slskdApiKey',
    LULL_JELLYFIN_URL: 'library.jellyfinUrl', LULL_JELLYFIN_USER: 'library.jellyfinUser', LULL_JELLYFIN_API_KEY: 'library.jellyfinApiKey',
    LULL_LASTFM_API_KEY: 'lastfm.apiKey', LULL_LASTFM_API_SECRET: 'lastfm.apiSecret', LULL_LISTENBRAINZ_USER: 'discovery.listenbrainzUser' };
  function isCfg(path) { return path.indexOf('config.') === 0; }
  function cfgItem(key) { var c = M.peek(CONFIG); return c && (c.items || []).filter(function (i) { return i.key === key; })[0]; }
  function cfgLocked(key) { var i = cfgItem(key); return i ? i.source === 'env' : true; }
  /* an unset key (value null) runs on its default, which /api/settings reports as the value in effect */
  function cfgDefault(key) { var v = val(lookup(CFG_SETTING[key] || '')); return v == null || v === '' ? null : v; }
  function cfgValue(key) { var i = cfgItem(key); return i && i.value != null ? i.value : cfgDefault(key); }
  /* PATCH one key; the answer is the whole list again. A refusal (400) names what is wrong in a sentence. */
  function cfgSave(key, value) {
    var body = {}; body[key] = value;
    return M.api(CONFIG, { method: 'PATCH', body: body }).then(function (res) {
      var c = M.peek(CONFIG); if (c && res && res.items) c.items = res.items;
      var i = cfgItem(key);
      if (i && i.restart) App.toast('Saved. lull restarts its services to use it, which takes a moment.', { kind: 'ok', icon: 'refresh' });
      else App.announce('Saved');
      App.emit('config', key);
      return res;
    });
  }
  /* what a locked or fallback key shows in place of its value */
  function cfgText(key) {
    var i = cfgItem(key);
    if (!i) return configText(lookup(CFG_SETTING[key] || ''));
    if (i.secret) return i.set ? 'set' : 'not set';
    var v = i.value == null || i.value === '' ? cfgDefault(key) : i.value;
    return v == null ? 'not set' : String(v);
  }
  /* a section that edits hub config says once when the hub can't (an older hub, or /api/config failing) */
  function cfgNotice() {
    var r = M.get(CONFIG);
    if (!r.error) return '';
    return html`<p class="sys-note">${icon('lock', null, 'ic-16')}<span>These come from the hub's config and show read-only here: ${clause(r.error.message)}.</span></p>`;
  }
  /* a text, number or secret field for one config key: saved when it changes (Enter or leaving the field) */
  function cfgField(key, label, help, o) {
    o = o || {};
    var r = M.get(CONFIG), i = cfgItem(key);
    if (r.loading) return row(label, help, ui.skeleton('text', 1));
    if (!i || i.source === 'env') {
      var why = i ? 'Set in the hub\'s environment, so it can\'t change here.' : 'Read-only until the hub can change its config.';
      return row(label, (help ? help + ' ' : '') + why, lockedValue(label, cfgText(key) + (o.unit && i && i.value != null ? ' ' + o.unit : '')), { cls: 'sys-row--locked' });
    }
    var id = App.uid('sys-cfg'), type = i.secret ? 'password' : o.type || 'text';
    var restart = i.restart ? ' Changing it restarts lull\'s services for a moment.' : '';
    return html`<div class="sys-row sys-row--cfg">
      <div class="sys-row__text"><label class="sys-row__label" for="${id}">${label}</label>${help || restart ? html`<span class="sys-row__help" id="${id}-h">${help}${restart}</span>` : ''}</div>
      <div class="sys-row__ctl"><span class="sys-cfg">${o.unit ? html`<span class="sys-cfg__unit">${o.unit}</span>` : ''}<input class="input" id="${id}" type="${type}" data-cfg="${key}" ${type === 'number' ? raw('inputmode="numeric" min="0"') : ''}
          value="${i.secret || i.value == null ? '' : String(i.value)}" placeholder="${i.secret ? (i.set ? 'Set. Type a new one to replace it' : 'Not set') : cfgDefault(key) != null ? String(cfgDefault(key)) : o.placeholder || ''}"
          autocomplete="${i.secret ? 'new-password' : 'off'}" spellcheck="false" aria-describedby="${id}-h ${id}-e"></span>
        <span class="field__error" id="${id}-e" hidden>${icon('warning', null, 'ic-16')}<span data-msg></span></span></div></div>`;
  }
  function cfgError(input, msg) {
    var e = document.getElementById(input.id + '-e'); if (!e) return;
    $('[data-msg]', e).textContent = msg || ''; e.hidden = !msg;
    input.setAttribute('aria-invalid', msg ? 'true' : 'false');
  }
  document.addEventListener('change', function (e) {
    var input = e.target.closest && e.target.closest('[data-cfg]'); if (!input) return;
    var key = input.getAttribute('data-cfg'), i = cfgItem(key), v = input.value.trim();
    if (i && i.secret && !v) return;                       // an empty secret field means "keep what is set"
    cfgError(input, ''); input.setAttribute('aria-busy', 'true');
    cfgSave(key, v === '' ? null : v).then(function () {
      input.removeAttribute('aria-busy');
      if (i && i.secret) { input.value = ''; input.placeholder = 'Set. Type a new one to replace it'; }
    }, function (err) { input.removeAttribute('aria-busy'); cfgError(input, err.message); });
  });

  /* values that are not a plain server setting: per-device prefs (device.*) and derived ones; path -> { get, set } */
  var custom = {};
  /* UI side effects of a change (repaint a dependent part, announce); run at once */
  var effects = {};
  var fmtRange = {};                              // path -> fn(value) -> visible text / aria-valuetext

  function get(path) { var c = custom[path]; return c && c.get ? c.get() : isCfg(path) ? cfgValue(path.slice(7)) : val(lookup(path)); }
  function setVal(path, v) {
    if (typeof get(path) === 'number') v = +v;
    if (custom[path] && custom[path].set) custom[path].set(v);
    else if (isCfg(path)) cfgSave(path.slice(7), String(v)).catch(function (err) { App.toast('Couldn\'t change that: ' + err.message, { kind: 'error' }); App.refresh(); });
    else save(one(path, v));
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
      loudnessTarget: +val(pb.loudnessTarget) || -14, radioWhenQueueEnds: !!val(pb.radioWhenQueueEnds), volumeStep: +val(pb.volumeStep) || 5,
      prefetchCount: val(pb.prefetchCount) == null ? 2 : +val(pb.prefetchCount) };
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
    if (isCfg(path) && M.get(CONFIG).loading) return ui.skeleton('chip', 1);
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
  /* on first visit: screens/stats.js (and stats.css) has the screen, and registers the route again with it */
  App.needStats = function () { return App.needScript('screens/stats.js', 'Stats', 'screens/stats.css'); };
  App.route('/stats', { need: App.needStats, render: function () { return ''; }, nav: 'stats', title: 'Stats',
    place: function () { return App.place('stats', '', 'your top songs'); } });

  /* =====================================================================
     2. Settings (/settings/:section?): GET + PATCH /api/settings; appearance is per device
     ===================================================================== */
  /* Settings sections that live in their own file register here; they build with the same controls
     (screens/appearance.js, owned by the themes work: docs/context/workspace.md). */
  App.settingsSections = App.settingsSections || {};
  App.settingsKit = { row: row, choice: choice, showChoice: showChoice, switchRow: switchRow, group: group, custom: custom, get: get, effects: effects,
    range: range, fmtRange: fmtRange,
    statusLine: statusLine, spinnerLine: spinnerLine, lockedValue: lockedValue, readError: readError, repaint: repaint,
    cfgField: cfgField, cfgNotice: cfgNotice, cfgItem: cfgItem, cfgSave: cfgSave, clause: clause,
    num: num, hours: hours, pct: pct, share: share };

  var SECTIONS = [
    ['playback', 'Playback', 'volume-2', 'Gapless, crossfade, loudness, equaliser'],
    ['sources', 'Sources', 'soulseek', 'Streaming and saving rules, peers, YouTube'],
    ['search', 'Search', 'search', 'File results'],
    ['cache', 'Cache', 'grid', 'Size, usage, clear'],
    ['library', 'Library and saving', 'library', 'Music folder or Jellyfin, names, duplicates'],
    ['sharing', 'Sharing', 'export', 'What Soulseek peers can download'],
    ['lastfm', 'Last.fm', 'radio', 'Scrobbling and history'],
    ['discovery', 'Discovery', 'sparkle', 'Sources, follows, adventurousness'],
    ['privacy', 'Privacy', 'eye-off', 'Private listening on this device'],
    ['devices', 'Devices', 'connect', 'Paired devices, add one, Subsonic apps'],
    ['appearance', 'Appearance', 'sun', 'Design, theme, motion, text size, Classic skins'],
    ['sounds', 'Sounds', 'bell', 'Sounds, sound packs, hidden surprises'],
    ['shortcuts', 'Shortcuts', 'keyboard', 'Every keyboard shortcut'],
    ['app', 'App', 'device', 'Install, version, server health']
  ];
  function sectionOf(id) { return SECTIONS.filter(function (s) { return s[0] === id; })[0]; }
  /* a section body that needs the server's settings: skeleton, honest error, or the body. `early`: a section whose words
     don't depend on the values is drawn while they load, its controls hidden (.sys-pending, system.css), so every row
     has its final size from the start and nothing moves when they arrive. A section that says things about the values
     ("needs a newer hub", "sharing nothing") waits behind the skeleton, as that would be false until they come */
  function withSettings(body, early) {
    var r = M.get(SETTINGS);
    if (r.loading) return early ? html`<div class="sys-pending" aria-busy="true">${body()}</div>` : ui.skeleton('text', 6);
    if (r.error) return readError(r.error, SETTINGS, 'Couldn\'t load settings');
    return body();
  }

  /* ---------- playback ---------- */
  fmtRange['playback.crossfade'] = function (v) { return v ? v + ' s' : 'Off'; };
  effects['playback.normalisation'] = function (v) { $$('[data-when="loudness"]').forEach(function (r) { r.hidden = v !== 'loudness'; }); };
  /* the equaliser's panel comes with its code (screens/eq.js), loaded on first use */
  function eqPanel() {
    if (App.eq) return App.eq.panel({ full: true });
    App.needEq().then(function () { if (/^\/settings(\/playback)?$/.test(App.current.path)) App.refresh(); }, function () {});
    return ui.skeleton('card', 1);
  }
  function secPlayback() {
    return html`
      ${switchRow('playback.gapless', 'Gapless playback', 'Where the source allows. Lossy files from Soulseek may still have a short gap.')}
      ${row('Crossfade', 'Blend the end of one song into the next. Albums marked gapless never crossfade.', range('playback.crossfade', 'Crossfade, seconds', 0, 12, 1))}
      ${row('Loudness normalisation', 'ReplayGain reads tags in the file. Loudness measures files that have no tags.', choice('playback.normalisation', 'Loudness normalisation', [['off', 'Off'], ['replaygain-track', 'Track'], ['replaygain-album', 'Album'], ['loudness', 'Loudness']], 'sys-seg--4'))}
      ${row('Target loudness', 'Quieter targets leave more headroom for dynamic music.', choice('playback.loudnessTarget', 'Target loudness', [[-18, '-18 LUFS'], [-14, '-14 LUFS'], [-11, '-11 LUFS']]), { when: 'loudness', hidden: get('playback.normalisation') !== 'loudness', cls: 'sys-row--sub' })}
      ${row('Volume step', 'How far Up, Down and the D-pad move the volume.', choice('playback.volumeStep', 'Volume step', [[2, '2%'], [5, '5%'], [10, '10%']]))}
      ${switchRow('playback.radioWhenQueueEnds', 'Continue with radio when the queue ends', 'Starts a radio from the last song so the music keeps going.')}
      ${lookup('playback.prefetchCount') == null ? '' : row('Preload upcoming songs', 'Your server starts getting the next songs from Soulseek while this one plays, so skipping plays at once. More uses more of its bandwidth. Saved songs never need it.',
        choice('playback.prefetchCount', 'Preload upcoming songs', [[0, 'Off'], [1, '1'], [2, '2'], [3, '3']], 'sys-seg--4'))}
      ${eqPanel()}`;
  }

  /* ---------- sources (ADR-0008): a streaming and a saving profile, then the peer limits ----------
     sources.stream / sources.save = { minimum: { losslessOnly, kbps, formats }, pick, below } (+ autoUpgrade,
     upgradeEveryH for saving). A hub from before the profiles has neither: the section says so. */
  var SOURCE_FORMATS = ['flac', 'wav', 'aiff', 'm4a', 'aac', 'mp3', 'ogg', 'opus'];
  var BELOW = {
    stream: [['playBest', 'Play the best one'], ['keepLooking', 'Keep looking'], ['ask', 'Ask me']],
    save: [['saveBestUpgrade', 'Save the best, upgrade later'], ['wait', 'Wait for one'], ['ask', 'Ask me']]
  };
  fmtRange['sources.stallTimeout'] = function (v) { return v + ' s'; };
  ['stream', 'save'].forEach(function (kind) {
    effects['sources.' + kind + '.minimum.losslessOnly'] = function (v) { $$('[data-when="' + kind + '-kbps"]').forEach(function (r) { r.hidden = !!v; }); };
  });
  effects['sources.save.autoUpgrade'] = function (v) { $$('[data-when="upgrade"]').forEach(function (r) { r.hidden = !v; }); };
  /* the streaming cap never sits below the streaming minimum (the hub refuses that): moving one past the other
     moves both, in one PATCH */
  custom['sources.stream.maxKbps'] = { set: function (v) {
    var c = { 'sources.stream.maxKbps': v }, min = +get('sources.stream.minimum.kbps') || 0;
    if (v && min > v) c['sources.stream.minimum.kbps'] = v;
    save(c); if (c['sources.stream.minimum.kbps'] != null) App.refresh();
  } };
  custom['sources.stream.minimum.kbps'] = { set: function (v) {
    var c = { 'sources.stream.minimum.kbps': v }, max = +get('sources.stream.maxKbps') || 0;
    if (max && v > max) c['sources.stream.maxKbps'] = v;
    save(c); if (c['sources.stream.maxKbps'] != null) App.refresh();
  } };
  /* formats as toggle chips: none pressed means any format */
  function formatChips(path) {
    var on = get(path) || [];
    return html`<div class="cluster sys-formats" role="group" aria-label="Formats that count" data-formats="${path}">${SOURCE_FORMATS.map(function (f) {
      return html`<button type="button" class="chip" aria-pressed="${on.indexOf(f) >= 0 ? 'true' : 'false'}" data-format="${f}">${icon('check', null, 'chip__check')}${f.toUpperCase()}</button>`;
    })}</div>`;
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('[data-formats] [data-format]'); if (!b) return;
    var g = b.closest('[data-formats]'), path = g.getAttribute('data-formats');
    b.setAttribute('aria-pressed', b.getAttribute('aria-pressed') === 'true' ? 'false' : 'true');
    setVal(path, $$('[data-format][aria-pressed="true"]', g).map(function (x) { return x.getAttribute('data-format'); }));
  });
  function profileRows(kind) {
    var base = 'sources.' + kind, ll = !!get(base + '.minimum.losslessOnly');
    return html`
      ${switchRow(base + '.minimum.losslessOnly', 'Lossless only', 'Only FLAC and other lossless files are good enough.')}
      ${row('Minimum bitrate', 'Lossy files below this aren\'t good enough. Lossless always is.', choice(base + '.minimum.kbps', 'Minimum bitrate', [[128, '128'], [192, '192'], [256, '256'], [320, '320']]), { when: kind + '-kbps', hidden: ll, cls: 'sys-row--sub' })}
      ${kind === 'stream' && lookup('sources.stream.maxKbps') != null ? row('Maximum bitrate', 'Streaming prefers copies at or under this, to save data: lossless counts as over any cap. Saving is never capped.',
        choice('sources.stream.maxKbps', 'Maximum bitrate', [[0, 'No cap'], [128, '128'], [192, '192'], [256, '256'], [320, '320']], 'sys-seg--wrap')) : ''}
      ${row('Formats', 'Only these count. With none picked, any format does.', formatChips(base + '.minimum.formats'), { cls: 'sys-row--stack' })}
      ${row('Which copy', 'Among the copies that are good enough.', choice(base + '.pick', 'Which copy', [['fastest', 'Fastest'], ['highest', 'Highest quality']]))}
      ${row('If none is good enough', kind === 'stream' ? 'Playing the best one marks it as below your minimum.' : 'Saving the best one marks it, and auto-upgrade replaces it later.',
        choice(base + '.below', 'If none is good enough', BELOW[kind], 'sys-seg--wrap'), { cls: 'sys-row--stack' })}`;
  }
  function secSources() {
    var profiles = lookup('sources.stream') && lookup('sources.save');
    return html`
      ${profiles ? html`${group('streaming')}${profileRows('stream')}
        ${group('saving')}${profileRows('save')}
        ${switchRow('sources.save.autoUpgrade', 'Upgrade saved songs', 'Look again for saved songs below your minimum, and replace them when a good enough copy turns up. One search a minute at most.')}
        ${row('Look again every', 'How long to wait before searching for the same song again.', choice('sources.save.upgradeEveryH', 'Look again every', [[6, '6 h'], [12, '12 h'], [24, 'Day'], [168, 'Week']]), { when: 'upgrade', hidden: !get('sources.save.autoUpgrade'), cls: 'sys-row--sub' })}`
      : html`<p class="sys-note">${icon('info', null, 'ic-16')}<span>Streaming and saving rules need a newer hub. Update lull on the server and they show up here.</span></p>`}
      ${group('peers')}
      ${row('Longest peer queue', 'Skip peers with more people waiting than this.', stepper('sources.maxPeerQueue', 'Longest peer queue', 0, 50, 5, 'waiting'))}
      ${row('Stall timeout', 'If a peer stops sending for this long, switch to the next copy.', range('sources.stallTimeout', 'Stall timeout, seconds', 5, 60, 5))}
      ${lookup('sources.switchToBetter') != null ? switchRow('sources.switchToBetter', 'Switch to a better copy while a song plays',
        'When a song starts on a lesser copy (from YouTube, below your minimum, or a slower peer after one failed), the server keeps fetching the best one. Once it is ready, the song moves over to it at a quiet moment, lined up so you don\'t hear a jump. If the two can\'t be lined up, the better copy plays from the next play instead.') : ''}
      ${youtubeRows()}
      ${group('soulseek connection')}
      ${cfgNotice()}${App.hubKit.soulseekRows()}
      ${cfgField('LULL_SLSKD_URL', 'slskd address', 'lull reaches Soulseek through slskd, which signs in with your Soulseek account.', { type: 'url' })}
      ${cfgField('LULL_SLSKD_API_KEY', 'slskd API key')}`;
  }

  /* ---------- YouTube (docs/context/youtube.md): a second source through yt-dlp ----------
     sources.youtube = { mode: off | fallback | startFast, format: opus | aac, match: official | close | loose, waitS, saves,
     autoUpdate }. GET /api/youtube says how it stands and which yt-dlp is in use; POST /api/youtube/update checks now. */
  var YOUTUBE = '/api/youtube';
  var yt = null;                                   // { loading, error, data } of GET /api/youtube
  fmtRange['sources.youtube.waitS'] = function (v) { return v + ' s'; };
  effects['sources.youtube.mode'] = function (v) {
    $$('[data-when="youtube"]').forEach(function (r) { r.hidden = v === 'off'; });
    $$('[data-when="youtube-wait"]').forEach(function (r) { r.hidden = v !== 'fallback'; });
    loadYoutube();
  };
  effects['sources.youtube.autoUpdate'] = function (v) {
    $$('[data-act="sys-ytdlp-update"]').forEach(function (b) { b.disabled = !v; });
    loadYoutube();
  };
  function loadYoutube() {
    yt = yt && yt.data ? yt : { loading: true };
    M.api(YOUTUBE).then(function (d) { yt = { data: d }; repaint('#sys-youtube', youtubeState()); },
      function (err) { yt = { error: err && err.message || String(err) }; repaint('#sys-youtube', youtubeState()); });
  }
  /* the `youtube` service (backing off, yt-dlp updated) changes while the page is open */
  App.on('server:service', function (e) { if (e && e.name === 'youtube' && $('#sys-youtube')) loadYoutube(); });
  function youtubeState() {
    if (!yt || yt.loading) return spinnerLine('Asking the server');
    if (yt.error) return statusLine('down', yt.error);
    var d = yt.data, y = d.ytdlp || {};
    var using = !y.version ? 'yt-dlp isn\'t working on the server.'
      : y.updated ? 'Using yt-dlp ' + y.version + ', fetched by the server' + (y.pinnedVersion ? ' (built in: ' + y.pinnedVersion + ').' : '.')
      : 'Using the built-in yt-dlp ' + y.version + '.';
    var checked = y.checkedAt ? ' Last checked ' + new Date(y.checkedAt * 1000).toLocaleString() + (y.latest ? '; the newest is ' + y.latest + '.' : '.') : '';
    return html`<div>${statusLine(d.state, d.detail)}</div><span class="sys-row__help">${using}${checked}${y.error ? ' ' + y.error : ''}</span>`;
  }
  App.action('sys-ytdlp-update', function (el) {
    var before = yt && yt.data && yt.data.ytdlp ? yt.data.ytdlp.version : null;
    el.disabled = true;
    M.api(YOUTUBE + '/update', { method: 'POST' }).then(function (d) {
      el.disabled = false; yt = { data: d }; repaint('#sys-youtube', youtubeState());
      var y = d.ytdlp || {};
      if (y.error) App.toast('Couldn\'t update yt-dlp: ' + y.error, { kind: 'error' });
      else if (y.version && y.version !== before) App.toast('Updated yt-dlp to ' + y.version, { icon: 'check' });
      else if (y.latest && y.latest !== y.version) App.toast('Using yt-dlp ' + y.version + '; the newest is ' + y.latest, { icon: 'info' });
      else App.toast('yt-dlp ' + y.version + ' is the newest', { icon: 'check' });
    }, function (err) { el.disabled = false; App.toast('Couldn\'t check: ' + err.message, { kind: 'error' }); });
  });
  function youtubeRows() {
    if (!lookup('sources.youtube')) return '';
    var mode = get('sources.youtube.mode'), off = mode === 'off';
    loadYoutube();
    return html`${group('youtube')}
      ${row('Use YouTube', 'Off: songs only ever play from Soulseek. As a fallback: YouTube plays a song when no Soulseek peer has it, or none starts in time. Start fast: YouTube starts every song at once while the best Soulseek copy is fetched, for this play (with "Switch to a better copy" on), the next one and saving.',
        choice('sources.youtube.mode', 'Use YouTube', [['off', 'Off'], ['fallback', 'As a fallback'], ['startFast', 'Start fast, then upgrade']], 'sys-seg--wrap'), { cls: 'sys-row--stack' })}
      ${row('Wait for Soulseek', 'How long a song waits for a Soulseek peer before YouTube plays it.', range('sources.youtube.waitS', 'Wait for Soulseek, seconds', 5, 120, 5), { when: 'youtube-wait', hidden: mode !== 'fallback', cls: 'sys-row--sub' })}
      ${row('Format', 'Opus sounds a little better for its size. AAC plays on more devices. Either way it is YouTube\'s own audio, never converted.',
        choice('sources.youtube.format', 'YouTube format', [['opus', 'Opus'], ['aac', 'AAC']]), { when: 'youtube', hidden: off })}
      ${row('Which uploads count', 'Official audio comes from YouTube Music or the artist\'s own channel. Close also takes other uploads of the same song and length. Loose allows lengths up to 15 seconds apart. Live, cover, sped-up and nightcore versions never count unless the song is one.',
        choice('sources.youtube.match', 'Which uploads count', [['official', 'Official audio only'], ['close', 'Close'], ['loose', 'Loose']], 'sys-seg--wrap'), { when: 'youtube', hidden: off, cls: 'sys-row--stack' })}
      <div data-when="youtube" ${off ? raw('hidden') : ''}>${switchRow('sources.youtube.saves', 'Save from YouTube', 'When no Soulseek peer has a song, Save keeps the YouTube copy (AAC). Upgrade saved songs replaces it once a good Soulseek copy turns up. Saves never use YouTube while a peer has the song.')}</div>
      <div data-when="youtube" ${off ? raw('hidden') : ''}>${cfgField('LULL_ACOUSTID_KEY', 'AcoustID key', 'Checks songs no store has a preview of. Optional, free at acoustid.org.')}
        ${switchRow('sources.youtube.contribute', 'Contribute to AcoustID', 'Songs you confirm that AcoustID lacks are sent to it under your user key, so other servers can check them. Off: nothing is sent.')}
        ${cfgField('LULL_ACOUSTID_USER_KEY', 'AcoustID user key')}</div>
      ${switchRow('sources.youtube.autoUpdate', 'Keep yt-dlp up to date', 'lull reaches YouTube with a program called yt-dlp, and YouTube changes often. The server fetches the newest version once a day, checks it, and keeps the built-in one in case the new one fails.')}
      ${row('YouTube now', '', html`<div class="sys-inline sys-inline--col"><div id="sys-youtube" aria-live="polite">${youtubeState()}</div><button class="btn btn--sm" data-act="sys-ytdlp-update" ${get('sources.youtube.autoUpdate') ? '' : raw('disabled')}>${icon('refresh')}Check for a new yt-dlp</button></div>`, { cls: 'sys-row--stack' })}`;
  }

  /* ---------- search (ADR-0008): Soulseek's raw files stay in the source picker unless this adds a Files tab ---------- */
  function secSearch() {
    if (!lookup('search')) return html`<p class="sys-note">${icon('info', null, 'ic-16')}<span>Search settings need a newer hub. Update lull on the server and they show up here.</span></p>`;
    return switchRow('search.showFiles', 'Show file results', 'Adds a Files tab to search: the Soulseek folders and files behind the songs.');
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
      ${cfgNotice()}${cfgField('LULL_CACHE_MAX_GB', 'Cache size', 'Streamed audio is kept up to this size so replays start instantly.', { type: 'number', unit: 'GB' })}
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
  /* the latest rescan, as the Activity list has it (POST /api/library/rescan, then `job` events) */
  function rescanStatus() {
    var job = (App.store.get('activity') || []).filter(function (j) { return j.kind === 'rescan'; })[0];
    if (!job) return '';
    if (job.state === 'failed') return statusLine('down', job.error || job.detail || 'Rescan failed');
    if (job.state === 'done') return statusLine('ok', job.detail || 'Rescan finished');
    return spinnerLine(job.detail || 'Rescanning the library');
  }
  function namingBox() {
    var naming = get('config.LULL_NAMING'), folder = get('config.LULL_MUSIC_DIR');
    return naming ? html`<div class="inset sys-preview"><span class="sys-preview__label">Saves as</span><span class="sys-preview__path">${folder || ''}/${namingPreview(naming)}</span></div>` : '';
  }
  /* the source switch (folder or Jellyfin) is screens/hub.js; saving rows read hub config */
  function secLibrary() {
    return html`${App.hubKit.librarySource()}
      ${group('saving')}
      ${cfgNotice()}
      ${cfgField('LULL_MUSIC_DIR', 'Music folder', 'Saved songs are written here, in either mode.')}
      ${cfgField('LULL_NAMING', 'Folder and file names', 'Saved songs are named from this template: {artist}, {album}, {year}, {disc}, {track:02}, {title}, {ext}.')}
      <div id="sys-naming">${namingBox()}</div>
      ${row('When a song is already saved', 'What Save does when the library has the same song.', choice('library.duplicatePolicy', 'Duplicate policy', [['ask', 'Ask each time'], ['keep', 'Keep existing'], ['replace', 'Replace if better']], 'sys-seg--wrap'))}
      ${App.hubKit.likeSaves()}
      ${row('Rescan library', 'Picks up files added to the music folder outside lull.', html`<div class="sys-inline sys-inline--col"><button class="btn" data-act="jellyfin-rescan">${icon('refresh')}Rescan library now</button><div id="sys-rescan" class="sys-rescan" aria-live="polite">${rescanStatus()}</div></div>`)}`;
  }

  /* ---------- sharing: what is shared is server config; slots and speed are stored ---------- */
  fmtRange['sharing.uploadLimitKBs'] = function (v) { return v ? num(v) + ' KB/s' : 'No limit'; };
  function secSharing() {
    var on = get('config.LULL_SHARE') === 'library';
    return html`${row('Sharing now', '', on ? statusLine('ok', 'Sharing your saved library, read-only') : statusLine('warn', 'Sharing nothing'), { cls: 'sys-row--status' })}
      ${row('Share with Soulseek peers', 'Your saved library, read-only, or nothing. Peers can download songs; they can never change or delete anything.', choice('config.LULL_SHARE', 'What to share', [['library', 'Saved library, read-only'], ['none', 'Nothing']], 'sys-seg--wrap'))}
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
        App.toast('Last.fm disconnected', { group: 'lastfm', icon: 'radio' });
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
      ${withSettings(function () { return html`${cfgNotice()}${cfgField('LULL_LASTFM_API_KEY', 'API key')}${cfgField('LULL_LASTFM_API_SECRET', 'Shared secret')}`; })}`;
  }

  /* ---------- privacy: private listening is this device's switch on the hub (PATCH /api/devices/me), so it
     survives reloads and is enforced by the server: nothing played here is recorded or sent anywhere ---------- */
  custom['device.private'] = { get: function () { return !!App.store.get('privateListening'); }, set: function (v) {
    App.setPrivateListening(v).then(null, function (err) {
      App.toast('Couldn\'t change private listening: ' + err.message, { kind: 'error' });
      App.refresh();
    });
  } };
  function secPrivacy() {
    return html`${switchRow('device.private', 'Private listening on this device', 'While it is on, what you play here leaves no trace: it isn\'t added to History or Stats, doesn\'t shape recommendations, and nothing goes to Last.fm, not even what is playing now. Playback is unchanged. Other devices keep their own switch, and a song already playing when you turn it on stays as it was.')}
      <p class="sys-card__lede">A small <b>Private</b> tag shows beside the player while it is on, so it never stays on by accident.</p>`;
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
      ${cfgField('LULL_LISTENBRAINZ_USER', 'ListenBrainz user', 'Your public listens and similar listeners. Optional.')}
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
  /* ---------- the server's version and updates: GET /api/updates { version, build: { release, changes, commit,
     install }, latest: { version, url }, state: current|available|ahead|unknown, error } ---------- */
  var srvUpd = null;                               // { loading } | { data } | { error }
  function loadServerUpdate(check) {
    if (!srvUpd || !srvUpd.data) srvUpd = { loading: true };
    return M.api('/api/updates' + (check ? '?check=true' : '')).then(function (r) { srvUpd = { data: r }; },
      function (err) { srvUpd = { error: errText(err) }; }).then(function () { repaint('#sys-server-update', serverUpdateBody()); });
  }
  /* "0.2.4", or between releases "0.2.4 + 12 changes (abc1234)": what the build says it holds */
  function buildName(b) {
    if (!b || !b.changes) return App.product + ' ' + (b ? b.release : App.server.version);
    return App.product + ' ' + b.release + ' + ' + b.changes + (b.changes === 1 ? ' change' : ' changes') + (b.commit ? ' (' + b.commit + ')' : '');
  }
  App.action('sys-update-check', function (el) {
    el.disabled = true;
    loadServerUpdate(true).then(function () {
      var r = srvUpd.data;
      if (srvUpd.error) App.toast('Couldn\'t reach your server to check for updates', { kind: 'error' });
      else if (r.state === 'unknown') App.toast('Couldn\'t check for updates: ' + (r.error || 'GitHub didn\'t answer'), { kind: 'error' });
      else App.announce({ current: 'Your server is up to date', ahead: 'Your server follows the latest changes', available: App.product + ' ' + (r.latest && r.latest.version) + ' is out' }[r.state] || '');
    });
  });
  function serverUpdateBody() {
    var u = srvUpd;
    if (!u || u.loading) return html`<span class="px sys-version" id="sys-version">${App.server.version ? buildName(App.server.build) : 'not reported yet'}</span>${spinnerLine('Checking')}`;
    if (u.error) return html`<span class="px sys-version" id="sys-version">${App.server.version ? buildName(App.server.build) : 'not reported yet'}</span>${statusLine('down', 'Couldn\'t ask your server: ' + u.error)}${checkButton('Try again')}`;
    var r = u.data, b = r.build || {}, latest = r.latest && r.latest.version;
    var how = {
      desktop: 'It runs inside the desktop app and updates with it.',
      docker: 'Update it where it runs: pull lull\'s newest image and start it again (docker compose pull, then docker compose up -d).',
      server: 'Update it the way it was installed.'
    }[b.install] || '';
    var state = {
      current: function () { return statusLine('ok', 'Up to date'); },
      ahead: function () { return html`${statusLine('ok', 'Follows the latest changes')}<span class="sys-row__help">It has everything in ${App.product} ${latest} and what landed since.</span>`; },
      available: function () {
        return html`${statusLine('warn', App.product + ' ' + latest + ' is out')}${how ? html`<span class="sys-row__help">${how}</span>` : ''}<a class="btn btn--sm" href="${r.latest.url}" target="_blank" rel="noopener">${icon('external', null, 'ic-16')}What's new</a>`;
      },
      unknown: function () { return statusLine('down', 'Couldn\'t check for updates. ' + (r.error || '')); }
    }[r.state];
    return html`<span class="px sys-version" id="sys-version">${buildName(b)}</span>${state ? state() : ''}${checkButton(r.state === 'unknown' ? 'Try again' : 'Check for updates')}`;
  }
  function checkButton(label) { return html`<button class="btn btn--sm" data-act="sys-update-check">${icon('refresh')}${label}</button>`; }
  function serverVersionRow(app) {
    if (!srvUpd) loadServerUpdate(false);
    return row(app ? 'Your server' : 'Version', app ? 'The hub this app plays from.' : 'This page comes from your server, so this is its version too.',
      html`<div class="sys-inline sys-inline--col" id="sys-server-update" aria-live="polite">${serverUpdateBody()}</div>`);
  }
  var NAMES = { soulseek: 'Soulseek', library: 'Library', lastfm: 'Last.fm', cache: 'Cache' };
  function healthRows(checked) {
    var s = App.store.get('services'), srv = App.server;
    var rows = [[srv.reachable ? 'ok' : 'down', 'Server', srv.reachable ? 'Answering' + (srv.version ? ', version ' + srv.version : '') : 'Not answering']]
      .concat(Object.keys(NAMES).map(function (k) { return [s[k].state, NAMES[k], s[k].detail]; }));
    return html`<div class="status-list">${rows.map(function (r) {
      return html`<div class="status-row">${ui.statusGlyph(r[0], STATE_WORD[r[0]])}<div><span class="status-row__name">${r[1]}</span><span class="status-row__detail">${r[2]}</span></div></div>`;
    })}</div><p class="sys-card__lede sys-health__when">${checked ? 'Checked just now from ' : 'Kept current by the server, from '}<span class="px">/health</span></p>`;
  }
  /* Settings > App's "Check now", and the status popover's "Check again" (catalog): the health rows show only on the page */
  App.action('sys-health', function (el) {
    var out = $('#sys-health'); el.disabled = true; if (out) out.setAttribute('aria-busy', 'true');
    checkHealth().then(function () {
      el.disabled = false;
      if (out) { out.removeAttribute('aria-busy'); repaint('#sys-health', healthRows(true)); }
      App.announce('Health checked');
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
  ['closeToTray', 'launchAtLogin', 'startInTray', 'miniStyle', 'miniOnTop', 'ownTitlebar', 'graphics', 'discordPresence', 'discordStatus',
    'discordCover', 'discordButtons', 'discordPaused'].forEach(function (key) {
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
      ${d.ownTitlebar == null ? '' : switchRow('desktop.ownTitlebar', 'Use lull title bar', 'Replaces the system title bar; takes effect after a restart.')}
      ${d.graphics == null ? '' : graphicsRow(window.LullDesktop.graphics)}
      ${d.discordPresence == null ? '' : discordRows(d)}`;
  }
  /* Graphics (app/src/desktop/graphics.rs): its words, the toast on a change and the line under it are desktop.js's, which
     only the app loads (LullDesktop.graphics) */
  function graphicsRow(g) {
    return html`${row('Graphics', g.help, choice('desktop.graphics', 'Graphics', g.modes, 'sys-seg--wrap'), { cls: 'sys-row--stack' })}<div id="sys-graphics">${raw(g.line())}</div>`;
  }
  /* Discord Rich Presence (app/src/desktop/discord.rs): the switch, then its look while it is on */
  function discordRows(d) {
    return html`${switchRow('desktop.discordPresence', 'Show what I\'m playing on Discord', 'While Discord runs on this computer, your status shows the song, its cover and its progress.')}
      <div data-when="discord" ${d.discordPresence ? '' : raw('hidden')}>
        ${row('Next to your name', 'What follows "Listening to" in the member list.', choice('desktop.discordStatus', 'Next to your name', [['song', 'Song'], ['artist', 'Artist'], ['lull', 'lull']]))}
        ${switchRow('desktop.discordCover', 'Show the album cover', 'Off shows lull\'s own picture instead.')}
        ${switchRow('desktop.discordButtons', 'Show buttons', '"Open on song.link" and "Get lull". Others see them; Discord doesn\'t show them to you.')}
        ${row('When paused', '', choice('desktop.discordPaused', 'When paused', [['clear', 'Hide it'], ['show', 'Show it as paused']]))}
      </div>`;
  }
  /* ---------- this app's own version and updates: LullDesktop.updates (desktop) or LullNative.updates (Android) ---------- */
  function appUpdates() {
    return (inWrapper() && window.LullDesktop.updates) || (window.LullNative && window.LullNative.updates) || null;
  }
  var upd = null, updHeard = false;                // the app's report { current, state, version, received, total, error }
  function showUpdate(r) { upd = r; repaint('#sys-app-update', updateBody()); }
  function loadUpdate() {
    var u = appUpdates();
    upd = { loading: true };
    u.status().then(showUpdate, function (err) { showUpdate({ askError: errText(err) }); });
    if (u.on && !updHeard) { updHeard = true; u.on(function (r) { if (upd) showUpdate(r); }); }
  }
  App.action('sys-app-update-check', function (el) {
    el.disabled = true;
    appUpdates().check().then(function (r) {
      showUpdate(r);
      if (r.state === 'current') App.announce(App.product + ' ' + r.current + ' is the newest version');
    }, function (err) { el.disabled = false; App.toast('Couldn\'t check for updates: ' + errText(err), { kind: 'error' }); });
  });
  App.action('sys-app-update-install', function (el) {
    el.disabled = true;
    appUpdates().install().then(function () {
      el.disabled = false;
      if (window.LullNative) App.toast('Your browser is downloading ' + App.product + ' ' + upd.version + '. Open the file to install it.', { icon: 'check' });
    }, function (err) { el.disabled = false; App.toast('Couldn\'t update: ' + errText(err), { kind: 'error' }); });
  });
  function updateBody() {
    var u = upd;
    if (!u || u.loading) return spinnerLine('Asking the app');
    if (u.askError) return statusLine('down', u.askError);
    var name = function (v) { return App.product + ' ' + v; };
    var button = function (act, label, primary) {
      return html`<button class="btn btn--sm ${primary ? 'btn--primary' : ''}" data-act="${act}">${primary ? '' : icon('refresh')}${label}</button>`;
    };
    var check = button('sys-app-update-check', u.state === 'failed' ? 'Try again' : 'Check for updates');
    var state = {
      idle: function () { return html`<span class="sys-row__help">Checks GitHub for a newer release.</span>${check}`; },
      checking: function () { return spinnerLine('Checking for updates'); },
      current: function () { return html`${statusLine('ok', 'Up to date')}${check}`; },
      downloading: function () {
        var pct = u.total ? Math.floor(u.received * 100 / u.total) : null;
        return html`${spinnerLine('Downloading ' + name(u.version) + (pct == null ? '' : ': ' + pct + '%'))}${pct == null ? '' : html`<progress max="100" value="${pct}" aria-label="Download progress"></progress>`}`;
      },
      ready: function () { return html`${statusLine('warn', name(u.version) + ' is out')}<span class="sys-row__help">It's downloaded.</span>${button('sys-app-update-install', 'Restart to update', true)}`; },
      available: function () {
        return html`${statusLine('warn', name(u.version) + ' is out')}<span class="sys-row__help">Your browser downloads it; open the file to install it over this version.</span>${button('sys-app-update-install', 'Download', true)}`;
      },
      failed: function () { return html`${statusLine('down', 'Couldn\'t check for updates. ' + u.error)}${check}`; },
      packaged: function () { return statusLine('ok', 'Updates come with ' + u.by); }
    }[u.state];
    if (!state) return statusLine('down', 'Unknown update state ' + u.state);
    return html`<span class="px sys-version">${name(u.current)}</span>${state()}`;
  }
  function appVersionRow() {
    if (!upd) loadUpdate();
    return row('This app', 'It updates from lull\'s releases on GitHub.', html`<div class="sys-inline sys-inline--col" id="sys-app-update" aria-live="polite">${updateBody()}</div>`);
  }
  function secApp() {
    if (inWrapper() && !desk) loadDesktop();
    var app = appUpdates();
    return html`${app ? appVersionRow() : ''}${serverVersionRow(!!app)}
      ${row('Install app', 'Adds lull to your home screen or dock with lock-screen controls and its own window.', html`<div id="sys-install">${installBody()}</div>`)}
      ${row('First-run setup', 'Your library, your Soulseek account and Last.fm, step by step.', html`<a class="btn" href="#/setup">Open setup</a>`)}
      ${row('Licence', 'lull is free software under the GNU Affero General Public License 3.0: you may use, change and share it, and its source is open to everyone who uses it.', html`<a class="btn" href="https://github.com/TannhauserCommons/lull" target="_blank" rel="noopener">${icon('external', null, 'ic-16')}Source code</a>`)}
      ${group('server')}
      ${inWrapper() ? '' : configRow('Server address', serverAddress(), 'The address this page was loaded from.')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label">Server health</span><span class="sys-row__help">What the server reports about itself and its services.</span></div>
        <div class="sys-row__ctl"><div id="sys-health" aria-live="polite">${healthRows(false)}</div><button class="btn btn--sm" data-act="sys-health">${icon('refresh')}Check now</button></div></div>
      ${inWrapper() ? html`${group('desktop app')}<div id="sys-desktop">${desktopRows()}</div>` : ''}`;
  }

  var BODY = {
    playback: function () { return withSettings(secPlayback, true); }, sources: function () { return withSettings(secSources); },
    search: function () { return withSettings(secSearch); },
    cache: function () { return withSettings(secCache); }, library: function () { return withSettings(secLibrary, true); },
    sharing: function () { return withSettings(secSharing); }, lastfm: secLastfm,
    discovery: function () { return withSettings(secDiscovery); }, privacy: secPrivacy,
    shortcuts: secShortcuts, app: secApp
  };
  function sectionBody(id) { return BODY[id] ? BODY[id]() : App.settingsSections[id](); }

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
          <div class="card sys-card sys-section" id="sys-section">${ui.labelRule(s[1].toLowerCase(), { id: 'sys-sec-title' })}${sectionBody(cur)}</div>
        </section>
      </div></div>`;
  }
  /* live parts repaint in place: services (Jellyfin, health), jobs (rescan), the server, install state */
  function mountSettings(ctx) {
    var offs = [
      App.on('prefs', function () {
        var p = App.prefs(); showChoice('device.theme', p.theme); showChoice('device.motion', p.motion); showChoice('device.design', p.design);
      }),
      App.store.subscribe(function (keys) {
        if (keys.indexOf('services') >= 0) repaint('#sys-health', healthRows(false));
        if (keys.indexOf('activity') >= 0) repaint('#sys-rescan', rescanStatus());
      }),
      App.on('server', function () {
        repaint('#sys-health', healthRows(false));
        if (srvUpd && srvUpd.data && srvUpd.data.version !== App.server.version) loadServerUpdate(false);
      }),
      App.on('pwa', function () { repaint('#sys-install', installBody()); }),
      App.on('config', function (key) { if (key === 'LULL_NAMING' || key === 'LULL_MUSIC_DIR') repaint('#sys-naming', namingBox()); })
    ];
    ctx.onCleanup(function () {
      offs.forEach(function (off) { off(); });
      if (!/^#\/settings\/app\b/.test(location.hash)) { desk = null; srvUpd = null; }   // leaving Settings > App: ask again next visit
    });
  }
  /* Settings' own fits (screens/settings-fit.js) load on entry, and the sections built from screens/hub.js
     (App.hubKit) load it too; once both are here a section renders at once */
  var HUB_SECTIONS = { sources: true, library: true, devices: true };
  App.needSettingsFit = function () { return App.needScript('screens/settings-fit.js', 'Settings'); };
  /* Settings > Sounds comes with its own file too (screens/sounds-settings.js), as it opens */
  function settingsNeeds(params) {
    var load = [
      App.settingsFit ? null : App.needSettingsFit(),
      HUB_SECTIONS[params.section] && !App.hubKit ? App.needHub() : null,
      params.section === 'sounds' && !App.settingsSections.sounds ? App.needSoundSettings() : null
    ].filter(Boolean);
    return load.length ? Promise.all(load) : null;
  }
  App.route('/settings/:section?', { need: settingsNeeds, render: renderSettings, mount: mountSettings, nav: 'settings', title: function (ctx) { var s = sectionOf(ctx.params.section); return s ? s[1] + ' settings' : 'Settings'; } });

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
      { label: reduced ? 'Turn full motion on' : 'Reduce motion', icon: 'equaliser', run: function () { App.setPref('motion', reduced ? 'full' : 'reduce'); App.toast(reduced ? 'Full motion on' : 'Reduced motion on', { confirms: true, group: 'motion', icon: 'equaliser' }); } },
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
      return { label: t.title, sub: t.artist, cover: t, run: function () { P.playTrack(t.id); App.toast('Playing ' + t.title, { confirms: true, group: 'play', icon: 'play' }); } };
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
    var sk = s.soulseek, jf = s.library, lf = s.lastfm, ca = s.cache;
    /* the hub's services are the paired listener's business: behind the pairing gate only reachability shows */
    var paired = App.server.authorized !== false;
    if (paired && (sk.state === 'down' || sk.state === 'warn')) {
      out.push({ id: 'sys-soulseek', sig: sk.state, kind: sk.state, icon: 'soulseek', text: 'Soulseek: ' + clause(sk.detail) + '. Saved songs still play.', action: { label: 'Check again', run: checkHealth } });
    }
    if (paired && jf.state === 'down') {
      out.push({ id: 'sys-jellyfin', sig: jf.state, kind: 'warn', icon: 'warning', text: 'Your library isn\'t responding. It\'s read-only, and saving waits until it\'s back.', action: { label: 'Retry', run: checkHealth } });
    }
    var cat = s.catalog;
    if (paired && cat && cat.state === 'down') {
      out.push({ id: 'sys-catalog', sig: cat.state, kind: 'warn', icon: 'search', text: 'Search can\'t reach the music catalogs: ' + clause(cat.detail) + '. Your library and Soulseek still answer.', action: { label: 'Check again', run: checkHealth } });
    }
    if (paired && lf.state === 'down') {
      out.push({ id: 'sys-lastfm', sig: lf.state, kind: 'warn', icon: 'warning', text: 'Last.fm is down.' + (lf.queued ? ' Scrobbles are queued (' + lf.queued + ') and will retry.' : ''), action: { label: 'Retry now', run: retryScrobbles } });
    }
    if (paired && (ca.state === 'warn' || ca.state === 'down')) {
      out.push({ id: 'sys-cache', sig: ca.state, kind: 'warn', icon: 'warning', text: ca.state === 'warn' ? 'Cache full and every item is in use, so playback may pause.' : 'Cache: ' + clause(ca.detail) + '.', action: { label: 'Cache settings', run: go('/settings/cache') } });
    }
    if (App.server.reachable === false) {
      out.push({ id: 'sys-unreachable', sig: 'down', kind: 'down', icon: 'offline', dismissible: false, text: 'Can\'t reach your hub. Downloaded songs still play, and changes wait on this device.', action: { label: 'Retry now', run: checkHealth } });
    }
    if (navigator.onLine === false) {
      out.push({ id: 'sys-offline', sig: 'offline', kind: 'warn', icon: 'offline', text: 'You\'re offline. Downloaded songs still play; search and streaming wait for the connection.', action: { label: 'Downloads', run: go('/downloads') } });
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
    syncBanners();
  });
  App.on('ready', syncBanners);
  App.on('design', function () { setSearchPaused(App.server.reachable === false); });   // the header was re-rendered
  window.addEventListener('offline', syncBanners);
  window.addEventListener('online', syncBanners);
})();
