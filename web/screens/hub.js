/* screens/hub.js.
   The hub's own settings (ADR-0002, ADR-0003), built with the settings kit (system.js):
   - the library source: a music folder or Jellyfin (found on the network, connected by Quick Connect or a password);
   - the Soulseek account slskd signs in with: hub config when the hub manages its own slskd (the desktop app,
     GET /api/setup `soulseekManaged`), otherwise only slskd's state (the login is set where slskd runs);
   - Settings > Devices: paired devices (online, rename, unpair), adding one by QR code, Subsonic app passwords.
   First-run setup (setup.js) is built from the same parts: App.hubKit.
   Reads: GET /api/library/source, GET /api/jellyfin/discover, GET /api/devices, `library.source` and `device.*` events. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var K = App.settingsKit;
  var SOURCE = '/api/library/source', DISCOVER = '/api/jellyfin/discover', DEVICES = '/api/devices', CONFIG = '/api/config';
  var C = App.connect;
  /* the hub config keys holding the Soulseek account, used only when the hub runs its own slskd */
  var SLSK_USER = 'LULL_SOULSEEK_USERNAME', SLSK_PASS = 'LULL_SOULSEEK_PASSWORD', SETUP = '/api/setup';
  function soulseekManaged() { var s = M.peek(SETUP); return !!(s && s.soulseekManaged); }
  var ELSEWHERE = 'Your Soulseek login is set where slskd runs (its own settings, or the deploy kit\'s .env).';

  function errText(err) { return err && err.message || String(err); }
  function fieldError(msg) { return msg ? html`<p class="field__error" role="alert">${icon('warning', null, 'ic-16')}<span>${msg}</span></p>` : ''; }
  /* "just now", "5 min ago", "3 h ago", "Oct 29" */
  function ago(at) {
    var s = Date.now() / 1000 - (at || 0);
    if (!at) return 'never';
    if (s < 90) return 'just now';
    if (s < 3600) return Math.round(s / 60) + ' min ago';
    if (s < 86400) return Math.round(s / 3600) + ' h ago';
    return App.fmt.date(at, true);
  }
  /* the browser and system this page runs in, as a default device name ("Firefox on Windows") */
  function deviceName() {
    var u = navigator.userAgent;
    var b = /Edg\//.test(u) ? 'Edge' : /Firefox\//.test(u) ? 'Firefox' : /Chrome\//.test(u) ? 'Chrome' : /Safari\//.test(u) ? 'Safari' : 'Browser';
    var os = /Android/.test(u) ? 'Android' : /iPhone/.test(u) ? 'iPhone' : /iPad/.test(u) ? 'iPad' : /Windows/.test(u) ? 'Windows'
      : /Mac OS/.test(u) ? 'Mac' : /Linux/.test(u) ? 'Linux' : '';
    return os ? b + ' on ' + os : b;
  }
  /* browsers install the app and keep downloads only on HTTPS (or localhost) */
  function httpsNote() {
    if (window.isSecureContext) return '';
    return html`<p class="sys-note sys-note--warn">${icon('lock', null, 'ic-16')}<span>This page isn't on HTTPS, so this browser can't install lull as an app or keep songs for offline. With Tailscale, run <span class="px">tailscale serve</span> on the hub and open its https address instead.</span></p>`;
  }

  /* =====================================================================
     1. Library source: GET /api/library/source { mode: folder|jellyfin, musicDir, jellyfin: { url, user, connected }|null,
        files?, lastScan? }. Switching is PUT /api/library/source, progress as `library.source` events.
     ===================================================================== */
  var lib = {
    pick: null,          // the mode the listener is looking at (null: the current one)
    url: '',             // the Jellyfin address being connected
    stage: 'choose',     // jellyfin: choose | quick (code shown) | signin (password form)
    another: false,      // connected, but connecting another server
    qc: null,            // Quick Connect { code, secret, state: waiting|approved|expired }
    busy: false, error: null,
    progress: null       // the latest `library.source` event
  };
  K.custom['hub.library'] = { get: function () { var s = M.peek(SOURCE); return lib.pick || (s && s.mode) || 'folder'; },
    set: function (v) { lib.pick = v; lib.stage = 'choose'; lib.error = null; repaintLib(); } };
  function repaintLib() { K.repaint('#hub-lib', libBody(), '[data-key="hub.library"] [aria-checked="true"]'); }

  function librarySource(o) {
    o = o || {};
    lib.withFolder = !!o.withFolder;
    return html`<div class="hub-lib" id="hub-lib" aria-live="polite">${libBody()}</div>`;
  }
  function libBody() {
    var r = M.get(SOURCE);
    if (r.loading) return ui.skeleton('text', 3);
    if (r.error) return K.readError(r.error, SOURCE, 'Couldn\'t read where your library lives');
    var src = r.data, pick = lib.pick || src.mode, env = K.cfgItem('LULL_LIBRARY');
    if (!lib.progress && src.switching) lib.progress = { mode: src.mode, state: 'switching', detail: null };
    if (env && env.source === 'env') {
      return html`${K.row('Library', 'Set in the hub\'s environment (LULL_LIBRARY), so it can\'t switch here.', K.lockedValue('Library', src.mode === 'jellyfin' ? 'Jellyfin' : 'Music folder'), { cls: 'sys-row--locked' })}
        <div class="hub-lib__panel">${src.mode === 'folder' ? folderPanel(src) : jellyfinPanel(src)}</div>`;
    }
    return html`${K.row('Library', 'Where your saved music lives and plays from. Switching keeps your likes, playlists and history.',
        K.choice('hub.library', 'Library', [['folder', 'Music folder'], ['jellyfin', 'Jellyfin']]))}
      ${progressLine()}
      <div class="hub-lib__panel">${pick === 'folder' ? folderPanel(src) : jellyfinPanel(src)}</div>`;
  }
  function progressLine() {
    var p = lib.progress; if (!p) return '';
    if (p.state === 'switching') return html`<p class="hub-lib__progress">${K.spinnerLine(p.detail || 'Switching your library')}</p>`;
    if (p.state === 'failed') return html`<p class="hub-lib__progress">${K.statusLine('down', p.detail || 'The switch failed')}</p>`;
    return html`<p class="hub-lib__progress">${K.statusLine('ok', p.detail || 'Library switched')}</p>`;
  }
  function folderPanel(src) {
    var facts = [src.files != null ? Number(src.files).toLocaleString('en') + ' songs' : '', src.lastScan ? 'scanned ' + ago(src.lastScan) : ''].filter(Boolean).join(', ');
    var using = src.mode === 'folder';
    return html`${lib.withFolder ? K.cfgField('LULL_MUSIC_DIR', 'Music folder', 'The folder on the hub that holds your music. Saved songs go here too.') : ''}
      <div class="inset sys-connect ${using ? 'sys-connect--ok' : ''}">${using ? ui.statusGlyph('ok', 'In use') : icon('library', null, 'ic-24')}
        <div><p class="sys-connect__title">${using ? 'Using the music folder' : 'Play from the music folder'}</p>
          <p class="sys-connect__text">${using ? (facts ? facts + '. ' : '') + 'New files are picked up as they appear.' : 'lull reads the folder itself: tags, covers, and new files as they appear. No other server needed.'}</p></div>
        ${using ? '' : html`<div class="sys-connect__actions"><button class="btn btn--primary" data-act="hub-use-folder" ${lib.busy ? raw('disabled') : ''}>Use the music folder</button></div>`}</div>
      ${fieldError(lib.error)}`;
  }
  function jellyfinPanel(src) {
    var jf = src.jellyfin, connected = !!(jf && jf.connected);
    if (connected && lib.qc && lib.qc.state === 'approved') { lib.stage = 'choose'; lib.qc = null; }
    if (connected && !lib.another) {
      var using = src.mode === 'jellyfin';
      return html`<div class="inset sys-connect sys-connect--ok">${ui.statusGlyph('ok', 'Connected')}
        <div><p class="sys-connect__title">Connected to <span class="px">${jf.url}</span>${jf.user ? html` as <b>${jf.user}</b>` : ''}</p>
          <p class="sys-connect__text">${using ? 'Saved songs play from Jellyfin, and your likes and playlists show up in its apps.' : 'Jellyfin is connected but not in use.'}</p></div>
        <div class="sys-connect__actions">${using ? '' : html`<button class="btn btn--primary" data-act="hub-use-jellyfin" ${lib.busy ? raw('disabled') : ''}>Use Jellyfin</button>`}
          <button class="btn" data-act="hub-jf-another">Connect another server</button></div></div>${fieldError(lib.error)}`;
    }
    if (!lib.url && jf && jf.url) lib.url = jf.url;
    if (lib.stage === 'quick') return quickBox();
    return html`<div class="hub-jf">
      <p class="hub-jf__label" id="hub-jf-found">Jellyfin servers on your network</p>
      <div class="hub-jf__found" role="group" aria-labelledby="hub-jf-found">${foundServers()}</div>
      <div class="field"><label class="field__label" for="hub-jf-url">Server address</label>
        <input class="input" id="hub-jf-url" type="url" inputmode="url" placeholder="http://192.168.1.20:8096" value="${lib.url}" autocomplete="off" spellcheck="false"></div>
      ${lib.stage === 'signin' ? signinForm() : html`<div class="hub-jf__actions">
        <button class="btn btn--primary" data-act="hub-jf-quick" ${lib.busy ? raw('disabled') : ''}>${lib.busy ? html`<span class="spin-ring" aria-hidden="true"></span>` : ''}Connect with Quick Connect</button>
        <button class="btn btn--quiet" data-act="hub-jf-signin-open">Sign in with a password</button>
        ${connected ? html`<button class="btn btn--quiet" data-act="hub-jf-cancel">Cancel</button>` : ''}</div>`}
      ${fieldError(lib.error)}</div>`;
  }
  function foundServers() {
    var r = M.get(DISCOVER);
    if (r.loading) return K.spinnerLine('Looking for Jellyfin');
    if (r.error) return html`<span class="sys-row__help">${K.clause(r.error.message)}. Type its address instead.</span>`;
    if (!r.data.items.length) return html`<span class="sys-row__help">None answered. Type its address below. <button class="btn btn--quiet btn--sm" data-act="reload-read" data-id="${DISCOVER}">Look again</button></span>`;
    return html`<div class="cluster">${r.data.items.map(function (s) {
      return html`<button type="button" class="chip hub-jf__server" data-act="hub-jf-server" data-url="${s.url}" aria-pressed="${s.url === lib.url ? 'true' : 'false'}">${icon('check', null, 'chip__check')}${s.name}<span class="px">${s.url.replace(/^https?:\/\//, '')}</span></button>`;
    })}</div>`;
  }
  function signinForm() {
    return html`<form class="hub-jf__signin" data-form="hub-jf-signin">
      <div class="field"><label class="field__label" for="hub-jf-user">Jellyfin user</label><input class="input" id="hub-jf-user" autocomplete="username" spellcheck="false" required></div>
      <div class="field"><label class="field__label" for="hub-jf-pass">Password</label><input class="input" id="hub-jf-pass" type="password" autocomplete="current-password"><span class="field__help">Used once to get a key; lull never stores it.</span></div>
      <div class="hub-jf__actions"><button class="btn btn--primary" type="submit" ${lib.busy ? raw('disabled') : ''}>Sign in</button><button class="btn btn--quiet" type="button" data-act="hub-jf-back">Use Quick Connect instead</button></div></form>`;
  }
  function quickBox() {
    var q = lib.qc;
    if (q.state === 'expired') {
      return html`<div class="inset sys-connect sys-connect--error">${ui.statusGlyph('down', 'Expired')}<div><p class="sys-connect__title">That code expired</p>
        <p class="sys-connect__text">Quick Connect codes last a few minutes.</p></div>
        <div class="sys-connect__actions"><button class="btn btn--primary" data-act="hub-jf-quick">Get a new code</button><button class="btn btn--quiet" data-act="hub-jf-back">Back</button></div></div>`;
    }
    if (q.state === 'approved') return html`<p>${K.spinnerLine('Approved. Connecting your Jellyfin library')}</p>`;
    return html`<div class="hub-qc">
      <p class="hub-qc__code px" aria-label="Quick Connect code ${q.code.split('').join(' ')}">${q.code}</p>
      <ol class="hub-qc__steps"><li>Open Jellyfin and sign in.</li><li>Go to your profile, then <b>Quick Connect</b>.</li><li>Enter this code and approve.</li></ol>
      <p>${K.spinnerLine('Waiting for approval')}</p>
      <div class="hub-jf__actions"><button class="btn btn--quiet" data-act="hub-jf-back">Cancel</button><button class="btn btn--quiet" data-act="hub-jf-signin-open">Sign in with a password instead</button></div></div>`;
  }

  function jfUrl() {
    var i = $('#hub-jf-url'); if (i) lib.url = i.value.trim();
    if (!lib.url) { lib.error = 'Pick a server above or type its address.'; repaintLib(); if ($('#hub-jf-url')) $('#hub-jf-url').focus(); return null; }
    if (!/^https?:\/\//.test(lib.url)) lib.url = 'http://' + lib.url;
    return lib.url;
  }
  function busy(on, err) { lib.busy = on; lib.error = err ? errText(err) : null; repaintLib(); }
  function switchTo(body) {
    busy(true);
    M.api(SOURCE, { method: 'PUT', body: body }).then(function () {
      lib.busy = false; lib.pick = null; lib.another = false; lib.stage = 'choose';
      lib.progress = { mode: body.mode, state: 'switching', detail: null };
      repaintLib();
    }, function (err) { busy(false, err); });
  }
  App.action('hub-use-folder', function () { switchTo({ mode: 'folder' }); });
  App.action('hub-use-jellyfin', function () { switchTo({ mode: 'jellyfin' }); });
  App.action('hub-jf-another', function () { lib.another = true; lib.stage = 'choose'; lib.error = null; repaintLib(); });
  App.action('hub-jf-cancel', function () { lib.another = false; lib.error = null; repaintLib(); });
  App.action('hub-jf-server', function (el) { lib.url = el.getAttribute('data-url'); lib.error = null; repaintLib(); });
  App.action('hub-jf-signin-open', function () { jfUrl(); lib.stage = 'signin'; lib.qc = null; lib.error = null; repaintLib(); if ($('#hub-jf-user')) $('#hub-jf-user').focus(); });
  App.action('hub-jf-back', function () { lib.stage = 'choose'; lib.qc = null; lib.error = null; repaintLib(); });
  App.action('hub-jf-quick', function () {
    var url = lib.stage === 'quick' ? lib.url : jfUrl(); if (!url) return;
    busy(true);
    M.api('/api/jellyfin/quickconnect', { method: 'POST', body: { url: url } }).then(function (res) {
      lib.busy = false; lib.stage = 'quick'; lib.qc = { code: res.code, secret: res.secret, state: 'waiting' };
      repaintLib(); pollQuick(lib.qc);
    }, function (err) { lib.stage = err.status === 409 ? 'signin' : 'choose'; busy(false, err); });   // 409: Quick Connect is off there
  });
  /* ask every two seconds while this code is the one showing */
  function pollQuick(q) {
    setTimeout(function () {
      if (lib.qc !== q || q.state !== 'waiting' || !$('#hub-lib')) return;
      M.api('/api/jellyfin/quickconnect/' + encodeURIComponent(q.secret)).then(function (res) {
        if (lib.qc !== q) return;
        q.state = res.state;
        if (res.state === 'waiting') { pollQuick(q); return; }
        if (res.state === 'approved') { lib.another = false; lib.progress = { mode: 'jellyfin', state: 'switching', detail: 'Connected. Switching your library to Jellyfin' }; }
        repaintLib();
      }, function (err) {
        if (lib.qc !== q) return;
        if (err.status === 404) { q.state = 'expired'; repaintLib(); return; }   // the hub forgot the code: start again
        lib.stage = 'choose'; lib.qc = null; busy(false, err);
      });
    }, 2000);
  }
  document.addEventListener('submit', function (e) {
    var f = e.target.closest('[data-form="hub-jf-signin"]'); if (!f) return;
    e.preventDefault();
    var url = jfUrl(); if (!url) return;
    var body = { url: url, username: $('#hub-jf-user', f).value.trim(), password: $('#hub-jf-pass', f).value };
    if (!body.username) { busy(false, { message: 'Enter your Jellyfin user name.' }); return; }
    busy(true);
    M.api('/api/jellyfin/signin', { method: 'POST', body: body }).then(function () {
      lib.busy = false; lib.stage = 'choose'; lib.another = false;
      lib.progress = { mode: 'jellyfin', state: 'switching', detail: 'Signed in. Switching your library to Jellyfin' };
      M.reload(SOURCE); repaintLib();
    }, function (err) { busy(false, err); });
  });
  document.addEventListener('input', function (e) { if (e.target.id === 'hub-jf-url') lib.url = e.target.value.trim(); });
  App.on('server:library.source', function (e) {
    lib.progress = e;
    if (e.state !== 'switching') { M.reload(SOURCE); M.reload('/api/setup'); }
    if ($('#hub-lib')) repaintLib();
  });

  /* =====================================================================
     2. Soulseek account: two hub config keys; slskd signs in again when they change (the hub restarts it)
     ===================================================================== */
  function soulseekStatus() {
    var st = App.store.get('services').soulseek;
    return K.statusLine(st.state, st.detail);
  }
  /* Settings > Sources: the two keys as config fields, with slskd's own status */
  function soulseekRows() {
    var r = M.get(SETUP), managed = soulseekManaged();
    return html`${K.row('Soulseek', managed ? 'The account slskd signs in with. Soulseek allows one sign-in per account at a time.' : ELSEWHERE, html`<span id="hub-slsk-status">${soulseekStatus()}</span>`)}
      ${!r.loading && managed ? html`${K.cfgField(SLSK_USER, 'Soulseek user name')}${K.cfgField(SLSK_PASS, 'Soulseek password')}` : ''}`;
  }
  /* first-run setup: both keys in one form, one restart */
  var slsk = { busy: false, error: null, sent: false };
  function soulseekForm() {
    var st = App.store.get('services').soulseek;
    if (!soulseekManaged()) return html`<p class="sys-row__help">${ELSEWHERE}</p>${soulseekStatus()}`;
    var r = M.get(CONFIG), user = K.cfgItem(SLSK_USER), pass = K.cfgItem(SLSK_PASS);
    if (r.loading) return ui.skeleton('text', 2);
    if (r.error) return html`${fieldError(r.error.message)}<button class="btn btn--sm" data-act="reload-read" data-id="${CONFIG}">Try again</button>`;
    if (!user || !pass || user.source === 'env') {
      return html`<p class="sys-row__help">${user && user.value ? html`Signed in as <b>${user.value}</b>. ` : ''}The hub's environment sets this account.</p>${soulseekStatus()}`;
    }
    if (user.value && pass.set && !slsk.editing) {
      return html`<p class="sys-step__done">${st.state === 'ok' ? icon('check', null, 'ic-16') : ''}${st.state === 'ok' ? html`Signed in as <b>${user.value}</b>` : html`<span>${user.value}</span>`}</p>
        ${st.state === 'ok' ? '' : soulseekStatus()}<button class="btn btn--sm btn--quiet" data-act="hub-slsk-edit">Use another account</button>`;
    }
    return html`<form class="hub-slsk" data-form="hub-slsk">
      <div class="field"><label class="field__label" for="hub-slsk-user">User name</label><input class="input" id="hub-slsk-user" value="${user.value || ''}" autocomplete="username" spellcheck="false"></div>
      <div class="field"><label class="field__label" for="hub-slsk-pass">Password</label><input class="input" id="hub-slsk-pass" type="password" autocomplete="current-password"><span class="field__help">No account yet? slskd creates one with any new name and password.</span></div>
      ${fieldError(slsk.error)}
      <button class="btn btn--primary" type="submit" ${slsk.busy ? raw('disabled') : ''}>${slsk.busy ? html`<span class="spin-ring" aria-hidden="true"></span>` : ''}Sign in</button></form>`;
  }
  function repaintSlsk() { K.repaint('#hub-slsk', soulseekForm()); }
  App.action('hub-slsk-edit', function () { slsk.editing = true; repaintSlsk(); if ($('#hub-slsk-user')) $('#hub-slsk-user').focus(); });
  document.addEventListener('submit', function (e) {
    var f = e.target.closest('[data-form="hub-slsk"]'); if (!f) return;
    e.preventDefault();
    var body = {}; body[SLSK_USER] = $('#hub-slsk-user', f).value.trim(); body[SLSK_PASS] = $('#hub-slsk-pass', f).value;
    if (!body[SLSK_USER] || !body[SLSK_PASS]) { slsk.error = 'Enter a user name and a password.'; repaintSlsk(); return; }
    slsk.busy = true; slsk.error = null; repaintSlsk();
    M.api(CONFIG, { method: 'PATCH', body: body }).then(function (res) {
      var c = M.peek(CONFIG); if (c && res && res.items) c.items = res.items;
      slsk.busy = false; slsk.editing = false; repaintSlsk();
      App.toast('Signing in to Soulseek', { icon: 'soulseek' });
    }, function (err) { slsk.busy = false; slsk.error = err.message; repaintSlsk(); });
  });
  App.store.subscribe(function (keys) {
    if (keys.indexOf('services') < 0) return;
    K.repaint('#hub-slsk-status', soulseekStatus());
    if ($('#hub-slsk') && !$('form', $('#hub-slsk'))) repaintSlsk();
  });

  /* =====================================================================
     3. Settings > Devices: the store's `devices` (connect.js keeps it: GET /api/devices and device events)
     ===================================================================== */
  var KIND_ICON = { browser: 'globe', app: 'device', subsonic: 'link' };
  function devices() { return App.store.get('devices').items; }
  function secDevices() {
    return html`${httpsNote()}
      <p class="sys-card__lede">Every device you pair shares this hub's library, likes, playlists and history. Unpairing one stops it at once.</p>
      <div id="hub-devices">${devicesBody()}</div>
      <p><button class="btn btn--primary" data-act="hub-pair">${icon('plus')}Add a device</button></p>
      ${K.group('subsonic apps')}
      <p class="sys-card__lede">Apps that speak Subsonic (Symfonium, Feishin, Substreamer and others) sign in to this hub with user name <span class="px">lull</span> and an app password made here, one per app.</p>
      <div id="hub-subsonic">${subsonicBody()}</div>
      <p><button class="btn" data-act="hub-subsonic-new">${icon('plus')}Make an app password</button></p>`;
  }
  function devicesBody() {
    var s = App.store.get('devices');
    if (s.loading) return ui.skeleton('row', 2);
    if (s.error) return ui.emptyState({ art: 'star', title: 'Couldn\'t list your devices', text: s.error.message, action: { label: 'Try again', act: 'cx-reload' } });
    var list = s.items.filter(function (d) { return d.kind !== 'subsonic'; });
    return html`<ul class="hub-devs">${list.map(function (d) { return deviceRow(d, s.me); })}</ul>`;
  }
  function subsonicBody() {
    var s = App.store.get('devices');
    if (s.loading || s.error) return '';
    var list = s.items.filter(function (d) { return d.kind === 'subsonic'; });
    if (!list.length) return html`<p class="sys-row__help">No app passwords yet.</p>`;
    return html`<ul class="hub-devs">${list.map(function (d) { return deviceRow(d, s.me); })}</ul>`;
  }
  function deviceRow(d, me) {
    var t = d.state && d.state.trackId && M.track(d.state.trackId);
    var sub = d.kind === 'subsonic' ? 'Made ' + App.fmt.date(d.createdAt, true) + ', used ' + ago(d.lastSeen)
      : d.online ? 'Online' + (t ? (d.state.playing ? ', playing ' : ', paused on ') + t.title : '') : 'Last seen ' + ago(d.lastSeen);
    return html`<li class="hub-dev ${d.online ? 'is-online' : ''}" data-device="${d.id}">
      <span class="hub-dev__ic">${icon(KIND_ICON[d.kind] || 'device')}</span>
      <span class="hub-dev__text"><span class="hub-dev__name">${d.name}${d.id === me ? html` <span class="badge-count">this device</span>` : ''}</span>
        <span class="hub-dev__sub">${d.kind !== 'subsonic' ? ui.statusGlyph(d.online ? 'ok' : 'off', d.online ? 'Online' : 'Offline') : ''}<span>${sub}</span></span></span>
      <button class="btn-icon btn-icon--bare" data-act="hub-dev-more" data-id="${d.id}" aria-haspopup="menu" aria-label="More for ${d.name}" data-tip="More">${icon('more')}</button></li>`;
  }
  function repaintDevices() { K.repaint('#hub-devices', devicesBody()); K.repaint('#hub-subsonic', subsonicBody()); }
  function deviceById(id) { return C.byId(id); }

  App.action('hub-dev-more', function (el) {
    var d = deviceById(el.getAttribute('data-id')); if (!d) return;
    var me = App.store.get('devices').me === d.id;
    App.menu(el, [
      { label: 'Rename', icon: 'edit', run: function () { renameDevice(d); } },
      { label: me ? 'Unpair this device' : d.kind === 'subsonic' ? 'Revoke password' : 'Unpair', icon: 'trash', danger: true, run: function () { unpairDevice(d, me); } }
    ], { label: d.name });
  });
  function renameDevice(d) {
    var dlg = App.openDialog({ title: 'rename device', body: html`<form class="hub-form" id="hub-rename"><div class="field"><label class="field__label" for="hub-rename-name">Name</label>
        <input class="input" id="hub-rename-name" value="${d.name}" maxlength="60" required autofocus></div><span class="field__error" id="hub-rename-err" hidden></span></form>`,
      foot: html`<button class="btn" data-close>Cancel</button><button class="btn btn--primary" type="submit" form="hub-rename">Rename</button>` });
    $('#hub-rename', dlg.el).addEventListener('submit', function (e) {
      e.preventDefault();
      var name = $('#hub-rename-name', dlg.el).value.trim(), err = $('#hub-rename-err', dlg.el);
      if (!name) return;
      M.api(DEVICES + '/' + encodeURIComponent(d.id), { method: 'PATCH', body: { name: name } }).then(function () {
        dlg.close(); C.reload(); App.announce('Renamed to ' + name);
      }, function (e2) { err.textContent = e2.message; err.hidden = false; });
    });
  }
  function unpairDevice(d, me) {
    App.confirm({ title: me ? 'unpair this device?' : 'unpair ' + d.name + '?',
      text: me ? 'This browser stops reaching your hub at once. Pair it again from another device to come back.'
        : d.kind === 'subsonic' ? 'The app using this password can\'t sign in any more.' : d.name + ' stops reaching your hub at once. Its downloads stay on it until removed there.',
      confirm: d.kind === 'subsonic' ? 'Revoke' : 'Unpair', danger: true }, function () {
      M.api(DEVICES + '/' + encodeURIComponent(d.id), { method: 'DELETE' }).then(function () {
        if (me) { App.checkPairing(); return; }
        C.reload(); App.toast(d.name + (d.kind === 'subsonic' ? ': password revoked' : ' unpaired'), { icon: 'trash' });
      }, function (err) { App.toast('Couldn\'t unpair ' + d.name + ': ' + err.message, { kind: 'error' }); });
    });
  }

  /* ---------- adding a device: POST /api/pair/codes { code, url, expiresAt }, shown as a QR code of url ---------- */
  var QR_LIB = 'vendor/qrcode/qrcode.js', qrLoading = null;
  function loadQr() {
    if (window.qrcode) return Promise.resolve(window.qrcode);
    return qrLoading || (qrLoading = new Promise(function (resolve, reject) {
      var s = document.createElement('script'); s.src = QR_LIB;
      s.onload = function () { resolve(window.qrcode); };
      s.onerror = function () { qrLoading = null; reject(new Error('The QR encoder didn\'t load')); };
      document.head.appendChild(s);
    }));
  }
  /* the code as an SVG of dark modules with a four-module quiet zone; colours are tokens (always dark on light) */
  function qrSvg(text) {
    return loadQr().then(function (qrcode) {
      var q = qrcode(0, 'M'); q.addData(text); q.make();
      var n = q.getModuleCount(), d = '';
      for (var r = 0; r < n; r++) for (var c = 0; c < n; c++) if (q.isDark(r, c)) d += 'M' + c + ' ' + r + 'h1v1h-1z';
      return '<svg class="hub-qr__svg" viewBox="-4 -4 ' + (n + 8) + ' ' + (n + 8) + '" role="img" aria-label="QR code for pairing" shape-rendering="crispEdges"><rect x="-4" y="-4" width="' + (n + 8) + '" height="' + (n + 8) + '" class="hub-qr__bg"/><path d="' + d + '" class="hub-qr__fg"/></svg>';
    });
  }
  App.action('hub-pair', function () { openPairDialog(); });
  var pairing = null;           // { dlg, timer, code, known: [deviceId] } while the dialog is open
  function openPairDialog() {
    var known = devices().map(function (d) { return d.id; });
    var dlg = App.openDialog({ title: 'add a device', className: 'hub-pair', body: html`<div id="hub-pair-body" aria-live="polite">${ui.skeleton('text', 3)}</div>`,
      foot: html`<button class="btn" data-close>Done</button>`,
      onClose: function () { if (pairing) clearInterval(pairing.timer); pairing = null; C.reload(); } });
    pairing = { dlg: dlg, timer: null, known: known };
    newCode();
  }
  function newCode() {
    var p = pairing; if (!p) return;
    clearInterval(p.timer);
    K.repaint('#hub-pair-body', ui.skeleton('text', 3));
    M.api('/api/pair/codes', { method: 'POST' }).then(function (res) {
      if (pairing !== p) return;
      p.code = res;
      return qrSvg(res.url).then(function (svg) { showCode(p, svg); }, function (err) { showCode(p, null, err); });
    }).catch(function (err) {
      if (pairing === p) K.repaint('#hub-pair-body', K.readError(err, '', 'Couldn\'t make a pair code'));
    });
  }
  function showCode(p, svg, qrErr) {
    var c = p.code;
    K.repaint('#hub-pair-body', html`<div class="hub-pair__grid">
      <div class="hub-qr">${svg ? raw(svg) : fieldError(errText(qrErr))}</div>
      <div class="hub-pair__text">
        <p>On the new device, scan this with the camera, or open lull there and type this code:</p>
        <p class="hub-pair__code px" aria-label="Pair code ${c.code.split('').join(' ')}">${c.code}</p>
        <p class="hub-pair__left" id="hub-pair-left"></p>
        <p class="sys-row__help">Or open <span class="px hub-pair__url">${c.url.slice(0, c.url.lastIndexOf('/') + 1)}<span class="hub-pair__code">${c.url.slice(c.url.lastIndexOf('/') + 1)}</span></span> <button class="btn btn--quiet btn--sm" data-act="hub-copy" data-text="${c.url}">${icon('copy', null, 'ic-16')}Copy</button></p>
        ${httpsNote()}</div></div>`);
    tickCode(p);
    p.timer = setInterval(function () { tickCode(p); }, 1000);
  }
  function tickCode(p) {
    var left = Math.round(p.code.expiresAt - Date.now() / 1000), el = $('#hub-pair-left');
    if (!el) return;
    if (left > 0) { el.textContent = 'Works once, for ' + App.fmt.time(left) + ' more'; return; }
    clearInterval(p.timer);
    K.repaint('#hub-pair-body', ui.emptyState({ art: 'moon', title: 'That code expired', text: 'Pair codes work once, for 10 minutes.', action: { label: 'New code', act: 'hub-pair-new' } }));
  }
  App.action('hub-pair-new', newCode);
  App.action('hub-copy', function (el) {
    var text = el.getAttribute('data-text');
    (navigator.clipboard ? navigator.clipboard.writeText(text) : Promise.reject(new Error('no clipboard'))).then(function () {
      App.toast('Copied', { icon: 'copy' });
    }, function () { App.toast('Couldn\'t copy here. Select the text and copy it instead.', { kind: 'error' }); });
  });

  /* ---------- Subsonic app passwords: POST /api/devices/subsonic { name } -> { device, username, password } (shown once) ---------- */
  App.action('hub-subsonic-new', function () {
    var dlg = App.openDialog({ title: 'app password', body: html`<form class="hub-form" id="hub-ss"><div class="field"><label class="field__label" for="hub-ss-name">Which app is it for?</label>
        <input class="input" id="hub-ss-name" placeholder="Symfonium on my phone" maxlength="60" required autofocus></div><span class="field__error" id="hub-ss-err" hidden></span></form>`,
      foot: html`<button class="btn" data-close>Cancel</button><button class="btn btn--primary" type="submit" form="hub-ss">Make password</button>` });
    $('#hub-ss', dlg.el).addEventListener('submit', function (e) {
      e.preventDefault();
      var name = $('#hub-ss-name', dlg.el).value.trim(), err = $('#hub-ss-err', dlg.el);
      if (!name) return;
      M.api('/api/devices/subsonic', { method: 'POST', body: { name: name } }).then(function (res) {
        dlg.close(); C.reload(); showAppPassword(res);
      }, function (e2) { err.textContent = e2.message; err.hidden = false; });
    });
  });
  function showAppPassword(res) {
    var server = location.href.split('#')[0].replace(/index\.html$/, '').replace(/\/$/, '');
    function line(label, text) {
      return html`<div class="hub-cred"><span class="hub-cred__label">${label}</span><span class="px hub-cred__val">${text}</span><button class="btn-icon btn-icon--bare btn-icon--sm" data-act="hub-copy" data-text="${text}" aria-label="Copy ${label.toLowerCase()}" data-tip="Copy">${icon('copy', null, 'ic-16')}</button></div>`;
    }
    App.openDialog({ title: 'sign in from ' + res.device.name, dismissible: false,
      body: html`<p>Enter these in the app. The password shows only now; if it's lost, make a new one.</p>
        ${line('Server', server)}${line('User name', res.username)}${line('Password', res.password)}`,
      foot: html`<button class="btn btn--primary" data-close>I've entered it</button>` });
  }

  /* live: the list changes (devices online, playing, paired); a new device while the pair dialog is open paired with it */
  App.store.subscribe(function (keys) {
    if (keys.indexOf('devices') < 0) return;
    if (pairing && devices().some(function (d) { return pairing.known.indexOf(d.id) < 0; })) {
      pairing.dlg.close(); App.toast('Paired a new device', { kind: 'ok', icon: 'connect' });
    }
    if ($('#hub-devices')) repaintDevices();
  });

  App.settingsSections.devices = secDevices;
  App.hubKit = { librarySource: librarySource, soulseekRows: soulseekRows, soulseekForm: function () { return html`<div id="hub-slsk">${soulseekForm()}</div>`; },
    httpsNote: httpsNote, deviceName: deviceName, devices: devices };
})();
