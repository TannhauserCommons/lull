/* designs/loops.js: a theme's loops that never end and name no rate (docs/themes.md, Drawing cheaply). Such a loop
   redraws on every screen refresh (165 times a second on a fast screen) and keeps moving when the listener asks lull to
   pause animations, since lull paces and holds only loops that set --loop-fps on what they animate (app.js). lull
   can't pick a rate for a theme, so Theme Studio and importing a theme (screens/appearance.js) say so, with the rate
   the guide suggests and the edit that adds it. A hint, never a refusal: the theme works either way.
   LullLoops.find(css, { also, base }) -> [{ selector, animation, line, fps, why, sure, at, insert }]:
     also   CSS that is in the page with the theme (lull's components.css, the design it extends): a rate set there
            on the same selector paces the theme's loop too (--loop-fps is set per element: it doesn't inherit)
     base   the design it extends, whose own selectors start :root[data-design="<base>"]
     at     where in css to insert `insert` (`--loop-fps: N;` after the loop's animation declaration)
   LullLoops.apply(css, hints) -> css with every hint's rate added; cost(n) and advice(hint) word them.
   Loaded by designs/studio.js and screens/appearance.js when they need it; by tests/web through module.exports. */
(function (root) {
  'use strict';

  /* ---------- reading CSS: rules with their declarations and where each sits in the text ---------- */
  /* the index after the comment or string starting at i, or i when there is none */
  function skip(css, i) {
    if (css[i] === '/' && css[i + 1] === '*') { var e = css.indexOf('*/', i + 2); return e < 0 ? css.length : e + 2; }
    if (css[i] === '"' || css[i] === '\'') {
      for (var j = i + 1; j < css.length; j++) { if (css[j] === '\\') j++; else if (css[j] === css[i] || css[j] === '\n') return j + 1; }
      return css.length;
    }
    return i;
  }
  /* the index of the first of `stops` at depth 0 (outside (), [], comments and strings) from i, or css.length */
  function scan(css, i, stops) {
    var depth = 0;
    while (i < css.length) {
      var n = skip(css, i); if (n !== i) { i = n; continue; }
      var c = css[i];
      if (depth === 0 && stops.indexOf(c) >= 0) return i;
      if (c === '(' || c === '[') depth++;
      else if ((c === ')' || c === ']') && depth > 0) depth--;
      i++;
    }
    return i;
  }
  /* the index of the } that closes the block whose { is at i */
  function close(css, i) {
    var depth = 0;
    for (var j = i; j < css.length; j++) {
      var n = skip(css, j); if (n !== j) { j = n - 1; continue; }
      if (css[j] === '{') depth++;
      else if (css[j] === '}' && --depth === 0) return j;
    }
    return css.length;
  }
  var GROUPS = /^@(media|supports|layer|container|scope|document|starting-style)\b/i, KEYFRAMES = /^@(-webkit-)?keyframes\s+(\S+)/i;
  /* { rules: [{ selector, decls: [{ name, value, from, to, semi }] }], keyframes: { name: body } }: a declaration's
     text runs from `from` to `to`, and `semi` is where its ; is (-1 for the last one, written without) */
  function parse(css) {
    var out = { rules: [], keyframes: {} };
    (function block(i, end) {
      while (i < end) {
        var stop = scan(css, i, '{};');
        if (stop >= end) return;
        var prelude = strip(css.slice(i, stop)).trim();
        if (css[stop] !== '{') { i = stop + 1; continue; }        // a statement (@import, @layer a, b;) or a stray }
        var shut = close(css, stop), m;
        if (GROUPS.test(prelude)) block(stop + 1, shut);
        else if ((m = KEYFRAMES.exec(prelude))) out.keyframes[unquote(m[2])] = css.slice(stop + 1, shut);
        else if (prelude && prelude[0] !== '@') out.rules.push({ selector: prelude, decls: decls(stop + 1, shut) });
        i = shut + 1;
      }
    })(0, css.length);
    return out;
    function decls(i, end) {
      var list = [];
      while (i < end) {
        var stop = Math.min(scan(css, i, ';{}'), end);
        if (css[stop] === '{') { i = close(css, stop) + 1; continue; }     // a nested rule: not this rule's own
        var text = css.slice(i, stop), colon = scan(text, 0, ':');
        if (colon < text.length) {
          var lead = text.length - text.replace(/^\s+/, '').length, tail = text.replace(/\s+$/, '').length;
          list.push({ name: strip(text.slice(0, colon)).trim().toLowerCase(), value: strip(text.slice(colon + 1)).trim(),
            from: i + lead, to: i + tail, semi: css[stop] === ';' ? stop : -1 });
        }
        i = stop + 1;
      }
      return list;
    }
  }
  function strip(s) { return s.replace(/\/\*[\s\S]*?(\*\/|$)/g, ' '); }
  function unquote(s) { return s.replace(/^(['"])(.*)\1$/, '$2'); }
  /* parts of a value split at depth-0 commas */
  function commas(value) {
    var out = [], i = 0;
    while (i <= value.length) { var c = scan(value, i, ','); out.push(value.slice(i, c).trim()); i = c + 1; }
    return out;
  }
  /* a selector as both sides of a comparison write it: one space between parts, no quotes in [attr=value], and the
     design it belongs to (:root[data-design=...] in front) left off */
  function norm(sel) {
    return sel.replace(/\[\s*([\w-]+)\s*([~|^$*]?=)\s*(['"])([\w-]*)\3\s*\]/g, '[$1$2$4]').replace(/\s+/g, ' ')
      .replace(/\s*([>+~])\s*/g, ' $1 ').replace(/(^|[^:]):(before|after|first-line|first-letter)\b/g, '$1::$2')
      .replace(/^:root\[data-design=[\w-]+\]\s*/, '').trim();
  }

  /* ---------- the loops ---------- */
  var NOT_A_NAME = /^(infinite|linear|ease|ease-in|ease-out|ease-in-out|step-start|step-end|normal|reverse|alternate|alternate-reverse|none|forwards|backwards|both|running|paused|initial|inherit|unset|revert|revert-layer|auto)$/i;
  function nameOf(layer) {
    var words = layer.replace(/[\w-]+\([^)]*\)/g, ' ').split(/\s+/);
    for (var i = 0; i < words.length; i++) {
      var w = words[i];
      if (w && !NOT_A_NAME.test(w) && !/^[-+.\d]/.test(w)) return unquote(w);
    }
    return '';
  }
  var important = function (v) { return v.replace(/\s*!important\s*$/i, ''); };
  /* the declaration that makes a rule loop forever, the loop's name and its timing, or null */
  function loopIn(rule) {
    var get = function (n) { var d = rule.decls.filter(function (x) { return x.name === n; }).pop(); return d; };
    var anim = get('animation'), count = get('animation-iteration-count'), name = get('animation-name');
    if (count && /(^|[\s,])infinite($|[\s,])/i.test(count.value)) {
      return { decl: count, name: name ? commas(important(name.value))[0] : anim ? nameOf(commas(important(anim.value))[0]) : '', value: anim ? anim.value : count.value };
    }
    if (anim) {
      var layers = commas(important(anim.value)).filter(function (l) { return /(^|\s)infinite(\s|$)/i.test(l); });
      if (layers.length) return { decl: anim, name: layers.map(nameOf).filter(Boolean).join(', '), value: layers.join(', ') };
    }
    return null;
  }
  function rateIn(rule) {
    var d = rule.decls.filter(function (x) { return x.name === '--loop-fps'; }).pop();
    return d ? parseFloat(important(d.value)) > 0 : false;
  }
  /* every selector a rate is set on, as norm() writes it */
  function paced(rules) {
    var set = {};
    rules.forEach(function (r) { if (rateIn(r)) commas(r.selector).forEach(function (s) { set[norm(s)] = true; }); });
    return set;
  }

  /* the guide's rate for a loop (docs/themes.md, Drawing cheaply): 30 for what moves with the music, 10 for a steps()
     loop or a fade or twinkle, 20 for motion; 20 too when nothing says what it is, and then it says so */
  var MUSIC = /(^|[^\w-])(eq|wave|vu|beat)([^\w-]|$)|is-playing|data-playing|-playing\b/i;
  var MOVES = /translate|rotate|skew|matrix|perspective|(^|[\s;{])(left|top|right|bottom|inset|margin[\w-]*|background-position[\w-]*|offset-[\w-]+|translate|rotate)\s*:/i;
  var FADES = /^(opacity|scale|filter|color|background-color|fill|stroke|visibility|transform|box-shadow|text-shadow)$/;
  function suggest(rule, loop, keyframes) {
    if (MUSIC.test(rule.selector) || MUSIC.test(loop.name)) return { fps: 30, why: 'it moves with the music', sure: true };
    if (/steps\(|step-start|step-end/i.test(loop.value)) return { fps: 10, why: 'it steps, like a blinking caret', sure: true };
    var body = loop.name.split(', ').map(function (n) { return keyframes[n]; }).filter(Boolean).join('\n');
    if (body) {
      if (MOVES.test(body)) return { fps: 20, why: 'it moves', sure: true };
      var props = (strip(body).match(/[\w-]+(?=\s*:)/g) || []).map(function (p) { return p.toLowerCase(); });
      if (props.length && props.every(function (p) { return FADES.test(p); })) return { fps: 10, why: 'it fades or twinkles', sure: true };
    }
    return { fps: 20, why: 'lull can\'t tell what it shows', sure: false };
  }

  function find(css, opts) {
    opts = opts || {};
    var own = parse(css || ''), others = (opts.also || []).map(parse);
    var rated = paced(own.rules), keyframes = {};
    others.forEach(function (o) { Object.assign(rated, paced(o.rules)); Object.assign(keyframes, o.keyframes); });
    Object.assign(keyframes, own.keyframes);
    var hints = [];
    own.rules.forEach(function (rule) {
      var loop = loopIn(rule);
      if (!loop || rateIn(rule) || commas(rule.selector).every(function (s) { return rated[norm(s)]; })) return;
      var s = suggest(rule, loop, keyframes), d = loop.decl;
      hints.push({ selector: rule.selector.replace(/\s+/g, ' '), animation: loop.name, line: css.slice(0, d.from).split('\n').length,
        fps: s.fps, why: s.why, sure: s.sure,
        at: d.semi >= 0 ? d.semi + 1 : d.to, insert: (d.semi >= 0 ? ' ' : '; ') + '--loop-fps: ' + s.fps + ';' });
    });
    return hints;
  }
  function apply(css, hints) {
    return hints.slice().sort(function (a, b) { return b.at - a.at; })
      .reduce(function (out, h) { return out.slice(0, h.at) + h.insert + out.slice(h.at); }, css);
  }

  /* the words Theme Studio and the import dialog use: what such a loop costs, and the rate a hint suggests */
  function cost(n) {
    return (n === 1 ? 'It never ends and names no rate, so it redraws' : 'They never end and name no rate, so each redraws') +
      ' on every screen refresh, up to 165 times a second, and keeps moving when \u201cPause animations when lull isn\u2019t the active window\u201d is on.';
  }
  function advice(h) {
    return h.sure ? h.why[0].toUpperCase() + h.why.slice(1) + ', so ' + h.fps + ' a second suits it.'
      : 'lull can\'t tell what it shows, so it suggests ' + h.fps + ' a second, which suits most motion (10 for a slow fade, 30 for what moves with the music).';
  }

  var api = { find: find, apply: apply, cost: cost, advice: advice };
  root.LullLoops = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof self !== 'undefined' ? self : this);
