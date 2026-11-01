/* screens/appearance.js.
   Settings > Appearance: design, theme, motion, text size and Classic mode's skin settings, all per device
   (App.setPref, and App.storage for Classic mode's two). Theme Studio grows here (ADR-0004).
   Builds with the shared settings controls from screens/system.js (App.settingsKit), which loads first. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, $ = App.$;
  var K = App.settingsKit, row = K.row, choice = K.choice, switchRow = K.switchRow, group = K.group, custom = K.custom, get = K.get;
  var SKIN_FAVS = '/api/skins/favourites';

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

  App.settingsSections.appearance = secAppearance;
  App.on('design', function (name) {
    if (!designFocus) return; designFocus = false;
    var b = $('.sys-design[data-value="' + name + '"]'); if (b) b.focus();
  });
})();
