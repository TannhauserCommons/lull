/* designs/loops.js: a theme's loops that never end and cost more than they need to (docs/themes.md, Drawing cheaply):
   one that names no rate redraws on every screen refresh (165 times a second on a fast screen) and keeps moving when the
   listener asks lull to pause animations, since a loop is cheap and holds only when it is stepped where it is declared
   and held by var(--loop-play) (lull's own, components.css), or names --loop-fps on what it animates, which lull steps
   and holds (app.js); one that animates more than transform and opacity is drawn again at every step; one under or in
   live glass has the blur worked out again at every step. Theme Studio and importing a theme (screens/appearance.js)
   say so, with the cheaper way, and add a rate where one is missing. A hint, never a refusal: the theme works either way.
   LullLoops.find(css, { also, base, unsure }) -> hints, each { kind, selector, animation, line } and by kind:
     rate      neither stepped and held where it is declared nor naming --loop-fps: { fps, why, sure, at, insert }
     repaint   it animates more than transform and opacity: { props, fix }
     glass     it is in an element with a live backdrop-filter (or has one itself): { glass, itself }
     glass-maybe  with `unsure` (no live page to look in), one for every loop the CSS can't place: { loops, glass }
     also   CSS that is in the page with the theme (lull's components.css, the design it extends): a rate or a hold set
            there on the same selector counts for the theme's loop too (both are set per element: neither inherits)
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
  /* { rules: [{ selector, decls: [{ name, value, from, to, semi }], supports }], keyframes: { name: body } }: a
     declaration's text runs from `from` to `to`, and `semi` is where its ; is (-1 for the last one, written without);
     `supports` when the rule is inside an @supports block */
  function parse(css) {
    var out = { rules: [], keyframes: {} };
    (function block(i, end, supports) {
      while (i < end) {
        var stop = scan(css, i, '{};');
        if (stop >= end) return;
        var prelude = strip(css.slice(i, stop)).trim();
        if (css[stop] !== '{') { i = stop + 1; continue; }        // a statement (@import, @layer a, b;) or a stray }
        var shut = close(css, stop), m;
        if (GROUPS.test(prelude)) block(stop + 1, shut, supports || /^@supports\b/i.test(prelude));
        else if ((m = KEYFRAMES.exec(prelude))) out.keyframes[unquote(m[2])] = css.slice(stop + 1, shut);
        else if (prelude && prelude[0] !== '@') out.rules.push({ selector: prelude, decls: decls(stop + 1, shut), supports: !!supports });
        i = shut + 1;
      }
    })(0, css.length, false);
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
  /* held while lull asks loops to hold still (animation-play-state: var(--loop-play, running)) */
  function holdIn(rule) {
    var d = rule.decls.filter(function (x) { return x.name === 'animation-play-state'; }).pop();
    return !!d && /var\(\s*--loop-play\b/.test(d.value);
  }
  /* stepped where it is declared: steps(), or one of lull's stepped easings (--loop-ease-in-out-55) */
  var STEPPED = /steps\(|step-start|step-end|var\(\s*--loop-/i;
  function steppedIn(rule, loop) {
    var t = rule.decls.filter(function (x) { return x.name === 'animation-timing-function'; }).pop();
    return STEPPED.test(t ? t.value : loop.value);
  }
  /* every selector a rate, or a hold, is set on, as norm() writes it */
  function marked(rules, test) {
    var set = {};
    rules.forEach(function (r) { if (test(r)) each(r, function (k) { set[k] = true; }); });
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

  /* what a loop's keyframes animate besides what the compositor moves alone (transform and opacity: the screen moves
     and fades a drawn layer without lull drawing it again). Anything else lull draws again at every step of the loop,
     on the thread that runs the app. The same split as the perf gate's (tests/ui/gates/scenarios.mjs). Null when the
     keyframes aren't in the CSS read */
  var CHEAP = /^(-webkit-)?(transform|opacity|translate|scale|rotate)$/;
  function repaints(loop, keyframes) {
    var props = {}, known = false;
    loop.name.split(', ').forEach(function (n) {
      if (keyframes[n] == null) return;
      known = true;
      parse(keyframes[n]).rules.forEach(function (r) {
        r.decls.forEach(function (d) { if (!/^animation/.test(d.name) && !CHEAP.test(d.name)) props[d.name] = true; });
      });
    });
    return known ? Object.keys(props) : null;
  }
  /* the cheaper way for what a loop repaints, by what it is (docs/themes.md, Drawing cheaply) */
  var REPAINT_FIX = [
    [/shadow|filter/, 'For a glow, draw it once on a ::before or ::after and fade and grow that with opacity and transform: scale().'],
    [/^(background-position[\w-]*|background-size|(-webkit-)?mask-position|object-position)$/, 'To slide a pattern, draw it on a ::before larger than the box and move that with transform: translate().'],
    [/^(left|top|right|bottom|inset|inset-[\w-]+|margin[\w-]*|padding[\w-]*)$/, 'To move it, use transform: translate() instead.'],
    [/^((min-|max-)?(width|height|inline-size|block-size)|flex-basis|font-size|line-height|border-width|border-radius)$/, 'To grow it, use transform: scale() instead.'],
    [/colou?r|^(fill|stroke)$/, 'To change its colour, lay the second colour on a ::before and fade it in with opacity.'],
    [/clip-path|^mask/, 'To reveal it, move or fade a ::before over it with transform and opacity.']
  ];
  function repaintFix(props) {
    var said = [];
    props.forEach(function (p) {
      var fix = REPAINT_FIX.filter(function (f) { return f[0].test(p); })[0];
      var text = fix ? fix[1] : 'Rebuild it with transform and opacity, which the screen moves without lull drawing anything.';
      if (said.indexOf(text) < 0) said.push(text);
    });
    return said.join(' ');
  }

  /* glass: an element with a live backdrop-filter, which blurs again whatever changes under or inside it, every frame.
     From the CSS alone lull can be sure only that a loop is inside glass: its own rule blurs, or a rule on an element
     it sits in does (an ancestor in its selector). Whether it passes under glass is a matter of where things land on
     the screen: underGlass() looks in a live page (Theme Studio's previews), and find() with `unsure` says it can't tell */
  function blurs(rule, none) {
    return rule.decls.some(function (d) { return /^(-webkit-)?backdrop-filter$/.test(d.name) && /^none$/i.test(important(d.value)) === !!none; });
  }
  /* a selector with its :is() and :where() lists written out: `.a :is(.b, .c)` -> `.a .b`, `.a .c` */
  function alternatives(sel) {
    var m = /:(is|where)\(/i.exec(sel); if (!m) return [sel];
    var open = m.index + m[0].length, close = scan(sel, open, ')');
    var head = sel.slice(0, m.index), tail = sel.slice(close + 1);
    return [].concat.apply([], commas(sel.slice(open, close)).map(function (alt) { return alternatives(head + alt + tail); }));
  }
  /* each element selector a rule is written for, as find() compares them (norm) and as it names them */
  function each(rule, fn) {
    commas(rule.selector).forEach(function (s) {
      alternatives(s).forEach(function (a) { fn(norm(a), a.replace(/\s+/g, ' ').replace(/^:root\[data-design=[^\]]*\]\s*/, '')); });
    });
  }
  /* the selectors of the elements a selector's element sits in: `.a > .b .c::before` -> `.a > .b .c`, `.a > .b`, `.a` */
  function within(sel) {
    var parts = norm(sel).replace(/::?(before|after|marker|placeholder|selection|backdrop|first-line|first-letter)\b.*$/, '');
    var out = [parts], words = [], depth = 0, cur = '';
    for (var i = 0; i < parts.length; i++) {
      var c = parts[i];
      if (c === '(' || c === '[') depth++; else if (c === ')' || c === ']') depth--;
      if (c === ' ' && depth === 0) { words.push(cur); cur = ''; } else cur += c;
    }
    words.push(cur);
    for (var j = words.length - 1; j > 0; j--) {
      if (words[j] === '>' || words[j] === '+' || words[j] === '~') continue;
      var before = words[j - 1];
      if (before === '+' || before === '~') continue;            // a sibling, not an element it sits in
      var head = words.slice(0, before === '>' ? j - 1 : j).join(' ');
      if (head) out.push(head);
    }
    return out;
  }

  /* a loop kept as a fallback: a rule inside @supports turns it off (animation: none) where the browser has what its
     better version needs, on the same element (the same selector, or the same with more to it, such as a class more).
     Skydeck's down LED: a box-shadow pulse where anchor positioning isn't, a transform and opacity glow where it is */
  function compounds(sel) { return within(sel)[0].split(' '); }
  function simple(compound) { return compound.match(/[.#]?[\w-]+(\([^)]*\))?|\[[^\]]*\]|::?[\w-]+(\([^)]*\))?/g) || []; }
  function covers(off, sel) {
    var a = compounds(off), b = compounds(sel);
    return a.length === b.length && b.every(function (c, i) {
      return /^[>+~]$/.test(c) ? a[i] === c : simple(c).every(function (x) { return simple(a[i]).indexOf(x) >= 0; });
    });
  }
  function fallback(rule, all) {
    return all.some(function (r) {
      if (!r.supports || r === rule) return false;
      var off = r.decls.some(function (d) { return (d.name === 'animation' || d.name === 'animation-name') && /^none$/i.test(important(d.value)); });
      return off && commas(rule.selector).every(function (sel) { return commas(r.selector).some(function (o) { return covers(o, sel); }); });
    });
  }

  function find(css, opts) {
    opts = opts || {};
    var own = parse(css || ''), others = (opts.also || []).map(parse);
    var rated = marked(own.rules, rateIn), held = marked(own.rules, holdIn), keyframes = {}, glass = {};
    /* glass as the cascade leaves it: a later backdrop-filter: none on the same element takes it away (Glasshouse's baked
       panels) */
    var addGlass = function (rules) {
      rules.forEach(function (r) {
        if (blurs(r)) each(r, function (k, name) { glass[k] = name; });
        else if (blurs(r, true)) each(r, function (k) { delete glass[k]; });
      });
    };
    others.forEach(function (o) { Object.assign(rated, marked(o.rules, rateIn)); Object.assign(held, marked(o.rules, holdIn)); Object.assign(keyframes, o.keyframes); addGlass(o.rules); });
    Object.assign(keyframes, own.keyframes);
    addGlass(own.rules);
    var hints = [], maybe = [];
    own.rules.forEach(function (rule) {
      var loop = loopIn(rule); if (!loop || fallback(rule, own.rules)) return;
      var d = loop.decl, where = { selector: rule.selector.replace(/\s+/g, ' '), animation: loop.name, line: css.slice(0, d.from).split('\n').length };
      var hint = function (kind, more) { hints.push(Object.assign({ kind: kind }, where, more)); };
      var mine = []; each(rule, function (k) { mine.push(k); });
      var every = function (set) { return mine.every(function (k) { return set[k]; }); };
      /* a rate named, or stepped in its own animation (a rule that sets the animation sets its timing) and held */
      if (!rateIn(rule) && !every(rated) && !(steppedIn(rule, loop) && (holdIn(rule) || every(held)))) {
        var s = suggest(rule, loop, keyframes);
        hint('rate', { fps: s.fps, why: s.why, sure: s.sure, at: d.semi >= 0 ? d.semi + 1 : d.to, insert: (d.semi >= 0 ? ' ' : '; ') + '--loop-fps: ' + s.fps + ';' });
      }
      var props = repaints(loop, keyframes);
      if (props && props.length) hint('repaint', { props: props, fix: repaintFix(props) });
      var inside = blurs(rule) ? where.selector : null;
      commas(rule.selector).forEach(function (sel) {
        alternatives(sel).forEach(function (alt) { within(alt).forEach(function (w) { if (!inside && glass[w]) inside = glass[w]; }); });
      });
      if (inside) hint('glass', { glass: inside, itself: inside === where.selector });
      else if (opts.unsure && Object.keys(glass).length) maybe.push(where);
    });
    if (maybe.length) hints.push({ kind: 'glass-maybe', loops: maybe, glass: Object.keys(glass).map(function (k) { return glass[k]; }) });
    return hints;
  }
  /* the edits the hints carry (rates), each where its loop is written */
  function apply(css, hints) {
    return hints.filter(function (h) { return h.insert; }).sort(function (a, b) { return b.at - a.at; })
      .reduce(function (out, h) { return out.slice(0, h.at) + h.insert + out.slice(h.at); }, css);
  }

  /* the box a loop draws in, now: an element's own; a pseudo-element's from its styles where they pin it down (fixed, or
     absolute in a positioned box), its transform at this moment applied; else the element it belongs to, which holds
     it in most cases but not all, so `exact` is false */
  function boxOf(win, t, pseudo) {
    if (!pseudo) return { box: t.getBoundingClientRect(), exact: true };
    var st = win.getComputedStyle(t, pseudo), host = { box: t.getBoundingClientRect(), exact: false }, at = null;
    if (st.position === 'fixed') at = { left: 0, top: 0 };
    else if (st.position === 'absolute') {
      var c = t;
      while (c && c !== win.document.documentElement && win.getComputedStyle(c).position === 'static') c = c.parentElement;
      var cb = c && c !== win.document.documentElement ? c.getBoundingClientRect() : null;
      at = cb ? { left: cb.left + c.clientLeft, top: cb.top + c.clientTop } : { left: 0, top: 0 };
    }
    var l = parseFloat(st.left), tp = parseFloat(st.top), w = parseFloat(st.width), h = parseFloat(st.height);
    if (!at || [l, tp, w, h].some(isNaN)) return host;
    var m = st.transform && st.transform !== 'none' ? new win.DOMMatrix(st.transform) : null;
    var o = (st.transformOrigin || '0 0').split(' ').map(parseFloat);
    var xs = [], ys = [];
    [[0, 0], [w, 0], [0, h], [w, h]].forEach(function (p) {
      var q = m ? m.transformPoint(new win.DOMPoint(p[0] - o[0], p[1] - o[1])) : { x: p[0] - o[0], y: p[1] - o[1] };
      xs.push(at.left + l + o[0] + q.x); ys.push(at.top + tp + o[1] + q.y);
    });
    return { box: { left: Math.min.apply(null, xs), right: Math.max.apply(null, xs), top: Math.min.apply(null, ys), bottom: Math.max.apply(null, ys) }, exact: true };
  }
  /* the loops in a live page (window) that pass under or sit in live glass, as the perf gate finds them: each running
     loop's box, clipped by what clips it, that overlaps a shown element with a backdrop-filter.
     [{ animation, target, glass, exact }], each element named tag.class.class(::pseudo); exact false: lull measured the
     element a pseudo-element belongs to, so the loop may pass under the glass */
  function underGlass(win) {
    var doc = win.document, out = [];
    if (!doc.getAnimations) return out;
    var shown = function (e, o) { return !e.checkVisibility || e.checkVisibility(o); };
    var named = function (e, pseudo) { return e.tagName.toLowerCase() + [].slice.call(e.classList, 0, 2).map(function (c) { return '.' + c; }).join('') + (pseudo || ''); };
    var glass = [].filter.call(doc.querySelectorAll('body *'), function (e) {
      var st = win.getComputedStyle(e), f = st.backdropFilter || st.webkitBackdropFilter;
      return f && f !== 'none' && shown(e);
    });
    if (!glass.length) return out;
    var hit = function (a, b) { return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom; };
    doc.getAnimations().forEach(function (a) {
      var e = a.effect, t = e && e.target;
      if (!t || a.playState !== 'running' || e.getTiming().iterations !== Infinity) return;
      if (a.timeline && a.timeline.constructor.name !== 'DocumentTimeline') return;
      if (!shown(t, { contentVisibilityAuto: true, visibilityProperty: true })) return;
      var m = boxOf(win, t, e.pseudoElement), b = m.box, r = { left: b.left, right: b.right, top: b.top, bottom: b.bottom };
      var fixed = e.pseudoElement && win.getComputedStyle(t, e.pseudoElement).position === 'fixed';   // clipped by no box it sits in
      for (var c = fixed ? null : e.pseudoElement ? t : t.parentElement; c; c = c.parentElement) {
        if (win.getComputedStyle(c).overflow === 'visible') continue;
        var k = c.getBoundingClientRect();
        r.left = Math.max(r.left, k.left); r.right = Math.min(r.right, k.right); r.top = Math.max(r.top, k.top); r.bottom = Math.min(r.bottom, k.bottom);
      }
      if (r.right <= r.left || r.bottom <= r.top || r.bottom <= 0 || r.top >= win.innerHeight || r.right <= 0 || r.left >= win.innerWidth) return;
      var g = glass.filter(function (x) { return hit(r, x.getBoundingClientRect()); })[0];
      if (g) out.push({ animation: a.animationName || 'an animation', target: named(t, e.pseudoElement), glass: named(g), exact: m.exact });
    });
    return out;
  }

  /* the words Theme Studio and the import dialog use. say(kind, n): a group's heading and what its loops cost;
     cost(n) and advice(hint): a rate's */
  function cost(n) {
    return (n === 1 ? 'It never ends and names no rate, so it redraws' : 'They never end and name no rate, so each redraws') +
      ' on every screen refresh, up to 165 times a second, and keeps moving when “Pause animations when lull isn’t the active window” is on.';
  }
  function advice(h) {
    return h.sure ? h.why[0].toUpperCase() + h.why.slice(1) + ', so ' + h.fps + ' a second suits it.'
      : 'lull can\'t tell what it shows, so it suggests ' + h.fps + ' a second, which suits most motion (10 for a slow fade, 30 for what moves with the music).';
  }
  function say(kind, n) {
    var one = n === 1;
    if (kind === 'rate') return { title: 'No rate', lead: cost(n) };
    if (kind === 'repaint') {
      return { title: 'Redrawn at every step', lead: (one ? 'It changes something' : 'They change something') + ' lull has to draw again at every step of the loop, on the thread that runs the app. ' +
        'Moving and fading (transform and opacity) cost almost nothing: the screen does those by itself. lull can’t rewrite ' +
        (one ? 'it' : 'them') + ' for you and be sure ' + (one ? 'it looks' : 'they look') + ' the same, so here’s the cheaper way.' };
    }
    return { title: 'Under glass', lead: 'Glass blurs what’s behind it as it changes (backdrop-filter), so a loop under it or inside it makes lull blur it all again at every step: the blur never rests. ' +
      'Move the loop out from under the glass, or bake the glass: draw what’s behind it into the glass’s own background, so nothing is blurred live (“Bake what’s static” in the theme guide).' };
  }

  var api = { find: find, apply: apply, underGlass: underGlass, cost: cost, advice: advice, say: say };
  root.LullLoops = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof self !== 'undefined' ? self : this);
