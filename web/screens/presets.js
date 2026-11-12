/* screens/presets.js: source presets in Settings › Sources (ADR-0017), loaded with that section (system.js
   settingsNeeds), so no other screen pays for it. Built with the settings kit (App.settingsKit).
   GET /api/sources/presets { presets: [{ id, name, builtin, stream }], device: { id, preset, chosen, phone } }.
   "This device uses" sets the calling device's preset (PUT /api/sources/device { preset|null }); the streaming rows
   (system.js profileRows) edit the preset picked under "Edit preset" (PATCH /api/sources/presets/{id} { stream }),
   read and written by the path 'preset.<field>' (App.presets.value, App.presets.save). "New preset", "Rename" and
   "Delete" ask the hub, which says what is wrong in a sentence (api.md). */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$;
  var K = App.settingsKit;
  var style = document.createElement('style');
  /* the actions under the chips start where the chips do */
  style.textContent = '@layer screens { .sys-presets__acts { justify-content: flex-start; } }';
  document.head.appendChild(style);

  var PRESETS = '/api/sources/presets';
  var editing = null;                                 // the preset the streaming rows edit: the device's until picked
  K.effects['preset.minimum.losslessOnly'] = function (v) { $$('[data-when="stream-kbps"]').forEach(function (r) { r.hidden = !!v; }); };
  /* the streaming cap never sits below the streaming minimum (the hub refuses that): moving one past the other
     moves both, in one PATCH */
  K.custom['preset.maxKbps'] = { set: function (v) {
    var c = { 'preset.maxKbps': v }, min = +K.get('preset.minimum.kbps') || 0;
    if (v && min > v) c['preset.minimum.kbps'] = v;
    presetSave(c); if (c['preset.minimum.kbps'] != null) App.refresh();
  } };
  K.custom['preset.minimum.kbps'] = { set: function (v) {
    var c = { 'preset.minimum.kbps': v }, max = +K.get('preset.maxKbps') || 0;
    if (max && v > max) c['preset.maxKbps'] = v;
    presetSave(c); if (c['preset.maxKbps'] != null) App.refresh();
  } };

  function presets() { return M.peek(PRESETS); }
  function isPreset(path) { return path.indexOf('preset.') === 0; }
  function presetById(id) { var d = presets(); return d && d.presets.filter(function (p) { return p.id === id; })[0]; }
  /* the preset the streaming rows edit; the device's when the one picked is gone */
  function edited() {
    var d = presets(); if (!d) return null;
    if (!presetById(editing)) editing = d.device.preset;
    return presetById(editing) || null;
  }
  function presetValue(path) {
    var p = edited();
    return p ? path.slice(7).split('.').reduce(function (o, k) { return o == null ? o : o[k]; }, p.stream) : undefined;
  }
  /* every preset change answers the whole list and the device's choice */
  function presetAnswer(res) { var d = presets(); if (d && res) { d.presets = res.presets; d.device = res.device; } return res; }
  /* PATCH the edited preset with only the changed fields. The change shows at once; the hub refuses a bad one whole
     (400), and then it is put back and the hub's sentence is shown. */
  function presetSave(changes) {
    var p = edited(); if (!p) return;
    var body = {}, before = {};
    Object.keys(changes).forEach(function (path) {
      var k = path.slice(7);
      before[k] = presetValue(path); K.assign(p.stream, k, changes[path]); K.assign(body, k, changes[path]);
    });
    M.api(PRESETS + '/' + encodeURIComponent(p.id), { method: 'PATCH', body: { stream: body } }).then(presetAnswer, function (err) {
      Object.keys(before).forEach(function (k) { K.assign(p.stream, k, before[k]); });
      App.toast('Couldn\'t save that setting: ' + err.message, { kind: 'error' });
      App.refresh();
    });
  }
  /* another device (or this one) changed the presets: read them again, so the hub's answer is the one shown */
  App.on('server:settings', function (stored) {
    if (!presets() || !stored || !stored.sources || !stored.sources.presets) return;
    if (App.current.path.indexOf('/settings') === 0) M.reload(PRESETS); else M.invalidate([PRESETS]);
  });
  /* presets as chips, the one `current` pressed: which this device uses, or which the rows below edit */
  function presetChips(which, label, current) {
    return html`<div class="cluster sys-presets" role="group" aria-label="${label}" data-presets="${which}">${presets().presets.map(function (p) {
      return html`<button type="button" class="chip" aria-pressed="${p.id === current ? 'true' : 'false'}" data-preset="${p.id}">${icon('check', null, 'chip__check')}${p.name}</button>`;
    })}</div>`;
  }
  document.addEventListener('click', function (e) {
    var b = e.target.closest && e.target.closest('[data-presets] [data-preset]'); if (!b || b.getAttribute('aria-pressed') === 'true') return;
    var id = b.getAttribute('data-preset'), p = presetById(id);
    if (b.closest('[data-presets]').getAttribute('data-presets') === 'edit') { editing = id; App.refresh(); return; }
    presetCall('/api/sources/device', 'PUT', { preset: id }, 'This device uses ' + (p ? p.name : id)).catch(function () {});
  });
  /* a preset change: the answer is shown, or the hub's sentence */
  function presetCall(path, method, body, said) {
    return M.api(path, { method: method, body: body }).then(function (res) {
      presetAnswer(res); App.refresh(); if (said) App.announce(said); return res;
    }, function (err) { App.toast(err.message, { kind: 'error' }); App.refresh(); throw err; });
  }
  App.action('sys-preset-default', function () {
    var d = presets();
    presetCall('/api/sources/device', 'PUT', { preset: null }, d ? 'This device uses ' + (d.device.phone ? 'Phone' : 'Desktop') + ' again' : null).catch(function () {});
  });
  App.action('sys-preset-new', function () {
    var from = edited();
    presetName({ title: 'new preset', value: '', action: 'Make it', note: from ? 'It starts as a copy of ' + from.name + '.' : '' }, function (name) {
      return M.api(PRESETS, { method: 'POST', body: { name: name, from: from && from.id } }).then(function (res) {
        presetAnswer(res); editing = res.presets[res.presets.length - 1].id; App.refresh(); App.announce('Made ' + name);
      });
    });
  });
  App.action('sys-preset-rename', function () {
    var p = edited(); if (!p || p.builtin) return;
    presetName({ title: 'rename preset', value: p.name, action: 'Rename' }, function (name) {
      return M.api(PRESETS + '/' + encodeURIComponent(p.id), { method: 'PATCH', body: { name: name } }).then(function (res) {
        presetAnswer(res); App.refresh(); App.announce('Renamed to ' + name);
      });
    });
  });
  App.action('sys-preset-delete', function () {
    var p = edited(), d = presets(); if (!p || p.builtin || !d) return;
    var mine = d.device.chosen && d.device.preset === p.id;
    App.confirm({ title: 'delete ' + p.name + '?', danger: true, confirm: 'Delete',
      text: 'Devices that use it go back to their default: Phone on a phone, Desktop on anything else.' + (mine ? ' This one too.' : '') }, function () {
      presetCall(PRESETS + '/' + encodeURIComponent(p.id), 'DELETE', undefined, 'Deleted ' + p.name).then(function () { editing = null; App.refresh(); }, function () {});
    });
  });
  /* a name for a new or renamed preset: the dialog stays open until the hub has taken it, and shows its sentence */
  function presetName(o, send) {
    var dlg = App.openDialog({ title: o.title, body: html`<form class="hub-form" id="sys-preset-form"><div class="field"><label class="field__label" for="sys-preset-name">Name</label>
        <input class="input" id="sys-preset-name" value="${o.value}" maxlength="40" required autofocus autocomplete="off" aria-describedby="sys-preset-note sys-preset-err"></div>
        ${o.note ? html`<p class="sys-row__help" id="sys-preset-note">${o.note}</p>` : ''}<span class="field__error" id="sys-preset-err" role="alert" hidden></span></form>`,
      foot: html`<button class="btn" data-close>Cancel</button><button class="btn btn--primary" type="submit" form="sys-preset-form">${o.action}</button>` });
    $('#sys-preset-form', dlg.el).addEventListener('submit', function (e) {
      e.preventDefault();
      var name = $('#sys-preset-name', dlg.el).value.trim(), err = $('#sys-preset-err', dlg.el);
      if (!name) return;
      send(name).then(function () { dlg.close(); }, function (e2) { err.textContent = e2.message; err.hidden = false; });
    });
  }
  /* which preset this device uses, and which one the streaming rows edit */
  function presetRows() {
    var r = M.get(PRESETS);
    if (r.loading) return ui.skeleton('text', 3);
    if (r.error) return K.readError(r.error, PRESETS, 'Couldn\'t load the source presets');
    var dev = r.data.device, p = edited(), def = dev.phone ? 'Phone' : 'Desktop';
    var counts = dev.phone ? 'a phone' : 'a computer';
    return html`
      ${K.row('This device uses', 'Phones use Phone and other devices Desktop until you choose. This device counts as ' + counts + (dev.chosen ? '.' : ', so it uses ' + def + '.'),
        html`${presetChips('device', 'Preset this device uses', dev.preset)}${dev.chosen ? html`<div class="sys-inline sys-presets__acts"><button type="button" class="btn btn--sm" data-act="sys-preset-default">Use ${def}, this device's default</button></div>` : ''}`, { cls: 'sys-row--stack' })}
      ${K.row('Edit preset', 'The streaming settings below change this preset, on every device that uses it.',
        html`${presetChips('edit', 'Preset to edit', p && p.id)}<div class="sys-inline sys-presets__acts">
          <button type="button" class="btn btn--sm" data-act="sys-preset-new">${icon('plus', null, 'ic-16')}New preset</button>
          ${p && !p.builtin ? html`<button type="button" class="btn btn--sm" data-act="sys-preset-rename">${icon('edit', null, 'ic-16')}Rename</button>
          <button type="button" class="btn btn--sm" data-act="sys-preset-delete">${icon('trash', null, 'ic-16')}Delete</button>` : ''}</div>`, { cls: 'sys-row--stack' })}`;
  }

  App.presets = { rows: presetRows, edited: edited, value: presetValue, save: presetSave };
})();
