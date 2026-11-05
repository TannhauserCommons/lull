/* screens/your-themes.js.
   Your themes, on first use (screens/appearance.js loads it when asked: a theme imported or deleted, Theme Studio's
   loop check), so a start never fetches it: App.importTheme(file), App.deleteTheme(id), and a theme's loops that draw
   more than they need to (designs/loops.js, docs/themes.md, Drawing cheaply), in words.
   App.loopCheck(css, base, opts) resolves to the hints for a theme's CSS, read beside what its loops share the page
   with: lull's components.css and the built-in design it extends, each fetched once (opts.unsure: say where the CSS
   alone can't tell whether a loop passes under glass). App.themeLoopsList(hints, opts) words them in groups, for Theme
   Studio (which adds what it finds in its live previews) and for an imported theme, which App.checkAfterImport checks
   once it is in and offers the rates. A hint, never a block: the theme works either way.
   App.hookCheck(css, slots) resolves to what the theme styles that lull moved, removed or keeps internal
   (designs/hooks.js), App.themeHooksList(hints, opts) words it; App.themeSlots(item) reads a stored theme's slot
   templates, whose own names that check leaves alone. Checked once, at an import and in Theme Studio. */
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
        App.checkAfterImport(r.body);
        App.themePictures(r.body);
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
  /* ---------- hooks lull moved, removed or keeps internal ---------- */
  App.hookCheck = function (css, slots) {
    return Promise.all([App.needScript(Designs.base + 'loops.js', 'the loop check'), App.needScript(Designs.base + 'hooks.js', 'the hook check')])
      .then(function () { return LullHooks.find(css, { slots: slots }); });
  };
  App.themeSlots = function (item) {
    return Promise.all((item.slots || []).map(function (name) {
      return M.api('/api/themes/' + encodeURIComponent(item.id) + '/slots/' + encodeURIComponent(name) + '.html').then(String, function () { return ''; });
    }));
  };
  /* opts: { lines: name each rule and its line (Theme Studio, beside the CSS), else how many rules */
  App.themeHooksList = function (hints, opts) {
    opts = opts || {};
    var short = function (sel) { return sel.length > 60 ? sel.slice(0, 57) + '…' : sel; };
    var item = function (h) {
      var w = LullHooks.words(h), n = h.rules.length, shown = h.rules.slice(0, 3);
      var where = opts.lines ? html`${shown.map(function (r, j) { return html`${j ? ', ' : ' ('}line ${r.line}: <code>${short(r.selector)}</code>`; })}${n > shown.length ? ' and ' + (n - shown.length) + ' more' : ''})`
        : ' (' + n + (n === 1 ? ' rule)' : ' rules)');
      return html`<li class="studio__warn">${icon(h.kind === 'internal' ? 'info' : 'warning', null, 'ic-16')}<span><b>${h.name}</b>${where}. ${w.what} ${w.then}</span></li>`;
    };
    var changed = hints.filter(function (h) { return h.kind !== 'internal'; }), internal = hints.filter(function (h) { return h.kind === 'internal'; });
    return html`${changed.length ? html`<h4 class="studio__sub">Changed in lull</h4><ul class="studio__warnings" role="list">${changed.map(item)}</ul>` : ''}${
      internal.length ? html`<h4 class="studio__sub">lull’s internal parts</h4><ul class="studio__warnings" role="list">${internal.map(item)}</ul>` : ''}`;
  };

  /* opts: { lines: say each loop's line, add(hint, i): an Add it for a rate } */
  var GROUPS = [['rate'], ['repaint'], ['glass', 'glass-live', 'glass-maybe'], ['status-words']];
  App.themeLoopsList = function (hints, opts) {
    opts = opts || {};
    var where = function (h) { return html`<b>${h.animation || 'A loop'}</b> on <code>${h.selector}</code>${opts.lines && h.line ? ', line ' + h.line : ''}`; };
    var item = function (h, i) {
      var ic = icon(h.kind === 'glass-maybe' ? 'info' : 'warning', null, 'ic-16');
      if (h.kind === 'rate') return html`<li class="studio__warn">${ic}<span>${where(h)}. ${LullLoops.advice(h)}</span>${opts.add ? opts.add(h, i) : ''}</li>`;
      if (h.kind === 'repaint') return html`<li class="studio__warn">${ic}<span>${where(h)} changes ${h.props.map(function (p, j) { return html`${j ? ', ' : ''}<code>${p}</code>`; })}. ${h.fix}</span></li>`;
      if (h.kind === 'glass') return html`<li class="studio__warn">${ic}<span>${where(h)}${h.itself ? ' blurs what’s behind it itself.' : html` sits inside <code>${h.glass}</code>.`}</span></li>`;
      if (h.kind === 'status-words') {
        return html`<li class="studio__warn">${ic}<span><b>“${h.words}”</b> on <code>${h.selector}</code>${opts.lines && h.line ? ', line ' + h.line : ''}, for the <code>${h.state}</code> state.${h.insert ? ''
          : ' It is drawn outside the player, where lull’s words aren’t: say what happened without a reason (“stopped”, “couldn’t play this song”), or draw it inside the player bar or the mini player.'}</span>${h.insert && opts.add ? opts.add(h, i) : ''}</li>`;
      }
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
  /* an imported theme, once it is in: its loops that could draw less (with the fixes lull can make) and what it styles
     that lull moved, removed or keeps internal, in one dialog, only when there is something to say */
  App.checkAfterImport = function (item) {
    var path = '/api/themes/' + encodeURIComponent(item.id) + '/theme.css';
    M.api(path).then(function (css) {
      return Promise.all([App.loopCheck(css, item.extends, { unsure: true }), App.themeSlots(item).then(function (slots) { return App.hookCheck(css, slots); })]).then(function (got) {
        var hints = got[0], hooks = got[1];
        var sure = hints.filter(function (h) { return h.kind !== 'glass-maybe'; });     // a "can't tell" alone bothers no one
        if (!sure.length && !hooks.length) return;
        /* loops that could draw less, and words a theme gives as reasons of its own: what it is, what changing it
           buys, and the fixes lull can make (a rate, its own words) in one go */
        var loops = {}, words = sure.filter(function (h) { return h.kind === 'status-words'; }), fixes = hints.filter(function (h) { return h.insert; });
        sure.forEach(function (h) { if (h.kind !== 'status-words') loops[h.selector + '|' + h.animation] = true; });
        var n = Object.keys(loops).length, both = n && words.length, changed = hooks.some(function (h) { return h.kind !== 'internal'; });
        var title = (both || ((n || words.length) && hooks.length)) ? 'A few things in ' + item.name + ' to look at'
          : n ? (n === 1 ? 'A loop' : n + ' loops') + ' in ' + item.name + ' could draw less'
          : words.length ? item.name + ' gives reasons of its own'
          : changed ? item.name + ' styles parts lull has changed' : item.name + ' styles lull’s internal parts';
        var buys = [n ? 'lighter on this device' : '', words.length ? 'right for every song' : ''].filter(Boolean).join(' and ');
        var rates = fixes.every(function (h) { return h.kind === 'rate'; }), lullWords = fixes.every(function (h) { return h.kind === 'status-words'; });
        var label = rates ? (fixes.length === 1 ? 'Add it' : 'Add them') : lullWords ? 'Use lull’s own words' : 'Make these changes';
        var d = App.openDialog({ title: title,
          body: html`<p class="prose">${sure.length ? item.name + ' works as it is; changing ' + (sure.length === 1 ? 'it' : 'them') + ' makes it ' + buys + '.' : item.name + ' works as it is.'}${
            hooks.length ? ' Theme Studio shows the line of each rule named here.' : ''}</p>${sure.length ? App.themeLoopsList(hints) : ''}${App.themeHooksList(hooks)}`,
          foot: fixes.length ? html`<button class="btn" data-close>Not now</button><button class="btn btn--primary" data-add-rates>${label}</button>`
            : html`<button class="btn btn--primary" data-close>OK</button>` });
        var add = $('[data-add-rates]', d.el);
        if (add) add.addEventListener('click', function () {
          d.close();
          M.api(path, { method: 'PUT', body: LullLoops.apply(css, fixes) }).then(function () {
            App.toast(rates ? (fixes.length === 1 ? 'Added its rate to ' : 'Added their rates to ') + item.name
              : lullWords ? item.name + ' shows lull’s own words now' : 'Updated ' + item.name, { kind: 'ok', icon: 'check' });
          }, function (err) { App.toast('Couldn\'t change ' + item.name + ': ' + err.message, { kind: 'error' }); });
        });
      });
    }).catch(function (err) { console.warn('lull: couldn\'t check ' + item.name, err); });
  };
})();
