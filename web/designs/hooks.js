/* designs/hooks.js: what a theme styles that lull has moved, removed or keeps for itself (docs/themes.md, section 9,
   Hooks that changed, and section 11). Theme Studio (when it opens a theme, and at each save) and an import ask once,
   never while drawing. A warning, never a refusal: the theme works either way, less the rules that no longer match.
   LullHooks.find(css, { slots }) -> hints, one per thing named, each { kind, name, rules: [{ selector, line }] } and
   by kind:
     moved     a rule that places or styles a hook lull has since moved (MOVED): { say, instead, since }
     gone      a class or data-* attribute that is neither in lull nor in the theme's own slot templates (`slots`, their
               markup): its rules match nothing
     internal  one of lull's internal hooks (section 11, Internal): it works today and may change in any release
   LullHooks.words(hint) -> { what, then }: the hint in plain words, to follow its name.
   Loaded after designs/loops.js (its CSS reader) by designs/studio.js and screens/your-themes.js when they need it;
   by tests/web through module.exports. */
(function (root) {
  'use strict';

  /* written from docs/themes.md section 11 and from web/ by tests/web/theme-hooks.test.mjs (LULL_WRITE_PACKAGES=1):
     the stable and internal hooks (a class stands for its BEM block; a name ending * for every name it starts), and
     the classes and data-* attributes lull's markup sets, with the prefixes it builds names from */
  var HOOKS = {"stable":{"classes":["app","badge","badge-below","badge-count","badge-new","badge-saved","badge-slsk","banner","banner-more","banner-slot","bezel-thumb","brand","btn","btn-gloss","btn-icon","btn-save","c1","c2","c3","c4","c5","c6","c7","c8","card","card-head","chip","chip-more","chip-reason","cls-attrib","cls-back","cls-browser","cls-busy","cls-card","cls-desk","cls-failed","cls-folder","cls-grid","cls-look","cls-meta","cls-more","cls-note","cls-now","cls-nsfw","cls-pop","cls-scrim","cls-shot","cls-skel-shot","cls-skipped","col-resize","cover","cover-view","cover-zoom","ctx","ctx-scrim","cx-dev","cx-pop","cx-remote","device","dialog","disc-card","disc-cold","disc-duo","disc-dup","disc-empty","disc-filters","disc-folder","disc-folder-tile","disc-hello","disc-home","disc-jump","disc-kv","disc-lead","disc-mod","disc-modules","disc-pick","disc-pick__why","disc-picks","disc-picks-skel","disc-q","disc-recent","disc-recents","disc-rel","disc-scan","disc-search","disc-shelf","disc-skel-shelf","disc-songs","disc-split","disc-splitview","disc-src","disc-starter","disc-starters","disc-step","disc-tabs","disc-tag","disc-tags","disc-top","dk-art","dk-caption","dk-card","dk-form","dk-glyph","dk-hubs","dk-mini","dk-modal","dk-offline","dk-page","dk-retry","dk-status","dk-strip","dk-strip-wrap","dk-tip","dk-titlebar","dl-btn","dl-mark","dpad","drag-chip","drop-line","drop-overlay","dup-better","dup-note","egg-line","egg-stage","empty","eq","eq-pop","eq-sheet","eqp","eqp--full","field","from","has-*","has-art","header","hub-pair","hub-qr","ic","ic-16","ic-24","input","inset","is-*","is-applied","is-bad","is-below","is-buf","is-buffering","is-current","is-done","is-dragging","is-drop-target","is-error","is-failed","is-going","is-hidden","is-lead","is-liked","is-off","is-on","is-open","is-out","is-past","is-peak","is-played","is-playing","is-playing-now","is-playing-place","is-power-on","is-preferred","is-resizing","is-revealed","is-scrubbing","is-selected","is-skipped","is-still","is-transferring","is-unavailable","is-wide","is-xfer","kbd","label-rule","lib","lib-about","lib-actions","lib-ahero","lib-artist","lib-bar","lib-bio","lib-card","lib-cols","lib-count","lib-day","lib-disc","lib-disco","lib-grid","lib-head","lib-hero","lib-history","lib-kicker","lib-listhead","lib-newtile","lib-notice","lib-page","lib-pick","lib-pick-list","lib-pick-new","lib-pick-none","lib-picker","lib-pl","lib-pl-list","lib-popular","lib-rule","lib-segs","lib-site","lib-skel-shelf","lib-sort","lib-sortnote","lib-table","lib-th","lib-thead","lib-tile","link","link-plain","main","menu","menu-list","mini","mosaic","nav-item","nav-pill","now","now-acts","now-dev","now-devc","now-head","now-ly","now-lycard","now-next","now-osd","now-q","now-qcard","now-sbtn","now-seg","now-slide","now-src","now-srccard","now__dev","now__devc","now__intro","now__side","nowcol","pbar","pbar__ask","pick-avail","pick-avail__src","pill-filter","pipeline","pipeline-wrap","place-mark","popover","progress","px","q-chip","rail","range","region-meta","region-title","rel-unshared","resolve","s-card","s-panel","s-screen","s-sky","score","scrim","scrobble","search-field","seek","seek-row","seek__target","seg","setup","setup-code","setup-step","sg","sheet","sheet-bar","shelf","shelf-pop","shortcut-list","shortcut-row","skel","skel-group","skel-row","skel-tile","skip-link","sky-line","sky-specks","sky-star","source-card","sparkle","spin-ring","src-badge","src-row","src-speed","src-table","status-list","status-row","switch","switch-row","sys-bar","sys-bars","sys-card","sys-design","sys-designs","sys-empty-stats","sys-gate","sys-grid","sys-group","sys-head","sys-heat","sys-ios","sys-lede","sys-nav","sys-page","sys-pal","sys-period","sys-row","sys-section","sys-settings","sys-span","sys-stats","sys-step","sys-tile","sys-tiles","sys-usage","sys-welcome","t-body","t-display","t-h2","t-lyric","t-meta","t-micro","t-px","t-row","t-title-xl","tab","tabbar","tile","tlist","toast","toast-region","tooltip","tooltip--asked","tray","tray__pill","trow","trow--dense","trow__num","view","vol","wave","wave-times","wordmark"],"attrs":["data-act","data-activity-count","data-answer","data-art","data-avail","data-bind","data-bits","data-brand","data-cache-cap","data-cache-used","data-classic","data-ctx","data-ctx-tab","data-design","data-dk","data-dk-artist","data-dk-buf","data-dk-dur","data-dk-now","data-dk-play-label","data-dk-player","data-dk-pos","data-dk-thumb","data-dk-title","data-dk-vol","data-drop","data-duration","data-egg","data-egg-of","data-egg-word","data-fit","data-fit-gave","data-fit-keep","data-fit-out","data-fit-row","data-fit-side","data-fit-sides","data-graphics","data-head","data-height","data-kbps","data-khz","data-l","data-like","data-liked","data-look","data-meta","data-module","data-motion","data-nav","data-noun","data-now","data-nowcol","data-nowcol-ok","data-on","data-page","data-panel","data-panel-split","data-place","data-player","data-playing","data-pointer","data-region","data-resize","data-route","data-saved","data-screen","data-sec","data-seek","data-shell","data-site","data-size","data-sortable","data-src","data-state","data-status","data-status-text","data-svc","data-tab","data-tauri-drag-region","data-theme","data-theme-fade","data-theme-pref","data-tip","data-tip-cut","data-title","data-titlebar","data-tracks","data-tray","data-two","data-value","data-view","data-was","data-window","data-window-inactive","data-window-preview","data-worst"]},"internal":{"classes":["cls-frame","cls-root","cls-slot","cluster","cx-foot","cx-list","cx-note","cx-title","disc-agg","disc-aggl","disc-alt","disc-alts","disc-cat","disc-cloud","disc-col","disc-fallback","disc-issue","disc-more","disc-note","disc-pending","disc-picker","disc-recover","disc-shead","disc-sheet","disc-steps","disc-sugg","disc-tile-open","disc-why","disc-why-pop","dk-actions","dk-choice","dk-choices","dk-code","dk-details","dk-dragstrip","dk-hb","dk-kv","dk-sep","ellipsis","fg-2","flow","gh-*","has-section","has-shot","hub-cred","hub-dev","hub-devs","hub-form","hub-jf","hub-lib","hub-qc","hub-slsk","is-all","is-better","is-closed","is-disabled","is-down","is-drop","is-here","is-in","is-index","is-leaving","is-lifted","is-nosource","is-online","is-phone","is-ready","is-sorting","is-stale","is-wait","lib-adddesc","lib-chips","lib-cover","lib-dl-storage","lib-dropzone","lib-facts","lib-flist","lib-foot","lib-form","lib-frow","lib-help","lib-hist","lib-imp","lib-import","lib-kept","lib-legend","lib-link","lib-link-card","lib-linkrev","lib-m3u","lib-more","lib-out","lib-status","lib-suggest","lib-tagcard","lib-tags","lib-textarea","lib-title","lib-toggle","lull-*","now-act","now-fade","num","prose","pt-*","sd-*","sl-*","spacer","stacked","studio","studio-sheet","sys-back","sys-cfg","sys-chips","sys-combo","sys-connect","sys-detail","sys-follows","sys-formats","sys-funnel","sys-health","sys-hit","sys-inline","sys-keys","sys-legend","sys-locked","sys-meter-label","sys-newart","sys-nomatch","sys-note","sys-off","sys-packs","sys-palette","sys-pending","sys-plus","sys-preview","sys-range","sys-rescan","sys-seg","sys-status","sys-stepper","sys-steps","sys-surprises","sys-taste","sys-themes","sys-then","sys-version","vh"],"attrs":["data-add-rates","data-album-id","data-attempt","data-banner","data-banner-*","data-booting","data-cfg","data-choice","data-close","data-cmd","data-cold-count","data-cols","data-cover-*","data-cx*","data-d","data-device","data-dir","data-dk-attempt","data-dk-close","data-dk-connect","data-dk-error","data-dk-found","data-dk-look","data-dk-offline","data-dk-retry-status","data-dk-welcome","data-dl","data-dl-mark","data-drop-theme","data-eq*","data-fill","data-filters","data-fk","data-form","data-format","data-formats","data-group","data-i","data-id","data-ids","data-imp-*","data-issue","data-item","data-job","data-key","data-kind","data-label","data-lib-*","data-linkrev*","data-locked-msg","data-max","data-min","data-mode","data-more","data-msg","data-name","data-now-qlist","data-now-steer","data-out","data-p","data-pack","data-period","data-pi","data-pic","data-pick","data-pick-again","data-pick-done","data-pick-made","data-pick-new","data-pl","data-place-at","data-placeholder","data-pos","data-q","data-qi","data-queue-index","data-range","data-ref","data-remote","data-rev-*","data-save-btn","data-save-songs","data-scan*","data-si","data-skin","data-skin-file","data-src-badge","data-src-mini","data-step","data-stepper","data-studio*","data-switch","data-sync","data-t","data-text","data-theme-file","data-toast-*","data-token","data-token-reset","data-track","data-track-id","data-uid","data-unit","data-url","data-via","data-wait-*","data-want-photo","data-webamp","data-week","data-weight","data-what","data-when","data-why-text","data-yes"]},"classes":["brand-mark","cls","cx-body","cx-err","disc-files","disc-only","disc-playlist","disc-results","dk-transparent","ic-play","info","lib-album__songs","lib-out-list","lib-partial","lib-send","none","sys-sounds"],"attrs":[],"prefixes":["a-","act-","b-","banner--","btn-gloss--","cls-tab-","ctx-tab-","disc-h-","disco-","f-","fallback-art-","i-","is-","lull-shell-","ly-","ly-back-","more-","my-","new-","now-ly--","now-tab-","op-","open-","own-","pad-","pl-tmp-","preview-","review-","row-","save-","sbtn-","scrobble--","sg--","studio-f-","studio-n-","sys-status--","sys-tab-","toast--","use-"]};

  /* hooks lull moved, newest first: each names what a rule for the old place looks like (its selector, and the
     properties that placed it, when only those stopped working), and what to use instead. docs/themes.md section 9
     lists the same. */
  var MOVED = [
    {
      since: '2026-11-04', name: 'Like and Save in the player bar',
      selector: /(\.pbar\b|#pbar\b)[^,]*\[data-act=(like|save)\]/, props: /^(grid-area|grid-column|grid-row|grid-column-(start|end)|grid-row-(start|end)|order)$/,
      say: 'Like and Save moved next to the song, into .pbar__acts, so they are no longer places of their own in the player bar’s grid: these rules don’t move them any more.',
      instead: 'To move both, place .pbar__acts. To restyle them, use .pbar__acts .btn-icon.'
    }
  ];

  var STATE = /^(is|has)-/;
  function block(c) { return STATE.test(c) ? c : c.replace(/(__|--).*$/, ''); }
  function listed(list, name) {
    return (list || []).some(function (n) { return n === name || (n.slice(-1) === '*' && name.indexOf(n.slice(0, -1)) === 0); });
  }
  function built(name) { return (HOOKS.prefixes || []).some(function (p) { return name.length > p.length && name.indexOf(p) === 0; }); }
  /* a selector with its quoted values written plain, its strings blanked, and what :not() holds left out (a rule with
     :not(.gone) still matches) */
  function plain(sel) {
    sel = sel.replace(/\[\s*([\w-]+)\s*([~|^$*]?=)\s*(['"])([^'"]*)\3\s*(i|s)?\s*\]/g, '[$1$2$4]').replace(/\s+/g, ' ').trim();
    for (var i = sel.indexOf(':not('); i >= 0; i = sel.indexOf(':not(', i + 1)) {
      var depth = 0, j = i + 4;
      for (; j < sel.length; j++) { if (sel[j] === '(') depth++; else if (sel[j] === ')' && --depth === 0) break; }
      sel = sel.slice(0, i) + sel.slice(j + 1);
      i = Math.max(-1, i - 1);
    }
    return sel;
  }
  function classes(sel) { return (sel.replace(/\[[^\]]*\]/g, '').match(/\.[a-zA-Z_][\w-]*/g) || []).map(function (c) { return c.slice(1); }); }
  function attrs(text) { return text.match(/data-[a-z][a-z0-9]*(?:-[a-z0-9]+)*/g) || []; }

  /* the names a theme's own slot templates put in the page: classes (class="", and cls="" on a part) and data-* */
  function own(slots) {
    var text = (slots || []).join('\n'), out = { classes: {}, attrs: {} };
    text.replace(/\b(?:class|cls)\s*=\s*(["'])([^"']*)\1/g, function (all, q, v) { v.split(/\s+/).forEach(function (c) { if (c) out.classes[c] = true; }); return all; });
    attrs(text).forEach(function (a) { out.attrs[a] = true; });
    return out;
  }

  function find(css, opts) {
    var mine = own(opts && opts.slots), byName = {}, out = [];
    var lines = [0];
    for (var i = 0; i < css.length; i++) if (css[i] === '\n') lines.push(i + 1);
    var lineOf = function (at) { var lo = 0, hi = lines.length - 1; while (lo < hi) { var mid = (lo + hi + 1) >> 1; if (lines[mid] <= at) lo = mid; else hi = mid - 1; } return lo + 1; };
    var note = function (kind, name, rule, extra) {
      var key = kind + ' ' + name, h = byName[key];
      if (!h) { h = byName[key] = Object.assign({ kind: kind, name: name, rules: [] }, extra || {}); out.push(h); }
      var line = lineOf(rule.at);
      if (!h.rules.some(function (r) { return r.line === line && r.selector === rule.selector; })) h.rules.push({ selector: rule.selector.replace(/\s+/g, ' '), line: line });
    };
    (root.LullLoops || require('./loops.js')).parse(css).rules.forEach(function (rule) {
      var props = rule.decls.map(function (d) { return d.name; });
      rule.selector.split(/,(?![^(]*\))/).forEach(function (part) {
        var sel = plain(part);
        MOVED.forEach(function (m) {
          if (m.selector.test(sel) && (!m.props || props.some(function (p) { return m.props.test(p); }))) {
            note('moved', m.name, rule, { say: m.say, instead: m.instead, since: m.since });
          }
        });
        classes(sel).forEach(function (c) {
          if (mine.classes[c]) return;
          var b = block(c);
          if (listed(HOOKS.stable && HOOKS.stable.classes, b)) return;
          if (listed(HOOKS.internal && HOOKS.internal.classes, b)) note('internal', '.' + b, rule);
          else if ((HOOKS.classes || []).indexOf(c) < 0 && !built(c)) note('gone', '.' + c, rule);
        });
        attrs(sel).forEach(function (a) {
          if (mine.attrs[a] || listed(HOOKS.stable && HOOKS.stable.attrs, a)) return;
          if (listed(HOOKS.internal && HOOKS.internal.attrs, a)) note('internal', a, rule);
          else if ((HOOKS.attrs || []).indexOf(a) < 0 && !built(a)) note('gone', a, rule);
        });
      });
    });
    var order = { moved: 0, gone: 1, internal: 2 };
    return out.sort(function (a, b) { return order[a.kind] - order[b.kind] || a.rules[0].line - b.rules[0].line; });
  }

  /* a hint in words someone who doesn't write CSS can follow: what it is, then what to do */
  function words(h) {
    if (h.kind === 'moved') return { what: h.say, then: h.instead };
    if (h.kind === 'gone') {
      return { what: 'Not in lull, or in this theme’s own slots: nothing on the page has it, so where a rule names it, that part does nothing.',
        then: 'Remove it, or name the part you meant: docs/themes.md section 11 lists the names lull keeps.' };
    }
    return { what: 'One of lull’s internal parts: it works today, and may be renamed or removed in any release.',
      then: 'Where you can, style a stable name from docs/themes.md section 11 instead.' };
  }

  var api = { find: find, words: words, MOVED: MOVED };
  root.LullHooks = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof self !== 'undefined' ? self : this);
