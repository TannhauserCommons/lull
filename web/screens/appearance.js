/* screens/appearance.js.
   Settings > Appearance: design, theme, motion (and pausing animations while lull isn't the active window), text size, the desktop app's see-through window (LullDesktop.glass) and
   the layout's reset, all per device (App.setPref); the
   listener's own themes (import by drag and drop, export, delete) and the way into Theme Studio (designs/studio.js,
   ADR-0004, docs/themes.md); Classic mode's group, which screens/skins.js builds (App.classicSettings). Live reload
   of a stored theme runs here too.
   Builds with the shared settings controls from screens/system.js (App.settingsKit), which loads first. */
(function () {
  'use strict';
  var html = App.html, M = App.data, icon = App.icon, $ = App.$;
  var K = App.settingsKit, row = K.row, choice = K.choice, group = K.group, custom = K.custom;

  var designFocus = false;                         // the picker had focus: give it back after the re-render
  custom['device.design'] = { get: function () { return App.prefs().design; }, set: function (v) {
    designFocus = !!(document.activeElement && document.activeElement.closest('.sys-designs'));
    App.setPref('design', v);
    App.announce(App.designs[v].label + (App.designs[v].builtIn ? ' design' : ' theme'));
  } };
  custom['device.theme'] = { get: function () { return App.prefs().theme; }, set: function (v) { App.setPref('theme', v === 'system' ? null : v); } };
  custom['device.motion'] = { get: function () { return App.prefs().motion; }, set: function (v) { App.setPref('motion', v); } };
  custom['device.textSize'] = { get: function () { return App.prefs().textSize; }, set: function (v) { App.setPref('textSize', v); } };
  custom['device.pauseInactive'] = { get: function () { return App.prefs().pauseInactive; }, set: function (v) { App.setPref('pauseInactive', !!v); } };

  /* the desktop app's see-through window (app/src/desktop/glass.rs): the listener's switch, stored by the app on this
     computer, and what the window shows now. An app older than the switch has no row. desktop.js loads after this file,
     so the app is looked up when the row is first drawn. */
  var desk;                                        // LullDesktop in the app, null elsewhere (undefined: not looked yet)
  var glassOn = null;                              // the switch, read from the app once per page
  custom['desktop.glass'] = { get: function () { return glassOn; }, set: function (v) {
    var before = glassOn; glassOn = v;
    desk.setSettings({ glass: v }).then(null, function (err) {
      glassOn = before; repaintGlass();
      App.toast('Couldn\'t change that: ' + (err && err.message || err), { kind: 'error' });
    });
  } };
  function lookUpDesk() {
    var D = window.LullDesktop;
    desk = D && D.inWrapper && D.glass ? D : null;
    if (desk) desk.glass.on(function () { repaintGlass(); });
  }
  var MATERIAL = { mica: 'Mica', acrylic: 'acrylic glass', blur: 'frosted glass', transparent: 'clear glass' };
  function glassHelp() {
    var g = desk.glass.state, name = App.designs[Designs.current].label;
    if (g && g.material) return 'Your desktop shows through the window, behind ' + MATERIAL[g.material] + '.';
    switch (g && g.held) {
      case 'off': return 'Off: the window stays solid, with ' + name + '\'s own background.';
      case 'system': return 'Transparency effects are off on this computer, so the window stays solid.';
      case 'unsupported': return 'This computer can\'t draw ' + name + '\'s glass smoothly, so the window stays solid.';
      case 'restart': return 'Restart lull to see your desktop through the window.';
      default: return 'Lets a theme show your desktop through the window. ' + name + ' keeps it solid.';
    }
  }
  function glassRow() {
    if (desk === undefined) lookUpDesk();
    if (!desk) return '';
    if (glassOn === null) {
      desk.settings().then(function (d) { glassOn = d.glass == null ? false : d.glass; if (d.glass == null) desk = null; repaintGlass(); },
        function (err) { console.error('lull: desktop settings', err); });
      return html`<div id="sys-glass"></div>`;
    }
    return html`<div id="sys-glass">${K.switchRow('desktop.glass', 'See-through window', glassHelp())}</div>`;
  }
  function repaintGlass() { if (onAppearance()) K.repaint('#sys-glass', desk ? K.switchRow('desktop.glass', 'See-through window', glassHelp()) : ''); }
  /* design preview card: the theme's picture, day or night as the device shows (a theme with one mode shows that one),
     over a tiny picture of the shell (rail / topnav / dock) in its swatches, which shows while the image loads or when
     there is no picture yet */
  function designCard(name, on) {
    var d = App.designs[name], sw = d.swatches, modes = d.modes || ['day', 'night'];
    var day = picture(d, modes.indexOf('day') >= 0 ? 'day' : 'night'), night = picture(d, modes.indexOf('night') >= 0 ? 'night' : 'day');
    var shot = day && night ? ';--pv-day:url("' + day + '");--pv-night:url("' + night + '")' : '';
    return html`<button type="button" class="sys-design" role="radio" aria-checked="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-value="${name}"
      style="${'--d1:' + sw[0] + ';--d2:' + sw[1] + ';--d3:' + sw[2] + ';--d4:' + sw[3] + shot}">
      <span class="sys-design__pv${shot ? ' has-shot' : ''}" data-shell="${d.shell}" aria-hidden="true"><i class="sys-design__nav"></i><i class="sys-design__win"><b></b><b></b><b></b></i><i class="sys-design__win sys-design__win--b"><b></b><b></b></i><i class="sys-design__bar"><b></b></i><i class="sys-design__dock"></i></span>
      <span class="sys-design__name">${d.label}${icon('check', null, 'ic-16 sys-design__check')}</span>
      <span class="sys-design__desc">${d.description || (d.author ? 'By ' + d.author : 'Your theme')}</span></button>`;
  }
  /* a mode's picture, absolute (a url() in a custom property resolves against the stylesheet that uses it): the package's
     own (assets/preview-<mode>.webp: the built-ins', or an author's), else the one lull made of the theme's files now */
  function picture(d, mode) {
    if (d.preview) return new URL(Designs.packageUrl(d.id) + 'assets/preview-' + mode + '.webp', location.href).href;
    if (d.rev && d.pictures.indexOf(mode) >= 0) return new URL(M.url('/api/theme-pictures/' + encodeURIComponent(d.id) + '/' + d.rev + '-' + mode + '.webp'), location.href).href;
    return null;
  }
  /* a listener's theme with no picture of its own and none made of its files now gets one, drawn on this device in the
     background (screens/theme-pictures.js, ADR-0080): right after it is imported or saved in Theme Studio, and here for a
     change made with no lull page open (its folder edited on the hub, a new lull) or a picture that didn't get made.
     Once per theme and rev on a page; a picture that fails waits for the theme's next change or the next visit.
     opts.later: the theme just changed somewhere else, where the page that changed it is likely drawing it now, so this
     one waits and asks the hub again before drawing (screens/theme-pictures.js). */
  var asked = {};
  App.themePictures = function (m, opts) {
    if (!m || m.builtIn || m.preview || !m.rev) return;
    var have = m.pictures || [], modes = (m.modes || ['day', 'night']).filter(function (x) { return have.indexOf(x) < 0; });
    if (!modes.length || asked[m.id + ' ' + m.rev]) return;
    asked[m.id + ' ' + m.rev] = true;
    App.needScript('screens/theme-pictures.js', 'theme pictures').then(function () {
      App.makeThemePictures({ id: m.id, rev: m.rev, label: m.label || m.name }, modes, opts);
    }, function (err) { console.warn('lull: theme pictures', err); });
  };
  App.on('themePictures', function () { Designs.refresh().then(function () { if (onAppearance()) App.refresh(); }, function () {}); });

  /* the listener's themes: edit in Theme Studio, export, delete; import by drop or file */
  function myThemes() {
    var mine = App.designOrder.filter(function (n) { return !App.designs[n].builtIn; });
    return html`<div class="sys-row sys-row--stack sys-themes" data-drop-theme>
        <div class="sys-row__text"><span class="sys-row__label">Your themes</span><span class="sys-row__help">Stored on your hub, so every device can use them. Drop a .lulltheme file here to add one, or make one in Theme Studio.</span></div>
        <div class="sys-row__ctl">${mine.length ? html`<ul class="sys-themes__list" role="list">${mine.map(function (n) {
          var d = App.designs[n];
          return html`<li class="sys-themes__item"><span class="sys-themes__name">${d.label}</span><span class="sys-themes__meta">${d.extends ? 'on ' + App.designs[d.extends].label : 'from scratch'}${d.author ? ', by ' + d.author : ''}</span>${d.description ? html`<span class="sys-themes__desc">${d.description}</span>` : ''}
            <span class="sys-themes__acts"><button class="btn btn--sm btn--quiet" data-act="theme-edit" data-id="${n}">${icon('edit', null, 'ic-16')}Edit</button><a class="btn btn--sm btn--quiet" href="${'api/themes/' + n + '.lulltheme'}" download="${n + '.lulltheme'}">${icon('export', null, 'ic-16')}Export</a><button class="btn-icon btn-icon--sm btn-icon--bare" data-act="theme-delete" data-id="${n}" aria-label="${'Delete ' + d.label}" data-tip="Delete">${icon('trash', null, 'ic-16')}</button></span></li>`;
        })}</ul>` : ''}
          <div class="cluster"><button class="btn btn--primary btn--sm" data-act="theme-studio">${icon('sparkle', null, 'ic-16')}Open Theme Studio</button>
          <label class="btn btn--sm">${icon('import', null, 'ic-16')}Import a theme<input class="vh" type="file" accept=".lulltheme,.zip" data-theme-file></label></div></div></div>`;
  }
  function secAppearance() {
    var p = App.prefs();
    loadCss();
    if (!listed) { listed = true; Designs.refresh().then(function () { fresh = true; if (onAppearance()) App.refresh(); }, function () {}); }
    else if (fresh) App.designOrder.forEach(function (n) { App.themePictures(App.designs[n], { later: !!changed[n] }); });
    return html`<div class="sys-row sys-row--stack"><div class="sys-row__text"><span class="sys-row__label" id="sys-design-l">Design</span><span class="sys-row__help">The whole look: colours, type, window chrome and layout. Applies at once on this device.</span></div>
        <div class="sys-row__ctl"><div class="sys-designs" role="radiogroup" aria-labelledby="sys-design-l" data-key="device.design">${App.designOrder.map(function (n) { return designCard(n, n === p.design); })}</div></div></div>
      ${row('Theme', 'Match system follows your device. T switches day and night anywhere.', choice('device.theme', 'Theme', [['system', 'Match system'], ['day', 'Day'], ['night', 'Night']]))}
      ${glassRow()}
      ${row('Motion', 'Reduced swaps slides and loops for quick fades. Full ignores the system setting.', choice('device.motion', 'Motion', [['system', 'Match system'], ['reduce', 'Reduced'], ['full', 'Full']]))}
      ${window.LullNative ? '' : K.switchRow('device.pauseInactive', 'Pause animations when ' + App.product + ' isn\'t the active window', 'Twinkles, the sky and other decoration hold still while you use another window, even one beside it. Spinners and progress keep moving. This device only.')}
      ${row('Text size', 'Scales all text and spacing on this device.', choice('device.textSize', 'Text size', [[90, 'Smaller'], [100, 'Default'], [112, 'Larger'], [125, 'Largest']], 'sys-seg--4'))}
      ${group('layout')}
      ${row('Reset layout', 'Puts the side panel, the Now Playing column, and side-by-side search back as they came. This device only.', html`<button class="btn btn--sm" data-act="layout-reset">${icon('undo', null, 'ic-16')}Reset layout</button>`)}
      ${group('themes')}
      ${myThemes()}
      ${classicGroup()}`;
  }
  /* Classic mode's group comes with Classic mode's code (skins.js), which a browser loads on first use */
  function classicGroup() {
    if (App.classicSettings) return App.classicSettings();
    App.needClassic().then(function () { if (onAppearance()) App.refresh(); }, function () {});
    return App.ui.skeleton('card', 1);
  }
  var listed = false;                              // GET /api/themes once per page; theme.changed keeps it current
  var fresh = false;                               // the list is the hub's, so each theme's rev is its files' now
  var changed = {};                                // themes this page heard change (theme.changed)
  function onAppearance() { return /^\/settings\/appearance/.test(App.current.path); }

  App.settingsSections.appearance = secAppearance;
  App.on('design', function (name) {
    if (!designFocus) return; designFocus = false;
    var b = $('.sys-design[data-value="' + name + '"]'); if (b) b.focus();
  });

  /* ---------- live reload: a stored theme's files changed on the hub ---------- */
  App.on('server:theme.changed', function (e) {
    var id = e && e.id;
    if (id) changed[id] = true;
    Designs.refresh().then(function () {
      if (id === Designs.current && !App.designs[id]) { App.setDesign(Designs.fallback); return; }   // deleted while in use
      return Designs.reload(id).then(function (inUse) { if (inUse) App.setDesign(Designs.current, true); });
    }).then(function () { if (onAppearance()) App.refresh(); }).catch(function (err) { console.error('Theme reload failed', err); });
  });

  /* ---------- import, delete, and the loop check: on first use (screens/your-themes.js) ----------
     Importing a theme, deleting one and the check of a theme's loops (designs/loops.js, docs/themes.md, Drawing
     cheaply) load the first time they are asked for, so a start never fetches them. App.themeLoops(css, base, opts)
     resolves to a theme's hints once it is in, for Theme Studio; App.themeLoopsList comes with it */
  function needThemes() { return App.needScript('screens/your-themes.js', 'Your themes'); }
  function importFile(file) { needThemes().then(function () { App.importTheme(file); }, function (err) { App.toast('Couldn\'t import ' + file.name + ': ' + err.message, { kind: 'error' }); }); }
  App.themeLoops = function (css, base, opts) { return needThemes().then(function () { return App.loopCheck(css, base, opts); }); };
  /* what a theme's CSS styles that lull moved, removed or keeps internal, beside its stored slots (item: its D.list entry,
     or null for a theme not saved yet); App.themeHooksList comes with it */
  App.themeHooks = function (css, item) {
    return needThemes().then(function () { return (item ? App.themeSlots(item) : Promise.resolve([])).then(function (slots) { return App.hookCheck(css, slots); }); });
  };
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
  App.action('theme-delete', function (el) { needThemes().then(function () { App.deleteTheme(el.getAttribute('data-id')); }, function (err) { App.toast(err.message, { kind: 'error' }); }); });
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
