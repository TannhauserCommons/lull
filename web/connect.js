/* lull Connect (ADR-0005): every paired device sees the others, reports what it plays, and can be controlled.
   - The device list lives in the store as `devices` { items, me, loading, error } (GET /api/devices), kept current
     by `device.online` and `device.state` events. Settings > Devices (hub.js) reads it too.
   - This device reports its state to POST /api/devices/me/state when it changes: at once on a new song, play or
     pause, otherwise at most once a second; an unchanged state is sent again only as the slow heartbeat.
   - `device.command` events for this device drive the engine; `takeover` reports at once and pauses.
   - The picker (App.connect.open(anchor)): the devices, a remote's controls, and "Play here" (takeover). */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, P = App.player, ps = P.state;
  var DEVICES = '/api/devices';
  var C = App.connect = {};

  /* ---------- the device list ---------- */
  App.store.set('devices', { items: [], me: null, loading: true, error: null });
  function devices() { return App.store.get('devices'); }
  C.reload = function () {
    return M.api(DEVICES).then(function (r) {
      (r.items || []).forEach(function (d) { if (d.state) d.state.recv = Date.now(); });
      App.store.set('devices', { items: r.items || [], me: r.me, loading: false, error: null });
    }, function (err) { App.store.set('devices', { items: devices().items, me: devices().me, loading: false, error: err }); });
  };
  C.byId = function (id) { return devices().items.filter(function (d) { return d.id === id; })[0]; };
  /* devices that can play and are online, other than this one */
  C.others = function () {
    var s = devices();
    return s.items.filter(function (d) { return d.id !== s.me && d.kind !== 'subsonic' && d.online; });
  };
  function patchDevice(id, patch) {
    var s = devices(), d = C.byId(id);
    if (!d) { if (!s.loading) C.reload(); return; }       // a device paired since the list was read
    Object.assign(d, patch);
    App.store.set('devices', s);
  }
  App.on('server:device.online', function (e) { patchDevice(e.deviceId, { online: e.online }); });
  App.on('server:device.state', function (e) {
    if (e.deviceId === devices().me) return;
    patchDevice(e.deviceId, { state: Object.assign({}, e, { recv: Date.now() }), online: true });
  });
  /* a remote's position now: what it reported plus the time since, while it plays */
  C.position = function (st) {
    if (!st) return 0;
    var t = M.track(st.trackId), p = st.position + (st.playing ? (Date.now() - st.recv) / 1000 : 0);
    return t && t.duration ? Math.min(p, t.duration) : p;
  };

  /* ---------- reporting this device's state ---------- */
  var HEARTBEAT = 30000, sendTimer = null, lastSent = 0, lastBody = null, reporting = true;
  function stateBody() {
    return { trackId: P.currentId(), position: Math.round(ps.pos * 10) / 10, playing: ps.playing, volume: ps.muted ? 0 : ps.volume,
      queue: ps.queue.map(function (q) { return q.trackId; }), queueIndex: ps.index };
  }
  /* force: a takeover waits for this device's state event, so it is sent even when unchanged */
  function send(force) {
    clearTimeout(sendTimer); sendTimer = null;
    var body = stateBody(), key = JSON.stringify(body);
    if (!force && key === lastBody && Date.now() - lastSent < HEARTBEAT - 1000) return;   // nothing new since the last report
    lastSent = Date.now(); lastBody = key;
    M.api(DEVICES + '/me/state', { method: 'POST', body: body }).catch(function (err) {
      if (err.status === 404) reporting = false;           // a hub without Connect: stop asking until it restarts
    });
  }
  function report(now) {
    if (!reporting || !App.server.authorized || !devices().me) return;
    if (now || Date.now() - lastSent > 1000) { send(); return; }
    if (!sendTimer) sendTimer = setTimeout(send, 1000 - (Date.now() - lastSent));
  }
  App.on('player', function (kind) {
    if (kind === 'track' || kind === 'play' || kind === 'ended') report(true);
    else if (kind === 'seek' || kind === 'volume' || kind === 'queue') report(false);
  });
  setInterval(function () { if (ps.playing) report(false); }, HEARTBEAT);

  /* ---------- commands for this device ---------- */
  App.on('server:device.command', function (e) {
    if (e.deviceId !== devices().me) return;
    var from = C.byId(e.from), name = from ? from.name : 'Another device';
    switch (e.action) {
      case 'play': P.play(); break;
      case 'pause': P.pause(); break;
      case 'next': P.next(); break;
      case 'prev': P.prev(); break;
      case 'seek': P.seek(+e.value || 0); break;
      case 'volume': P.setVolume(+e.value); break;
      case 'takeover':
        send(true); P.pause();
        App.toast(name + ' took over the music', { icon: 'connect' });
        break;
    }
  });

  /* when this device is paired (again), and after every reconnect: read the list, report, accept reports again.
     `server` fires several times while the list loads, so one start runs at a time; a reconnect reports afresh. */
  var starting = null;
  function start() {
    if (starting) return;
    reporting = true; lastBody = null;
    starting = C.reload().then(function () { starting = null; report(true); });
  }
  App.on('server', function (srv) { if (srv.authorized && srv.checked && devices().loading) start(); });
  App.on('server:resync', start);
  App.on('server:hub.restarting', function () { reporting = true; });

  /* ---------- controlling another device ---------- */
  function command(d, action, value) {
    return M.api(DEVICES + '/' + encodeURIComponent(d.id) + '/command', { method: 'POST', body: { action: action, value: value } })
      .catch(function (err) { App.toast('Couldn\'t reach ' + d.name + ': ' + err.message, { kind: 'error' }); throw err; });
  }
  C.command = command;
  /* the ids a remote state names that this device hasn't seen: read them so the queue can play here */
  function learnTracks(ids) {
    var unknown = ids.filter(function (id, i) { return !M.track(id) && ids.indexOf(id) === i; });
    return Promise.all(unknown.map(function (id) { return M.api('/api/tracks/' + encodeURIComponent(id)).catch(function () { return null; }); }));
  }
  /* "Play here": the other device sends its state and pauses; its queue and position continue on this one */
  C.takeover = function (d) {
    var waited = new Promise(function (resolve) {
      var off = App.on('server:device.state', function (e) { if (e.deviceId === d.id) { off(); resolve(); } });
      setTimeout(function () { off(); resolve(); }, 3000);
    });
    return command(d, 'takeover').then(function () { return waited; }).then(function () {
      var st = C.byId(d.id).state;
      if (!st || !st.queue || !st.queue.length) { App.toast(d.name + ' had nothing queued', { icon: 'info' }); return; }
      var at = C.position(st), queue = st.queue.slice(Math.max(0, st.queueIndex - 50), st.queueIndex + 150);
      var index = st.queueIndex - Math.max(0, st.queueIndex - 50);
      return learnTracks(queue).then(function () {
        var known = queue.filter(function (id) { return M.track(id); }), cur = queue[index];
        if (known.indexOf(cur) < 0) { App.toast('Couldn\'t read the song ' + d.name + ' was playing', { kind: 'error' }); return; }
        P.playList(known, known.indexOf(cur), { at: at, autoplay: !!st.playing, context: null });   // the state doesn't say where it played from
        App.toast('Playing here, from ' + d.name, { kind: 'ok', icon: 'connect' });
      });
    }).catch(function () {});
  };

  /* ---------- the picker ---------- */
  var picked = null;                           // the remote whose controls are open
  function deviceIcon(d) { return icon(d.kind === 'app' ? 'device' : 'globe'); }
  function stateLine(d) {
    var st = d.state, t = st && st.trackId && M.track(st.trackId);
    if (!t) return 'Online, nothing playing';
    return (st.playing ? 'Playing ' : 'Paused on ') + t.title + ', ' + t.artist;
  }
  function remoteControls(d) {
    var st = d.state, t = st && st.trackId && M.track(st.trackId);
    if (!t) return html`<div class="cx-remote"><p class="cx-remote__none">Nothing playing on ${d.name}.</p></div>`;
    var dur = t.duration || 0, pos = C.position(st), vol = Math.round((st.volume == null ? 1 : st.volume) * 100);
    return html`<div class="cx-remote" data-remote="${d.id}">
      <div class="cx-remote__now">${ui.cover(t, 48)}<div class="cx-remote__text"><span class="cx-remote__title">${t.title}</span><span class="cx-remote__by">${t.artist}</span></div></div>
      <div class="cx-remote__seek"><span class="num" data-cx="pos">${App.fmt.time(pos)}</span>
        <input class="range" type="range" min="0" max="${Math.max(1, Math.round(dur))}" step="1" value="${Math.round(pos)}" style="--fill:${dur ? pos / dur * 100 : 0}%" aria-label="Seek on ${d.name}" data-cx-seek="${d.id}">
        <span class="num">${App.fmt.time(dur)}</span></div>
      <div class="cx-remote__keys">
        <button class="btn-icon btn-icon--bare" data-act="cx-cmd" data-cmd="prev" data-id="${d.id}" aria-label="Previous on ${d.name}" data-tip="Previous">${icon('prev')}</button>
        <button class="btn-icon" data-act="cx-cmd" data-cmd="${st.playing ? 'pause' : 'play'}" data-id="${d.id}" aria-label="${st.playing ? 'Pause' : 'Play'} on ${d.name}" data-tip="${st.playing ? 'Pause' : 'Play'}">${icon(st.playing ? 'pause' : 'play')}</button>
        <button class="btn-icon btn-icon--bare" data-act="cx-cmd" data-cmd="next" data-id="${d.id}" aria-label="Next on ${d.name}" data-tip="Next">${icon('next')}</button>
        <span class="cx-remote__vol">${icon('volume-2', null, 'ic-16')}<input class="range" type="range" min="0" max="100" step="5" value="${vol}" style="--fill:${vol}%" aria-label="Volume on ${d.name}" data-cx-vol="${d.id}"></span>
      </div>
      <button class="btn btn--primary btn--block" data-act="cx-here" data-id="${d.id}">${icon('play')}Play here</button>
    </div>`;
  }
  function pickerBody() {
    var s = devices(), others = C.others(), meName = (C.byId(s.me) || {}).name || 'This device';
    if (s.loading) return ui.skeleton('row', 2);
    if (s.error) return html`<div class="cx-err">${ui.emptyState({ art: 'star', title: 'Couldn\'t list your devices', text: s.error.message, action: { label: 'Try again', act: 'cx-reload' } })}</div>`;
    return html`<ul class="cx-list">
      <li class="cx-dev is-here"><span class="cx-dev__ic">${icon('connect')}</span><span class="cx-dev__text"><span class="cx-dev__name">${meName}</span>
        <span class="cx-dev__sub">This device${P.current() ? ', ' + (ps.playing ? 'playing ' : 'paused on ') + P.current().title : ''}</span></span>${ui.statusGlyph('ok', 'This device')}</li>
      ${others.map(function (d) {
        var open = picked === d.id;
        return html`<li class="cx-dev ${open ? 'is-open' : ''}"><button class="cx-dev__pick" data-act="cx-pick" data-id="${d.id}" aria-expanded="${open ? 'true' : 'false'}">
          <span class="cx-dev__ic">${deviceIcon(d)}</span><span class="cx-dev__text"><span class="cx-dev__name">${d.name}</span><span class="cx-dev__sub">${stateLine(d)}</span></span>
          ${icon(open ? 'chevron-up' : 'chevron-down', null, 'ic-16')}</button>${open ? remoteControls(d) : ''}</li>`;
      })}</ul>
      ${others.length ? '' : html`<p class="cx-note">No other device is online. Open lull on another paired device, or pair one in Settings, Devices.</p>`}
      <p class="cx-foot"><a class="btn btn--quiet btn--sm" href="#/settings/devices" data-act="cx-close">${icon('settings', null, 'ic-16')}Manage devices</a></p>`;
  }
  var open = null;                             // { el, close } of the open picker
  /* repaint in place; focus goes back to the same control (same action and device) */
  function repaint() {
    if (!open || !document.contains(open.el)) return;
    var f = document.activeElement, mine = f && open.el.contains(f);
    if (mine && f.matches('input[type="range"]')) return;          // don't yank a slider mid-drag
    var sel = mine && f.getAttribute('data-act') ? '[data-act="' + f.getAttribute('data-act') + '"]' + (f.getAttribute('data-id') ? '[data-id="' + f.getAttribute('data-id') + '"]' : '') : null;
    App.patch($('.cx-body', open.el), pickerBody());
    var again = sel && open.el.querySelector(sel); if (again) again.focus();
    if (open.place) open.place();
  }
  C.open = function (anchor) {
    if (devices().error || !devices().items.length) C.reload();
    picked = null;
    var body = html`<div class="cx-body">${pickerBody()}</div>`;
    var onClose = function () { open = null; };
    open = window.innerWidth < 640
      ? App.openSheet({ title: 'play on', className: 'cx-sheet', body: body, onClose: onClose })
      : App.popover(anchor, html`<div class="popover__head"><h2 class="cx-title">play on</h2></div>${body}`, { label: 'Devices', className: 'cx-pop', onClose: onClose });
  };
  App.store.subscribe(function (keys) { if (keys.indexOf('devices') >= 0) repaint(); });
  App.on('player', function (kind) { if (kind === 'track' || kind === 'play') repaint(); });
  /* positions of remotes move while the picker is open */
  setInterval(function () {
    if (!open || !picked) return;
    var d = C.byId(picked), el = open.el.querySelector('[data-cx="pos"]'), seek = open.el.querySelector('[data-cx-seek]');
    if (!d || !el || !seek || document.activeElement === seek) return;
    var t = d.state && M.track(d.state.trackId), p = C.position(d.state);
    el.textContent = App.fmt.time(p); seek.value = Math.round(p);
    if (t && t.duration) seek.style.setProperty('--fill', p / t.duration * 100 + '%');
  }, 1000);

  App.action('connect', function (el) { C.open(el); });
  App.action('cx-reload', function () { C.reload(); });
  App.action('cx-close', function () { App.closePopover(); App.closeTop(); });
  App.action('cx-pick', function (el) {
    var id = el.getAttribute('data-id'); picked = picked === id ? null : id; repaint();
    var b = open && open.el.querySelector('[data-act="cx-pick"][data-id="' + id + '"]'); if (b) b.focus();
    var d = C.byId(id); if (d && d.state) learnTracks([d.state.trackId]).then(repaint);
  });
  App.action('cx-cmd', function (el) {
    var d = C.byId(el.getAttribute('data-id')), cmd = el.getAttribute('data-cmd'); if (!d) return;
    command(d, cmd).then(function () {
      if (d.state && (cmd === 'play' || cmd === 'pause')) { d.state.position = C.position(d.state); d.state.recv = Date.now(); d.state.playing = cmd === 'play'; repaint(); }
    }, function () {});
  });
  App.action('cx-here', function (el) { var d = C.byId(el.getAttribute('data-id')); App.closePopover(); App.closeTop(); if (d) C.takeover(d); });
  document.addEventListener('change', function (e) {
    var s = e.target.closest && e.target.closest('[data-cx-seek], [data-cx-vol]'); if (!s) return;
    var seek = s.hasAttribute('data-cx-seek'), d = C.byId(s.getAttribute(seek ? 'data-cx-seek' : 'data-cx-vol')); if (!d) return;
    var v = seek ? +s.value : +s.value / 100;
    command(d, seek ? 'seek' : 'volume', v).then(function () {
      if (!d.state) return;
      if (seek) { d.state.position = v; d.state.recv = Date.now(); } else d.state.volume = v;
    }, function () {});
  });
  document.addEventListener('input', function (e) {
    var s = e.target.closest && e.target.closest('[data-cx-seek], [data-cx-vol]'); if (!s) return;
    s.style.setProperty('--fill', (s.value - s.min) / (s.max - s.min) * 100 + '%');
  });
})();
