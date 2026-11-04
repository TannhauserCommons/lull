/* screens/your-themes.js.
   Your themes, on first use (screens/appearance.js loads it when asked: a theme imported or deleted, Theme Studio's
   loop check), so a start never fetches it: App.importTheme(file), App.deleteTheme(id), and a theme's loops that draw
   more than they need to (designs/loops.js, docs/themes.md, Drawing cheaply), in words.
   App.loopCheck(css, base, opts) resolves to the hints for a theme's CSS, read beside what its loops share the page
   with: lull's components.css and the built-in design it extends, each fetched once (opts.unsure: say where the CSS
   alone can't tell whether a loop passes under glass). App.themeLoopsList(hints, opts) words them in groups, for Theme
   Studio (which adds what it finds in its live previews) and for an imported theme, which App.loopsAfterImport checks
   once it is in and offers the rates. A hint, never a block: the theme works either way. */
(function () {
  'use strict';
  var html = App.html, M = App.data, icon = App.icon, $ = App.$;
  function onAppearance() { return /^\/settings\/appearance/.test(App.current.path); }

  /* ---------- import ---------- */
  App.importTheme = function importFile(file, replace) {
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
        App.loopsAfterImport(r.body);
        return Designs.refresh().then(function () { if (onAppearance()) App.refresh(); });
      })
      .catch(function (err) { App.toast('Couldn\'t import ' + file.name + ': ' + err.message, { kind: 'error' }); });
  };

  /* ---------- delete ---------- */
  App.deleteTheme = function (id) {
    var d = App.designs[id];
    App.confirm({ title: 'Delete ' + d.label + '?', text: 'It goes from your hub and from every device using it (they switch to ' + App.designs[Designs.fallback].label + '). Export it first to keep a copy.', confirm: 'Delete', danger: true }, function () {
      if (Designs.current === id) App.setDesign(Designs.fallback);     // first, so nothing asks for its files once they're gone
      M.api('/api/themes/' + encodeURIComponent(id), { method: 'DELETE' }).then(function () {
        App.toast('Deleted ' + d.label, { icon: 'trash' });
        return Designs.refresh().then(function () { if (onAppearance()) App.refresh(); });
      }).catch(function (err) { App.toast('Couldn\'t delete ' + d.label + ': ' + err.message, { kind: 'error' }); });
    });
  };


  var beside = {};
  function besideText(url) {
    return beside[url] || (beside[url] = fetch(url).then(function (r) {
      if (!r.ok) throw new Error(url + ' answered ' + r.status);
      return r.text();
    }).catch(function (err) { delete beside[url]; throw err; }));
  }
  App.loopCheck = function (css, base, opts) {
    if (Designs.builtIn.indexOf(base) < 0) base = null;
    var also = [besideText(Designs.base + '../components.css')].concat(base ? [besideText(Designs.base + base + '/theme.css')] : []);
    return Promise.all([App.needScript(Designs.base + 'loops.js', 'the loop check')].concat(also)).then(function (got) {
      return LullLoops.find(css, { also: got.slice(1), base: base, unsure: !!(opts && opts.unsure) });
    });
  };
  /* opts: { lines: say each loop's line, add(hint, i): an Add it for a rate } */
  var GROUPS = [['rate'], ['repaint'], ['glass', 'glass-live', 'glass-maybe']];
  App.themeLoopsList = function (hints, opts) {
    opts = opts || {};
    var where = function (h) { return html`<b>${h.animation || 'A loop'}</b> on <code>${h.selector}</code>${opts.lines && h.line ? ', line ' + h.line : ''}`; };
    var item = function (h, i) {
      var ic = icon(h.kind === 'glass-maybe' ? 'info' : 'warning', null, 'ic-16');
      if (h.kind === 'rate') return html`<li class="studio__warn">${ic}<span>${where(h)}. ${LullLoops.advice(h)}</span>${opts.add ? opts.add(h, i) : ''}</li>`;
      if (h.kind === 'repaint') return html`<li class="studio__warn">${ic}<span>${where(h)} changes ${h.props.map(function (p, j) { return html`${j ? ', ' : ''}<code>${p}</code>`; })}. ${h.fix}</span></li>`;
      if (h.kind === 'glass') return html`<li class="studio__warn">${ic}<span>${where(h)}${h.itself ? ' blurs what’s behind it itself.' : html` sits inside <code>${h.glass}</code>.`}</span></li>`;
      if (h.kind === 'glass-live') return html`<li class="studio__warn">${ic}<span><b>${h.animation}</b> on <code>${h.target}</code> ${h.exact ? 'passes' : 'may pass'} under <code>${h.glass}</code> on ${h.screen}, in the ${h.modes.join(' and ')} preview${h.modes.length > 1 ? 's' : ''}.</span></li>`;
      return html`<li class="studio__warn studio__warn--maybe">${ic}<span>lull can’t tell from the CSS alone whether ${h.loops.map(function (l, j) { return html`${j ? (j === h.loops.length - 1 ? ' or ' : ', ') : ''}<b>${l.animation || 'a loop'}</b>`; })} pass${h.loops.length === 1 ? 'es' : ''} under the glass of ${h.glass.slice(0, 3).map(function (g, j) { return html`${j ? ', ' : ''}<code>${g}</code>`; })}${h.glass.length > 3 ? ' and more' : ''}. Theme Studio checks it on its live preview.</span></li>`;
    };
    return html`${GROUPS.map(function (kinds) {
      var group = hints.map(function (h, i) { return { h: h, i: i }; }).filter(function (x) { return kinds.indexOf(x.h.kind) >= 0; });
      var sure = group.filter(function (x) { return x.h.kind !== 'glass-maybe'; });
      if (!group.length) return '';
      var said = LullLoops.say(kinds[0], sure.length || 1);
      return html`<h4 class="studio__sub">${said.title}</h4>${sure.length ? html`<p class="field__help">${said.lead}</p>` : ''}
        <ul class="studio__warnings" role="list">${group.map(function (x) { return item(x.h, x.i); })}</ul>`;
    })}`;
  };
  App.loopsAfterImport = function (item) {
    var path = '/api/themes/' + encodeURIComponent(item.id) + '/theme.css';
    M.api(path).then(function (css) {
      return App.loopCheck(css, item.extends, { unsure: true }).then(function (hints) {
        var sure = hints.filter(function (h) { return h.kind !== 'glass-maybe'; });
        if (!sure.length) return;                                // only a "can't tell": nothing to bother anyone with
        var loops = {}, rates = hints.filter(function (h) { return h.kind === 'rate'; });
        sure.forEach(function (h) { loops[h.selector + '|' + h.animation] = true; });
        var n = Object.keys(loops).length;
        var d = App.openDialog({ title: (n === 1 ? 'A loop' : n + ' loops') + ' in ' + item.name + ' could draw less',
          body: html`<p class="prose">${item.name} works as it is; changing ${n === 1 ? 'it' : 'them'} makes it lighter on this device.</p>${App.themeLoopsList(hints)}`,
          foot: rates.length ? html`<button class="btn" data-close>Not now</button><button class="btn btn--primary" data-add-rates>${rates.length === 1 ? 'Add it' : 'Add them'}</button>`
            : html`<button class="btn btn--primary" data-close>OK</button>` });
        var add = $('[data-add-rates]', d.el);
        if (add) add.addEventListener('click', function () {
          d.close();
          M.api(path, { method: 'PUT', body: LullLoops.apply(css, rates) }).then(function () {
            App.toast((rates.length === 1 ? 'Added its rate to ' : 'Added their rates to ') + item.name, { kind: 'ok', icon: 'check' });
          }, function (err) { App.toast('Couldn\'t add the rates: ' + err.message, { kind: 'error' }); });
        });
      });
    }).catch(function (err) { console.warn('lull: couldn\'t check the loops in ' + item.name, err); });
  };
})();
