/* screens/appearance.js.
   Settings > Appearance: design, theme, motion, text size, the layout's reset and Classic mode's skin settings, all per device
   (App.setPref, and App.storage for Classic mode's two); the listener's own themes (import by drag and drop, export,
   delete) and the way into Theme Studio (designs/studio.js, ADR-0004, docs/themes.md). Live reload of a stored theme
   runs here too.
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
    App.announce(App.designs[v].label + (App.designs[v].builtIn ? ' design' : ' theme'));
  } };
  custom['device.theme'] = { get: function () { return App.prefs().theme; }, set: function (v) { App.setPref('theme', v === 'system' ? null : v); } };
  custom['device.motion'] = { get: function () { return App.prefs().motion; }, set: function (v) { App.setPref('motion', v); } };
  custom['device.textSize'] = { get: function () { return App.prefs().textSize; }, set: function (v) { App.setPref('textSize', v); } };
  /* Classic mode reads these two from the same storage keys */
  custom['device.defaultSkin'] = { get: function () { return App.storage('defaultSkin'); }, set: function (v) { App.storage('defaultSkin', v); } };
  custom['device.showNsfwSkins'] = { get: function () { return App.storage('showNsfwSkins') === true; }, set: function (v) { App.storage('showNsfwSkins', v); } };
  /* design preview card: the package's screenshot (assets/preview-{day,night}.webp, when d.preview) over a tiny
     picture of the shell (rail / topnav / dock) in its swatches, which shows while the image loads or when there is none */
  function designCard(name, on) {
    var d = App.designs[name], sw = d.swatches;
    var pv = new URL(Designs.packageUrl(name) + 'assets/preview', location.href).href;   // absolute: a url() in a custom property resolves against the stylesheet that uses it
    var shot = d.preview ? ';--pv-day:url("' + pv + '-day.webp");--pv-night:url("' + pv + '-night.webp")' : '';
    return html`<button type="button" class="sys-design" role="radio" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-value="${name}"
      style="${'--d1:' + sw[0] + ';--d2:' + sw[1] + ';--d3:' + sw[2] + ';--d4:' + sw[3] + shot}">
      <span class="sys-design__pv${d.preview ? ' has-shot' : ''}" data-shell="${d.shell}" aria-hidden="true"><i class="sys-design__nav"></i><i class="sys-design__win"><b></b><b></b><b></b></i><i class="sys-design__win sys-design__win--b"><b></b><b></b></i><i class="sys-design__bar"><b></b></i><i class="sys-design__dock"></i></span>
      <span class="sys-design__name">${d.label}${icon('check', null, 'ic-16 sys-design__check')}</span>
      <span class="sys-design__desc">${d.description || (d.author ? 'By ' + d.author : 'Your theme')}</span></button>`;
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
  /* the listener's themes: edit in Theme Studio, export, delete; import by drop or file */
  function myThemes() {
    var mine = App.designOrder.filter(function (n) { return !App.designs[n].builtIn; });
    return html`<div class="sys-row sys-row--stack sys-themes" data-drop-theme>
        <div class="sys-row__text"><span class="sys-row__label">Your themes</span><span class="sys-row__help">Stored on your hub, so every device can use them. Drop a .lulltheme file here to add one, or make one in Theme Studio.</span></div>
        <div class="sys-row__ctl">${mine.length ? html`<ul class="sys-themes__list" role="list">${mine.map(function (n) {
          var d = App.designs[n];
          return html`<li class="sys-themes__item"><span class="sys-themes__name">${d.label}</span><span class="sys-themes__meta">${d.extends ? 'on ' + App.designs[d.extends].label : 'from scratch'}${d.author ? ', by ' + d.author : ''}</span>
            <span class="sys-themes__acts"><button class="btn btn--sm btn--quiet" data-act="theme-edit" data-id="${n}">${icon('edit', null, 'ic-16')}Edit</button><a class="btn btn--sm btn--quiet" href="${'api/themes/' + n + '.lulltheme'}" download="${n + '.lulltheme'}">${icon('export', null, 'ic-16')}Export</a><button class="btn-icon btn-icon--sm btn-icon--bare" data-act="theme-delete" data-id="${n}" aria-label="${'Delete ' + d.label}" data-tip="Delete">${icon('trash', null, 'ic-16')}</button></span></li>`;
        })}</ul>` : ''}
          <div class="cluster"><button class="btn btn--primary btn--sm" data-act="theme-studio">${icon('sparkle', null, 'ic-16')}Open Theme Studio</button>
          <label class="btn btn--sm">${icon('import', null, 'ic-16')}Import a theme<input class="vh" type="file" accept=".lulltheme,.zip" data-theme-file></label></div></div></div>`;
  }
  function secAppearance() {
    var p = App.prefs();
    loadCss();
    if (!listed) { listed = true; Designs.refresh().then(function () { if (onAppearance()) App.refresh(); }, function () {}); }
    return html`<div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-design-l">Design</span><span class="sys-row__help">The whole look: colours, type, window chrome and layout. Applies at once on this device.</span></div>
        <div class="sys-row__ctl"><div class="sys-designs" role="radiogroup" aria-labelledby="sys-design-l" data-key="device.design">${App.designOrder.map(function (n) { return designCard(n, n === p.design); })}</div></div></div>
      ${row('Theme', 'Match system follows your device. T switches day and night anywhere.', choice('device.theme', 'Theme', [['system', 'Match system'], ['day', 'Day'], ['night', 'Night']]))}
      ${row('Motion', 'Reduced swaps slides and loops for quick fades. Full ignores the system setting.', choice('device.motion', 'Motion', [['system', 'Match system'], ['reduce', 'Reduced'], ['full', 'Full']]))}
      ${row('Text size', 'Scales all text and spacing on this device.', choice('device.textSize', 'Text size', [[90, 'Smaller'], [100, 'Default'], [112, 'Larger'], [125, 'Largest']], 'sys-seg--4'))}
      ${group('layout')}
      ${row('Reset layout', 'Puts the side panel, the Now Playing column, and side-by-side search back as they came. This device only.', html`<button class="btn btn--sm" data-act="layout-reset">${icon('undo', null, 'ic-16')}Reset layout</button>`)}
      ${group('themes')}
      ${myThemes()}
      ${group('classic mode')}
      <div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-skin-l">Default skin</span><span class="sys-row__help">Used when Classic mode opens on this device. Your favourite skins are listed; find more in Classic mode.</span></div>
        <div class="sys-row__ctl">${skinPicker()}<a class="btn btn--quiet btn--sm" href="#/classic">${icon('classic')}Browse skins in Classic mode</a></div></div>
      ${switchRow('device.showNsfwSkins', 'Show NSFW skins', 'Hidden by default in the skin browser on this device.')}`;
  }
  var listed = false;                              // GET /api/themes once per page; theme.changed keeps it current
  function onAppearance() { return /^\/settings\/appearance/.test(App.current.path); }

  App.settingsSections.appearance = secAppearance;
  App.on('design', function (name) {
    if (!designFocus) return; designFocus = false;
    var b = $('.sys-design[data-value="' + name + '"]'); if (b) b.focus();
  });

  /* ---------- live reload: a stored theme's files changed on the hub ---------- */
  App.on('server:theme.changed', function (e) {
    var id = e && e.id;
    Designs.refresh().then(function () {
      if (id === Designs.current && !App.designs[id]) { App.setDesign(Designs.fallback); return; }   // deleted while in use
      return Designs.reload(id).then(function (inUse) { if (inUse) App.setDesign(Designs.current, true); });
    }).then(function () { if (onAppearance()) App.refresh(); }).catch(function (err) { console.error('Theme reload failed', err); });
  });

  /* ---------- import, export, delete ---------- */
  function importFile(file, replace) {
    if (!/\.(lulltheme|zip)$/i.test(file.name)) { App.toast(file.name + ' is not a .lulltheme file', { kind: 'error' }); return; }
    fetch(M.url('/api/themes' + (replace ? '?replace=true' : '')), { method: 'POST', body: file, credentials: 'same-origin' })
      .then(function (r) { return r.json().then(function (j) { return { status: r.status, body: j }; }); })
      .then(function (r) {
        if (r.status === 409) {
          App.confirm({ title: 'Replace the theme?', text: r.body.message + '. Importing replaces it with this file.', confirm: 'Replace' }, function () { importFile(file, true); });
          return;
        }
        if (r.status !== 200) throw new Error(r.body.message || 'The hub answered ' + r.status);
        App.toast('Added ' + r.body.name, { kind: 'ok', icon: 'sparkle', action: 'Use it', onAction: function () { App.setPref('design', r.body.id); } });
        return Designs.refresh().then(function () { if (onAppearance()) App.refresh(); });
      })
      .catch(function (err) { App.toast('Couldn\'t import ' + file.name + ': ' + err.message, { kind: 'error' }); });
  }
  document.addEventListener('change', function (e) {
    var f = e.target.closest && e.target.closest('[data-theme-file]');
    if (f && f.files && f.files[0]) { importFile(f.files[0]); f.value = ''; }
  });
  function droppable(e) { return e.dataTransfer && [].indexOf.call(e.dataTransfer.types || [], 'Files') >= 0 && e.target.closest && e.target.closest('[data-drop-theme]'); }
  document.addEventListener('dragover', function (e) {
    var z = droppable(e); if (!z) return;
    e.preventDefault(); z.classList.add('is-drop');
  });
  document.addEventListener('dragleave', function (e) { var z = e.target.closest && e.target.closest('[data-drop-theme]'); if (z && !z.contains(e.relatedTarget)) z.classList.remove('is-drop'); });
  document.addEventListener('drop', function (e) {
    var z = droppable(e); if (!z) return;
    e.preventDefault(); z.classList.remove('is-drop');
    [].forEach.call(e.dataTransfer.files, function (f) { importFile(f); });
  });
  App.action('theme-delete', function (el) {
    var id = el.getAttribute('data-id'), d = App.designs[id];
    App.confirm({ title: 'Delete ' + d.label + '?', text: 'It goes from your hub and from every device using it (they switch to ' + App.designs[Designs.fallback].label + '). Export it first to keep a copy.', confirm: 'Delete', danger: true }, function () {
      if (Designs.current === id) App.setDesign(Designs.fallback);     // first, so nothing asks for its files once they're gone
      M.api('/api/themes/' + encodeURIComponent(id), { method: 'DELETE' }).then(function () {
        App.toast('Deleted ' + d.label, { icon: 'trash' });
        return Designs.refresh().then(function () { if (onAppearance()) App.refresh(); });
      }).catch(function (err) { App.toast('Couldn\'t delete ' + d.label + ': ' + err.message, { kind: 'error' }); });
    });
  });
  /* Theme Studio (designs/studio.js) loads the first time it opens: it is most of this screen's code */
  var studio = null;
  function openStudio(id) {
    (studio || (studio = new Promise(function (ok, fail) {
      var sc = document.createElement('script');
      sc.src = Designs.base + 'studio.js';
      sc.onload = ok; sc.onerror = function () { studio = null; fail(new Error('studio.js failed to load')); };
      document.head.appendChild(sc);
    }))).then(function () { App.themeStudio(id); }, function (err) { App.toast('Couldn\'t open Theme Studio: ' + err.message, { kind: 'error' }); });
  }
  App.action('theme-studio', function () { openStudio(null); });
  /* Settings > Appearance > Layout: the stored layout (ADR-0007, layout.js) back to its defaults */
  App.action('layout-reset', function () {
    App.confirm({ title: 'Reset the layout?', text: 'The side panel and the Now Playing column go back to their widths, search to one list and Home to its first order, all sections shown. Only on this device.', confirm: 'Reset' }, function () {
      App.layout.reset();
      App.toast('Layout reset', { icon: 'undo' });
    });
  });
  App.action('theme-edit', function (el) { openStudio(el.getAttribute('data-id')); });


  var studioCss = false;                           // designs/studio.css: Your themes and Theme Studio
  function loadCss() {
    if (studioCss) return; studioCss = true;
    var s = document.createElement('style');            // the screens layer, like every screen's CSS
    s.textContent = '@import url("' + Designs.base + 'studio.css") layer(screens);';
    document.head.appendChild(s);
  }
})();
