/* designs/studio.js: Theme Studio (ADR-0004, docs/themes.md), loaded by screens/appearance.js the first time it opens.
   App.themeStudio(id) opens it on a stored theme, or with id null on a new one. Styles: designs/studio.css. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$, esc = App.esc;
  /* Theme Studio: a theme is theme.json + theme.css. The CSS text is the one source of truth: the token pickers
     rewrite a marked block at its top; the editor edits all of it. Two live previews (the real app in frames, day and
     night) show every change at once; contrast warnings read the colours the previews actually compute. */
  /* [category, kind, tokens]: kind 'mode' = a colour per day and night, 'color' = one colour, 'font', or a length
     [name, min, max, unit]. Names are tokens.css's (without --). */
  var TOKENS = [
    ['Page and panels', 'mode', ['sky', 'sky-deep', 'panel', 'panel-2', 'tray', 'tray-edge']],
    ['Text and accents', 'mode', ['fg-sky', 'fg-sky-2', 'fg-panel', 'fg-panel-2', 'link', 'accent-ui', 'heart', 'focus', 'edge']],
    ['Player', 'mode', ['seek-played', 'seek-buffered', 'seek-line', 'play-ring', 'sparkle', 'speck']],
    ['Cards and screens', 'color', ['cream', 'cream-2', 'cream-3', 'ink', 'ink-2', 'link-on-cream', 'screen', 'line', 'stack-1', 'stack-2']],
    ['Type', 'font', ['f-ui', 'f-px']],
    ['Space', 'length', [['gutter', 8, 64, 'px'], ['card-pad', 8, 48, 'px']]],
    ['Radius', 'length', [['r-chip', 0, 24, 'px'], ['r-thumb', 0, 24, 'px'], ['r-art', 0, 40, 'px'], ['r-input', 0, 32, 'px'], ['r-panel', 0, 40, 'px'], ['r-card', 0, 48, 'px']]],
    ['Stroke', 'length', [['stroke', 0, 6, 'px'], ['stroke-thin', 0, 4, 'px']]],
    ['Stacks (elevation)', 'length', [['stack-off-1', 0, 20, 'px'], ['stack-off-2', 0, 40, 'px']]],
    ['Motion', 'length', [['dur-1', 0, 600, 'ms'], ['dur-2', 0, 800, 'ms'], ['dur-3', 0, 1000, 'ms'], ['dur-4', 0, 1600, 'ms']]],
    ['Layout', 'length', [['rail', 160, 360, 'px'], ['ctx', 240, 480, 'px'], ['bar-h', 56, 140, 'px'], ['tag-h', 16, 32, 'px']]]
  ];
  /* [fg, bg, what, minimum ratio]: text pairs need 4.5, rings and glyphs 3 */
  var PAIRS = [['fg-sky', 'sky', 'Text on the page', 4.5], ['fg-sky-2', 'sky', 'Quiet text on the page', 4.5],
    ['fg-panel', 'panel', 'Text on panels', 4.5], ['fg-panel-2', 'panel', 'Quiet text on panels', 4.5], ['link', 'panel', 'Links on panels', 4.5],
    ['ink', 'cream', 'Text on cards', 4.5], ['ink-2', 'cream', 'Quiet text on cards', 4.5], ['link-on-cream', 'cream', 'Links on cards', 4.5],
    ['lilac', 'screen', 'Device screen text', 4.5], ['focus', 'sky', 'Focus ring on the page', 3], ['heart', 'panel', 'Hearts on panels', 3]];
  var ROUTES = [['home', 'Home'], ['now', 'Now Playing'], ['library', 'Library'], ['playlists', 'Playlists'], ['stats', 'Stats'], ['settings/appearance', 'Settings']];
  var BEGIN = '/* tokens: written by Theme Studio (edit them there, or here) */', END = '/* end of tokens */';

  var S = null;                                    // the open studio: { draft, stored, frames, ... }
  function openStudio(id) {
    var stored = id && App.designs[id] && !App.designs[id].builtIn ? App.designs[id] : null;
    var base = stored ? stored.extends : (App.designs[Designs.current].builtIn ? Designs.current : App.designs[Designs.current].extends) || Designs.fallback;
    S = { id: stored ? id : null, dirty: false, tab: 'tokens', route: 'home', phone: false, frames: [],
      draft: { name: stored ? stored.label : 'My ' + App.designs[base].label, extends: base, shell: stored ? stored.shell : App.designs[base].shell,
        modes: stored ? stored.modes.slice() : ['day', 'night'], description: stored ? stored.description : '', author: stored ? stored.author : '', css: '',
        /* what Studio doesn't edit stays as the theme has it */
        coverVariety: !!(stored && stored.coverVariety), window: stored ? stored.window : null } };
    var sheet = App.openSheet({ title: 'Theme Studio', wide: true, className: 'studio-sheet', body: studioBody(), foot: studioFoot(),
      onMount: function (el) {
        el.style.width = 'min(1600px, 100%)'; el.style.maxWidth = 'none'; el.style.height = 'min(940px, calc(100dvh - 24px))';
        S.el = el; mountStudio();
      },
      onClose: function () { S = null; } });
    S.close = sheet.close;
    if (stored) {
      M.api('/api/themes/' + encodeURIComponent(id) + '/theme.css').then(function (css) { setCss(css, false); })
        .catch(function (err) { App.toast('Couldn\'t read ' + stored.label + ': ' + err.message, { kind: 'error' }); });
    }
  }

  function studioBody() {
    var d = S.draft;
    return html`<div class="studio">
      <div class="studio__side">
        <div class="studio__meta">
          <label class="field"><span class="field__label">Name</span><input class="input" data-studio="name" value="${d.name}" maxlength="60" autocomplete="off"></label>
          <label class="field"><span class="field__label">Start from</span><select class="input" data-studio="extends">${Designs.builtIn.map(function (n) {
            return html`<option value="${n}" ${n === d.extends ? raw('selected') : ''}>${App.designs[n].label}</option>`;
          })}<option value="" ${!d.extends ? raw('selected') : ''}>Nothing (base tokens only)</option></select></label>
          <div class="field"><span class="field__label" id="studio-shell-l">Layout</span>${seg('shell', 'studio-shell-l', [['rail', 'Rail'], ['topnav', 'Top bar'], ['dock', 'Dock']], d.shell)}</div>
          <div class="field"><span class="field__label" id="studio-modes-l">Modes</span>${seg('modes', 'studio-modes-l', [['day,night', 'Day and night'], ['day', 'Day only'], ['night', 'Night only']], d.modes.join(','))}</div>
        </div>
        <div class="seg studio__tabs" role="tablist" aria-label="Edit" style="--n:2;--i:0"><span class="seg__ind" aria-hidden="true"></span>
          <button class="seg__btn" role="tab" id="studio-t-tokens" aria-controls="studio-p-tokens" aria-selected="true" data-studio-tab="tokens">Tokens</button>
          <button class="seg__btn" role="tab" id="studio-t-css" aria-controls="studio-p-css" aria-selected="false" tabindex="-1" data-studio-tab="css">CSS</button></div>
        <div class="studio__panel" id="studio-p-tokens" role="tabpanel" aria-labelledby="studio-t-tokens"><div class="studio__tokens" data-studio-tokens>${ui.skeleton('text', 4)}</div></div>
        <div class="studio__panel" id="studio-p-css" role="tabpanel" aria-labelledby="studio-t-css" hidden>
          <div class="studio__editor"><pre class="studio__lines" aria-hidden="true" data-studio-lines>1</pre><textarea class="studio__css" data-studio="css" spellcheck="false" autocomplete="off" autocapitalize="off" wrap="off" aria-label="theme.css" aria-describedby="studio-css-help"></textarea></div>
          <p class="field__help" id="studio-css-help">Any CSS, loaded after the design you start from, so plain selectors win. Tab indents, Ctrl+S saves. Hooks and examples: docs/themes.md.</p>
        </div>
        <section class="studio__check" aria-labelledby="studio-check-h"><h3 class="studio__h" id="studio-check-h">Contrast</h3><ul class="studio__warnings" role="list" data-studio-warnings><li>${'Checking...'}</li></ul></section>
        <section class="studio__check" aria-labelledby="studio-loops-h"><h3 class="studio__h" id="studio-loops-h">Loops</h3><div data-studio-loops aria-live="polite"><ul class="studio__warnings" role="list"><li>${'Checking...'}</li></ul></div></section>
        <section class="studio__check" aria-labelledby="studio-words-h"><h3 class="studio__h" id="studio-words-h">Player words</h3><div data-studio-words aria-live="polite"><ul class="studio__warnings" role="list"><li>${'Checking...'}</li></ul></div></section>
      </div>
      <div class="studio__stage">
        <div class="studio__bar">
          <label class="field studio__route"><span class="vh">Screen</span><select class="input" data-studio="route">${ROUTES.map(function (r) { return html`<option value="${r[0]}">${r[1]}</option>`; })}</select></label>
          ${seg('size', null, [['desktop', 'Desktop'], ['phone', 'Phone']], 'desktop', 'Preview size')}
        </div>
        <div class="studio__frames" data-studio-frames>${['day', 'night'].map(function (m) {
          return html`<figure class="studio__frame" data-mode="${m}"><figcaption class="studio__cap">${m}</figcaption><div class="studio__view"><iframe title="${'Preview, ' + m}" data-studio-frame="${m}" tabindex="-1" loading="lazy"></iframe></div></figure>`;
        })}</div>
      </div>
    </div>`;
  }
  function seg(key, labelledBy, options, cur, label) {
    var i = Math.max(0, options.map(function (o) { return o[0]; }).indexOf(cur));
    return html`<div class="seg" role="radiogroup" ${labelledBy ? raw('aria-labelledby="' + labelledBy + '"') : raw('aria-label="' + esc(label) + '"')} data-studio-seg="${key}" style="--n:${options.length};--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${options.map(function (o, j) {
      return html`<button type="button" class="seg__btn" role="radio" aria-checked="${j === i ? 'true' : 'false'}" tabindex="${j === i ? '0' : '-1'}" data-value="${o[0]}">${o[1]}</button>`;
    })}</div>`;
  }
  function studioFoot() {
    return html`<span class="studio__state" data-studio-state aria-live="polite"></span>
      <button class="btn btn--quiet" data-studio-act="export">${icon('export', null, 'ic-16')}Export</button>
      <button class="btn" data-studio-act="use">Use this theme</button>
      <button class="btn btn--primary" data-studio-act="save">${icon('save', null, 'ic-16')}Save to hub</button>`;
  }

  /* ---------- wiring ---------- */
  function mountStudio() {
    var el = S.el;
    el.addEventListener('input', onInput);
    el.addEventListener('change', onInput);
    el.addEventListener('click', onClick);
    el.addEventListener('keydown', onKey);
    var ta = $('[data-studio="css"]', el);
    ta.addEventListener('scroll', function () { $('[data-studio-lines]', el).scrollTop = ta.scrollTop; });
    $$('[data-studio-frame]', el).forEach(function (f) {
      S.frames.push(f);
      f.addEventListener('load', function () { prepareFrame(f); });
    });
    loadFrames();
    if (!S.id) setCss('', false);
    state();
  }
  function frameUrl(mode) {
    var d = S.draft, design = S.id || d.extends || Designs.fallback;
    return 'index.html#/' + S.route + '?design=' + encodeURIComponent(design) + '&theme=' + mode;
  }
  function loadFrames() {
    S.frames.forEach(function (f) {
      f.parentNode.parentNode.classList.toggle('is-phone', S.phone);
      f.src = frameUrl(f.getAttribute('data-studio-frame'));
    });
  }
  /* a preview frame is the real app; the draft replaces the stored theme's CSS there (layer user) */
  function prepareFrame(f) {
    var w = f.contentWindow, doc = f.contentDocument; if (!w || !doc || !w.Designs) return;
    var stored = doc.getElementById('user-css'); if (stored) stored.remove();
    paintFrame(f);
    w.Designs.ready(function () { setTimeout(function () { if (S) { readTokens(); checkContrast(); } }, 600); lookLive(); });
  }
  function draftCss() {
    var css = S.draft.css;
    if (S.id) css = css.replace(/url\(\s*(['"]?)(assets\/)/g, 'url($1' + Designs.packageUrl(S.id) + '$2');   // the preview isn't next to the package
    return '@layer user {\n' + css + '\n}';
  }
  function paintFrame(f) {
    var doc = f.contentDocument; if (!doc || !doc.head) return;
    var s = doc.getElementById('studio-draft');
    if (!s) { s = doc.createElement('style'); s.id = 'studio-draft'; doc.head.appendChild(s); }
    s.textContent = draftCss();
    var root = doc.documentElement, d = S.draft, modes = d.modes;
    root.setAttribute('data-shell', d.shell);
    if (modes.length === 1) root.setAttribute('data-theme', modes[0]);
  }
  var paintTimer = null;
  function repaint() {
    clearTimeout(paintTimer);
    paintTimer = setTimeout(function () {
      if (!S) return;
      S.frames.forEach(paintFrame);
      checkLoops();
      setTimeout(function () { if (S) { readTokens(); checkContrast(); } }, 120);
    }, 160);
  }
  function setCss(css, dirty) {
    S.draft.css = css;
    var ta = $('[data-studio="css"]', S.el);
    if (ta && ta.value !== css) ta.value = css;
    lineNumbers();
    if (dirty) { S.dirty = true; state(); }
    repaint();
  }
  function lineNumbers() {
    var n = (S.draft.css.match(/\n/g) || []).length + 1, out = '';
    for (var i = 1; i <= n; i++) out += i + '\n';
    $('[data-studio-lines]', S.el).textContent = out;
  }
  function state(text) {
    var st = $('[data-studio-state]', S.el); if (!st) return;
    st.textContent = text || (S.dirty ? 'Unsaved changes' : S.id ? 'Saved on your hub' : 'Not saved yet');
  }

  function onInput(e) {
    var t = e.target, k = t.getAttribute('data-studio'), tok = t.getAttribute('data-token');
    if (k === 'name') { S.draft.name = t.value; S.dirty = true; state(); }
    else if (k === 'extends' && e.type === 'change') { S.draft.extends = t.value || null; if (t.value) S.draft.shell = App.designs[t.value].shell; S.dirty = true; showSeg('shell', S.draft.shell); loadFrames(); state(); }
    else if (k === 'route' && e.type === 'change') { S.route = t.value; S.frames.forEach(function (f) { if (f.contentWindow) f.contentWindow.location.hash = '#/' + S.route; }); lookLive(); }
    else if (k === 'css') { S.draft.css = t.value; lineNumbers(); S.dirty = true; state(); repaint(); }
    else if (tok) {
      var mode = t.getAttribute('data-mode'), unit = t.getAttribute('data-unit');
      var value = unit ? t.value + unit : t.value;
      if (unit) {
        var out = t.parentNode.querySelector('output'); if (out) out.textContent = value;
        t.style.setProperty('--fill', (t.value - t.min) / (t.max - t.min) * 100 + '%');
      }
      if (e.type === 'input' && t.type === 'text') return;                // fonts: on change, not every key
      setToken(tok, mode, value);
    }
  }
  function onClick(e) {
    var b = e.target.closest('button'); if (!b) return;
    var segEl = b.closest('[data-studio-seg]');
    if (segEl && b.getAttribute('role') === 'radio') {
      var key = segEl.getAttribute('data-studio-seg'), v = b.getAttribute('data-value');
      showSeg(key, v);
      if (key === 'shell') { S.draft.shell = v; S.dirty = true; repaint(); }
      if (key === 'modes') { S.draft.modes = v.split(','); S.dirty = true; repaint(); }
      if (key === 'size') { S.phone = v === 'phone'; S.frames.forEach(function (f) { f.parentNode.parentNode.classList.toggle('is-phone', S.phone); }); }
      state(); return;
    }
    var tab = b.getAttribute('data-studio-tab');
    if (tab) { showTab(tab); return; }
    var loop = b.getAttribute('data-studio-loop');
    if (loop) { addRate(+loop); return; }
    var reset = b.getAttribute('data-token-reset');
    if (reset) { setToken(reset, b.getAttribute('data-mode'), null); return; }
    var act = b.getAttribute('data-studio-act');
    if (act === 'save') save();
    if (act === 'use') save().then(function (id) { if (id) App.setPref('design', id); });
    if (act === 'export') save().then(function (id) { if (id) location.href = M.url('/api/themes/' + id + '.lulltheme'); });
  }
  function showSeg(key, v) {
    var g = $('[data-studio-seg="' + key + '"]', S.el); if (!g) return;
    $$('[role="radio"]', g).forEach(function (r, j) {
      var on = r.getAttribute('data-value') === v;
      r.setAttribute('aria-checked', on ? 'true' : 'false'); r.tabIndex = on ? 0 : -1;
      if (on) g.style.setProperty('--i', j);
    });
  }
  function showTab(tab) {
    S.tab = tab;
    $$('[data-studio-tab]', S.el).forEach(function (t, i) {
      var on = t.getAttribute('data-studio-tab') === tab;
      t.setAttribute('aria-selected', on ? 'true' : 'false'); t.tabIndex = on ? 0 : -1;
      if (on) t.parentNode.style.setProperty('--i', i);
    });
    $('#studio-p-tokens', S.el).hidden = tab !== 'tokens';
    $('#studio-p-css', S.el).hidden = tab !== 'css';
  }
  /* the editor: Tab / Shift+Tab indent, Enter keeps the indent, Ctrl+S saves; elsewhere arrows move along a segment */
  function onKey(e) {
    var t = e.target;
    if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); save(); return; }
    if (t.getAttribute('data-studio') !== 'css') return;
    var v = t.value, a = t.selectionStart, z = t.selectionEnd;
    if (e.key === 'Tab') {
      e.preventDefault();
      var start = v.lastIndexOf('\n', a - 1) + 1, block = v.slice(start, z);
      var next = e.shiftKey ? block.replace(/^ {1,2}/gm, '') : (a === z ? null : block.replace(/^/gm, '  '));
      if (next === null) { edit(t, a, z, '  ', a + 2); return; }
      edit(t, start, z, next, null, start, start + next.length);
    } else if (e.key === 'Enter' && !e.shiftKey) {
      var line = v.slice(v.lastIndexOf('\n', a - 1) + 1, a), indent = /^\s*/.exec(line)[0] + (/\{\s*$/.test(line) ? '  ' : '');
      e.preventDefault(); edit(t, a, z, '\n' + indent, a + 1 + indent.length);
    }
  }
  function edit(t, from, to, text, caret, selA, selB) {
    t.setRangeText(text, from, to, 'end');
    if (caret != null) t.setSelectionRange(caret, caret); else t.setSelectionRange(selA, selB);
    t.dispatchEvent(new Event('input', { bubbles: true }));
  }

  /* ---------- tokens: the marked block at the top of theme.css ---------- */
  function tokenBlock(css) {
    var a = css.indexOf(BEGIN), b = css.indexOf(END);
    return a >= 0 && b > a ? { from: a, to: b + END.length, text: css.slice(a, b) } : null;
  }
  /* { all: {name: value}, day: {...}, night: {...} } from the block */
  function parseTokens(css) {
    var blk = tokenBlock(css), out = { all: {}, day: {}, night: {} };
    if (!blk) return out;
    var re = /(:root(?:\[data-theme="(day|night)"\])?)\s*\{([^}]*)\}/g, m;
    while ((m = re.exec(blk.text))) {
      var into = out[m[2] || 'all'], decl = /--([a-z0-9-]+)\s*:\s*([^;]+);/g, d;
      while ((d = decl.exec(m[3]))) into[d[1]] = d[2].trim();
    }
    return out;
  }
  function writeTokens(css, t) {
    function rules(sel, map) {
      var keys = Object.keys(map); if (!keys.length) return '';
      return sel + ' {\n' + keys.map(function (k) { return '  --' + k + ': ' + map[k] + ';'; }).join('\n') + '\n}\n';
    }
    var body = rules(':root', t.all) + rules(':root[data-theme="day"]', t.day) + rules(':root[data-theme="night"]', t.night);
    var blk = tokenBlock(css), block = body ? BEGIN + '\n' + body + END : '';
    if (blk) return css.slice(0, blk.from) + block + css.slice(blk.to).replace(block ? /^/ : /^\n+/, '');
    return block ? block + '\n\n' + css : css;
  }
  function setToken(name, mode, value) {
    var t = parseTokens(S.draft.css), into = t[mode || 'all'];
    if (value == null) delete into[name]; else into[name] = value;
    setCss(writeTokens(S.draft.css, t), true);
  }
  /* the value each token shows: the draft's own, else what the preview computes for that mode */
  function readTokens() {
    var host = $('[data-studio-tokens]', S.el); if (!host) return;
    var own = parseTokens(S.draft.css), day = frameVars('day'), night = frameVars('night');
    if (!day && !night) return;
    var focus = document.activeElement && host.contains(document.activeElement) ? document.activeElement.getAttribute('data-token') + '|' + document.activeElement.getAttribute('data-mode') : null;
    if (focus && S.el.querySelector('[data-token]:focus[type="range"]')) return;     // never re-render under a dragging slider
    host.innerHTML = String(html`${TOKENS.map(function (cat) {
      return html`<section class="studio__cat" aria-label="${cat[0]}"><h3 class="studio__h">${cat[0]}</h3>${cat[2].map(function (tk) {
        return tokenRow(cat[1], tk, own, day, night);
      })}</section>`;
    })}`);
    if (focus) { var f = host.querySelector('[data-token="' + focus.split('|')[0] + '"][data-mode="' + focus.split('|')[1] + '"]'); if (f) f.focus(); }
  }
  function tokenRow(kind, tk, own, day, night) {
    var name = kind === 'length' ? tk[0] : tk;
    var mine = kind === 'mode' ? own.day[name] != null || own.night[name] != null : own.all[name] != null;
    var reset = mine ? html`<button type="button" class="btn-icon btn-icon--sm btn-icon--bare" data-token-reset="${name}" ${kind === 'mode' ? raw('data-mode="day"') : ''} aria-label="${'Reset --' + name}" data-tip="Back to the design's value">${icon('undo', null, 'ic-16')}</button>` : '';
    var label = html`<span class="studio__tok"><code>--${name}</code>${reset}</span>`;
    if (kind === 'mode') {
      return html`<div class="studio__row">${label}<span class="studio__pair">${['day', 'night'].map(function (m) {
        var v = own[m][name] || hex((m === 'day' ? day : night) || day || night, name);
        return html`<input type="color" class="studio__swatch" value="${v}" data-token="${name}" data-mode="${m}" aria-label="${'--' + name + ', ' + m}" data-tip="${m}">`;
      })}</span></div>`;
    }
    if (kind === 'color') {
      return html`<div class="studio__row">${label}<input type="color" class="studio__swatch" value="${own.all[name] || hex(day || night, name)}" data-token="${name}" data-mode="all" aria-label="${'--' + name}"></div>`;
    }
    if (kind === 'font') {
      return html`<div class="studio__row studio__row--wide">${label}<input class="input" value="${own.all[name] || ((day || night).get(name) || '')}" data-token="${name}" data-mode="all" aria-label="${'--' + name}" spellcheck="false"></div>`;
    }
    var cur = own.all[name] || (day || night).get(name), n = parseFloat(cur);
    if (/s$/.test(cur) && !/ms$/.test(cur)) n *= 1000;
    return html`<div class="studio__row">${label}<span class="studio__range"><input type="range" class="range" min="${tk[1]}" max="${tk[2]}" step="1" value="${isNaN(n) ? tk[1] : n}" style="${'--fill:' + ((isNaN(n) ? 0 : n) - tk[1]) / (tk[2] - tk[1]) * 100 + '%'}" data-token="${name}" data-mode="all" data-unit="${tk[3]}" aria-label="${'--' + name}"><output class="num">${cur}</output></span></div>`;
  }
  /* a frame's resolved custom properties ('day' | 'night'), or null while it loads */
  function frameVars(mode) {
    var f = S.frames.filter(function (x) { return x.getAttribute('data-studio-frame') === mode; })[0];
    var doc = f && f.contentDocument; if (!doc || !doc.body || !f.contentWindow.Designs) return null;
    var cs = f.contentWindow.getComputedStyle(doc.documentElement), probe = doc.getElementById('studio-probe');
    if (!probe) { probe = doc.createElement('i'); probe.id = 'studio-probe'; probe.hidden = true; doc.body.appendChild(probe); }
    return {
      get: function (name) { return cs.getPropertyValue('--' + name).trim(); },
      rgb: function (name) {
        probe.style.color = ''; probe.style.color = cs.getPropertyValue('--' + name).trim();
        var m = /rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)/.exec(f.contentWindow.getComputedStyle(probe).color);
        return m ? [+m[1], +m[2], +m[3]] : null;
      }
    };
  }
  function hex(vars, name) {
    var c = vars && vars.rgb(name);
    return c ? '#' + c.map(function (x) { return ('0' + Math.round(x).toString(16)).slice(-2); }).join('') : '#000000';
  }

  /* ---------- contrast: WCAG ratios of the token pairs, in each mode the theme has ---------- */
  function luminance(c) {
    var v = c.map(function (x) { x /= 255; return x <= 0.03928 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); });
    return 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
  }
  function ratio(a, b) { var x = luminance(a), y = luminance(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); }
  function checkContrast() {
    var list = $('[data-studio-warnings]', S.el); if (!list) return;
    var out = [];
    S.draft.modes.forEach(function (m) {
      var v = frameVars(m); if (!v) return;
      PAIRS.forEach(function (p) {
        var fg = v.rgb(p[0]), bg = v.rgb(p[1]); if (!fg || !bg) return;
        var r = ratio(fg, bg);
        if (r < p[3]) out.push(html`<li class="studio__warn">${icon('warning', null, 'ic-16')}<span><b>${p[2]}</b>, ${m}: ${r.toFixed(1)} to 1 (needs ${p[3]}). <code>--${p[0]}</code> on <code>--${p[1]}</code></span></li>`);
      });
    });
    list.innerHTML = String(out.length ? html`${out}` : html`<li class="studio__ok">${icon('check', null, 'ic-16')}<span>Every text and accent pair reads well${S.draft.modes.length > 1 ? ' in day and night' : ''}.</span></li>`);
  }

  /* ---------- loops that draw more than they need to (designs/loops.js) ----------
     from the CSS: no rate (with an Add it), what they redraw, glass they sit in; and, from the live previews, the loops
     on the screen shown that pass under glass, which only where things land can tell */
  function checkLoops() {
    var css = S.draft.css;
    App.themeLoops(css, S.draft.extends).then(function (hints) {
      if (!S || S.draft.css !== css) return;
      S.loops = { css: css, hints: hints };
      showLoops();
      lookLive();
    }, function (err) { console.warn('lull: couldn\'t check the theme\'s loops', err); });
  }
  /* the previews' loops under glass, once they have started (and been paced) after a change */
  var liveTimer = 0;
  function lookLive() {
    clearTimeout(liveTimer);
    liveTimer = setTimeout(function () {
      if (!S || !window.LullLoops) return;
      var found = {}, screen = (ROUTES.filter(function (r) { return r[0] === S.route; })[0] || ['', 'this screen'])[1];
      S.frames.forEach(function (f) {
        var w = f.contentWindow, mode = f.getAttribute('data-studio-frame');
        if (!w || !w.document || !w.Designs) return;
        LullLoops.underGlass(w).forEach(function (g) {
          var key = g.animation + '|' + g.target + '|' + g.glass;
          (found[key] = found[key] || Object.assign({ kind: 'glass-live', screen: screen, modes: [] }, g)).modes.push(mode);
          found[key].exact = found[key].exact && g.exact;
        });
      });
      S.live = Object.keys(found).map(function (k) { return found[k]; });
      showLoops();
    }, 1500);
  }
  /* the loops, under Loops, and the reasons a theme words for itself, under Player words: each fix lull can make (a
     rate, its own words) a button, by the hint's place in S.loops.hints */
  function showLoops() {
    var loops = $('[data-studio-loops]', S.el), words = $('[data-studio-words]', S.el); if (!loops || !S.loops) return;
    var own = S.loops.hints.map(function (h, n) { return Object.assign({ n: n }, h); });
    /* what the previews show adds to what the CSS says: a loop it already places inside glass isn't named twice */
    var inGlass = own.filter(function (h) { return h.kind === 'glass'; }).map(function (h) { return h.animation; });
    var looping = own.filter(function (h) { return h.kind !== 'status-words'; })
      .concat((S.live || []).filter(function (g) { return inGlass.indexOf(g.animation) < 0; }));
    var saying = own.filter(function (h) { return h.kind === 'status-words'; });
    var add = function (h) {
      var words = h.kind === 'status-words';
      return html`<button type="button" class="btn btn--sm" data-studio-loop="${h.n}" aria-label="${words ? 'Use lull’s own words on ' + h.selector : 'Add --loop-fps: ' + h.fps + ' to ' + h.selector}">${words ? 'Use lull’s own words' : 'Add it'}</button>`;
    };
    var ok = function (text) { return html`<ul class="studio__warnings" role="list"><li class="studio__ok">${icon('check', null, 'ic-16')}<span>${text}</span></li></ul>`; };
    loops.innerHTML = String(looping.length ? App.themeLoopsList(looping, { lines: true, add: add })
      : ok('Every loop names its rate, moves by transform and opacity, and stays off glass on this screen.'));
    words.innerHTML = String(saying.length ? App.themeLoopsList(saying, { lines: true, add: add })
      : ok('Every state the player shows is in words that hold for every song.'));
  }
  function addRate(n) {
    var h = S.loops && S.loops.css === S.draft.css && S.loops.hints[n]; if (!h) return;
    setCss(LullLoops.apply(S.draft.css, [h]), true);
    App.announce(h.kind === 'status-words' ? 'Now lull’s own words on ' + h.selector : 'Added --loop-fps: ' + h.fps + ' to ' + h.selector);
  }

  /* ---------- save: theme.json, then theme.css (the hub checks the whole package each time) ---------- */
  function slug(name) {
    var s = name.toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 32) || 'theme';
    if (Designs.builtIn.indexOf(s) >= 0 || /^\d/.test(s)) s = 'my-' + s;
    var id = s, i = 2;
    while (App.designs[id]) id = s + '-' + i++;
    return id;
  }
  function manifest(id) {
    var d = S.draft, day = frameVars(d.modes[0]) || frameVars('day');
    var m = { id: id, name: d.name.trim() || 'My theme', author: d.author, api: Designs.API, extends: d.extends || null, shell: d.shell,
      swatches: ['sky', 'panel', 'accent-ui', 'fg-panel'].map(function (n) { return hex(day, n); }), modes: d.modes, description: d.description };
    if (d.coverVariety) m.coverVariety = true;
    if (d.window) m.window = d.window;
    return m;
  }
  function save() {
    if (!S) return Promise.resolve(null);
    var id = S.id || slug(S.draft.name), css = S.draft.css;
    state('Saving...');
    return M.api('/api/themes/' + id + '/theme.json', { method: 'PUT', body: JSON.stringify(manifest(id)) })
      .then(function () { return M.api('/api/themes/' + id + '/theme.css', { method: 'PUT', body: css }); })
      .then(function (saved) {
        App.themePictures(saved);                  // its card's picture, of what was just saved (ADR-0080)
        if (!S) return id;
        S.id = id; S.dirty = false; state();
        App.announce('Saved ' + S.draft.name);
        return Designs.refresh().then(function () { return id; });
      })
      .catch(function (err) {
        if (S) state('Not saved');
        App.toast('Couldn\'t save the theme: ' + err.message, { kind: 'error', sticky: true });
        return null;
      });
  }
  App.themeStudio = openStudio;
})();
