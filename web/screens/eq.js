/* screens/eq.js.
   The equaliser's face; web/eq.js (LullEq) holds the settings, per device. App.eq.panel({ full }) is its markup:
   the switch, the presets, Bass and Treble, and with `full` (Settings > Playback) every band and the level too.
   App.eq.open(anchor) is the player's quick way in (the player's "…" > Equaliser): a popover, a sheet on phones.
   Panels never re-render while open: every change, from a panel, Classic's Webamp or another window, writes the
   settings and each panel on screen updates its values in place, so a slider being dragged keeps its pointer. */
(function () {
  'use strict';
  var html = App.html, icon = App.icon, $ = App.$, $$ = App.$$, E = window.LullEq;
  var bandsOpen = false;                                    // the full panel's bands, open or not, across re-renders
  var moving = null;                                        // the slider the listener drags: its value is theirs

  function fmt(db) { db = Math.round(db * 10) / 10; return (db > 0 ? '+' : db < 0 ? '−' : '') + Math.abs(db) + ' dB'; }
  function hz(f) { return f >= 1000 ? f / 1000 + ' kHz' : f + ' Hz'; }
  function fill(db) { return (Math.max(-12, Math.min(12, db)) + 12) / 24 * 100 + '%'; }
  function slider(attr, label, v) {
    return html`<input class="range range--centred" type="range" min="-12" max="12" step="0.5" value="${v}" style="${'--fill:' + fill(v)}"
      aria-label="${label}" aria-valuetext="${fmt(v)}" ${App.raw(attr)}>`;
  }
  function levelText(s) { return (s.preamp == null ? 'Automatic, ' : 'Set by hand, ') + fmt(E.level(s)); }

  /* the bands' curve, as the filters shape it (±12 dB over 20 Hz to 20 kHz) */
  var CURVE_HZ = Array.from({ length: 64 }, function (_, i) { return 20 * Math.pow(1000, i / 63); });
  function curve(s) {
    var r = E.response(s, CURVE_HZ);
    return r.map(function (db, i) { return (i ? 'L' : 'M') + (i / 63 * 200).toFixed(1) + ' ' + (30 - Math.max(-12, Math.min(12, db)) * 2.4).toFixed(1); }).join('');
  }

  function onSwitch(s, id, more) {
    return html`<button type="button" class="switch" role="switch" aria-checked="${s.on ? 'true' : 'false'}" aria-labelledby="${id}" data-eq-on${App.raw(more)}></button>`;
  }

  function panel(o) {
    o = o || {};
    if (!App.player.canEqualize()) {
      return html`<div class="eqp eqp--none"><p class="eqp__note">${icon('info', null, 'ic-16')}This version of the app has no equaliser yet. Update the app to use it.</p></div>`;
    }
    var s = E.read(), id = App.uid('eqp'), current = E.preset(s);
    var presets = E.PRESETS.map(function (p) {
      return html`<button type="button" class="chip" aria-pressed="${current === p.id ? 'true' : 'false'}" data-eq-preset="${p.id}">${p.name}</button>`;
    });
    return html`<div class="eqp ${o.full ? 'eqp--full' : ''} ${s.on ? '' : 'is-off'}" data-eqp>
      ${o.full ? html`<div class="sys-row switch-row eqp__top">
          <div class="sys-row__text"><span class="sys-row__label" id="${id}">Equaliser</span><span class="sys-row__help" id="${id + '-h'}">Bass and treble, or a preset. On this device only: Classic mode's equaliser window is this one too.</span></div>
          <div class="sys-row__ctl">${onSwitch(s, id, ' aria-describedby="' + id + '-h"')}</div>
        </div>` : html`<div class="switch-row eqp__top"><span class="switch-row__label eqp__title" id="${id}">Equaliser</span>${onSwitch(s, id, '')}</div>`}
      <div class="eqp__presets" role="group" aria-label="Presets">${presets}<span class="chip eqp__custom" data-eq-custom ${current ? App.raw('hidden') : ''}>Custom</span></div>
      <div class="eqp__tones">
        <label class="eqp__tone"><span class="eqp__label">Bass</span>${slider('data-eq-tone="bass"', 'Bass', E.bass(s))}<output class="eqp__val num" data-eq-out="bass">${fmt(E.bass(s))}</output></label>
        <label class="eqp__tone"><span class="eqp__label">Treble</span>${slider('data-eq-tone="treble"', 'Treble', E.treble(s))}<output class="eqp__val num" data-eq-out="treble">${fmt(E.treble(s))}</output></label>
      </div>
      ${o.full ? html`<details class="eqp__more" data-eq-bands ${bandsOpen ? App.raw('open') : ''}>
          <summary class="eqp__summary">${icon('chevron-down', null, 'ic-16 eqp__chev')}Every band</summary>
          <svg class="eqp__curve" viewBox="0 0 200 60" preserveAspectRatio="none" aria-hidden="true"><path class="eqp__zero" d="M0 30H200"/><path class="eqp__line" data-eq-curve d="${curve(s)}"/></svg>
          <div class="eqp__bands">${E.BANDS.map(function (f, i) {
            return html`<label class="eqp__tone"><span class="eqp__label num">${hz(f)}</span>${slider('data-eq-band="' + i + '"', hz(f), s.bands[i])}<output class="eqp__val num" data-eq-out="${'b' + i}">${fmt(s.bands[i])}</output></label>`;
          })}
            <label class="eqp__tone eqp__level"><span class="eqp__label">Level</span>${slider('data-eq-level', 'Level', E.level(s))}<output class="eqp__val num" data-eq-out="level">${fmt(E.level(s))}</output></label>
            <p class="eqp__levelnote"><span data-eq-out="levelnote">${levelText(s)}</span><button type="button" class="btn btn--quiet btn--sm" data-eq-auto ${s.preamp == null ? App.raw('hidden') : ''}>Make automatic</button></p>
            <p class="eqp__help">The level comes down by as much as the curve goes up, so boosting never distorts.</p>
          </div>
        </details>` : html`<a class="eqp__link" href="#/settings/playback">${icon('settings', null, 'ic-16')}Every band in Settings</a>`}
    </div>`;
  }

  /* every panel on screen shows the settings, leaving the slider being dragged as it is */
  function setRange(el, v) {
    if (el === moving) return;
    el.value = v; el.style.setProperty('--fill', fill(v)); el.setAttribute('aria-valuetext', fmt(v));
  }
  function show(s) {
    var current = E.preset(s);
    $$('[data-eqp]').forEach(function (p) {
      p.classList.toggle('is-off', !s.on);
      var sw = $('[data-eq-on]', p); if (sw) sw.setAttribute('aria-checked', String(s.on));
      $$('[data-eq-preset]', p).forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-eq-preset') === current)); });
      var custom = $('[data-eq-custom]', p); if (custom) custom.hidden = !!current;
      var vals = { bass: E.bass(s), treble: E.treble(s), level: E.level(s) };
      s.bands.forEach(function (v, i) { vals['b' + i] = v; });
      $$('[data-eq-out]', p).forEach(function (out) {
        var k = out.getAttribute('data-eq-out');
        out.textContent = k === 'levelnote' ? levelText(s) : fmt(vals[k]);
      });
      $$('[data-eq-tone]', p).forEach(function (r) { setRange(r, vals[r.getAttribute('data-eq-tone')]); });
      $$('[data-eq-band]', p).forEach(function (r) { setRange(r, vals['b' + r.getAttribute('data-eq-band')]); });
      $$('[data-eq-level]', p).forEach(function (r) { setRange(r, vals.level); });
      var auto = $('[data-eq-auto]', p); if (auto) auto.hidden = s.preamp == null;
      var line = $('[data-eq-curve]', p); if (line) line.setAttribute('d', curve(s));
    });
  }
  E.on(show);

  document.addEventListener('input', function (e) {
    var r = e.target, v = +r.value, s = E.read();
    if (!r.closest || !r.closest('[data-eqp]')) return;
    if (r.hasAttribute('data-eq-tone')) s = r.getAttribute('data-eq-tone') === 'bass' ? E.withBass(s, v) : E.withTreble(s, v);
    else if (r.hasAttribute('data-eq-band')) s = E.withBand(s, +r.getAttribute('data-eq-band'), v);
    else if (r.hasAttribute('data-eq-level')) s = E.withPreamp(s, v);
    else return;
    if (!s.on) s.on = true;                                   // moving a slider means the listener wants to hear it
    r.style.setProperty('--fill', fill(v)); r.setAttribute('aria-valuetext', fmt(v));
    moving = r;
    try { E.write(s); } finally { moving = null; }
  });
  document.addEventListener('click', function (e) {
    var t = e.target.closest && e.target.closest('[data-eq-on], [data-eq-preset], [data-eq-auto]');
    if (!t || !t.closest('[data-eqp]')) return;
    var s = E.read();
    if (t.hasAttribute('data-eq-on')) { s.on = !s.on; App.announce('Equaliser ' + (s.on ? 'on' : 'off')); }
    else if (t.hasAttribute('data-eq-preset')) { s = E.withPreset(s, t.getAttribute('data-eq-preset')); App.announce(t.textContent + ' equaliser'); }
    else s = E.withPreamp(s, null);
    E.write(s);
  });
  document.addEventListener('toggle', function (e) {
    if (e.target.matches && e.target.matches('[data-eq-bands]')) bandsOpen = e.target.open;
  }, true);

  function open(anchor) {
    var body = panel();
    if (window.innerWidth < 640) App.openSheet({ label: 'Equaliser', className: 'eq-sheet', body: body });
    else App.popover(anchor, body, { label: 'Equaliser', className: 'eq-pop' });
  }

  App.eq = { panel: panel, open: open };
})();
