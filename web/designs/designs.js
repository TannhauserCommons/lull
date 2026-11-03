/* Themes: the five built-in designs and the listener's own themes, all theme packages (ADR-0004, docs/themes.md).
   Classic script, loaded in <head> right after <link id="design-css"> by index.html and the desktop pages.

   A package is designs/<id>/ (built in) or /api/themes/<id>/ (the listener's, stored on the hub):
     theme.json   manifest: id, name, author, api, extends, shell, swatches, modes, description, slots, coverVariety
     theme.css    a built-in design's CSS is the `design` cascade layer (it says so itself); a listener's theme is
                  imported into the `user` layer, after the design it extends
     slots/*.html slot templates: static markup with {{field}} placeholders and <lull-part> elements for live parts;
                  a listener's are sanitized with the vendored DOMPurify (purify.min.js, 3.4.17, loaded only for them)
     assets/      fonts and images, referenced relatively from theme.css
   A built-in's theme.json and slots also ship as one generated script, designs/<id>.js, which the pre-paint boot
   loads alongside the CSS: no request chain before first paint. The theme in use is per device ('lull:prefs'.design);
   ?theme=safe boots the default design and keeps it. The listener's manifests seen last are cached ('lull:themes'), so
   the boot knows their shell at once and asks the hub only for that theme's theme.json, to learn whether it was deleted;
   only Settings > Appearance asks the hub for the list (Designs.refresh). */
(function () {
  'use strict';
  var D = window.Designs = {};
  /* the product name: every shipped string (titles, brand link names, copy) reads it from here */
  D.product = 'lull';
  D.API = 1;                                     // the theme API this build speaks (docs/themes.md, versioning)
  D.builtIn = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
  D.fallback = 'lull';
  /* id -> { id, label, description, author, api, extends, shell, swatches, modes, preview, builtIn, slots, coverVariety }.
     The same objects for the life of the page (App.designs is this object), filled in from the manifests. */
  D.list = {};
  D.order = [];                                  // built-ins first, then the listener's themes by name
  D.current = null;

  var root = document.documentElement;
  var link = document.getElementById('design-css');
  /* designs/ folder relative to this page, read from the link the page ships with; the hub's API sits beside it */
  D.base = link ? link.getAttribute('href').replace(/[^/]+\/theme\.css$/, '') : 'designs/';
  var API = D.base + '../api/themes/';
  var safe = /(?:^|[#?&])theme=safe(?:&|$)/.test(location.hash + '&' + location.search);
  D.safe = safe;

  function store(key, value) {
    try {
      if (arguments.length < 2) return JSON.parse(localStorage.getItem('lull:' + key));
      localStorage.setItem('lull:' + key, JSON.stringify(value));
    } catch (e) { return null; }
    return null;
  }

  /* ---------- manifests ---------- */
  D.packageUrl = function (id) {
    return D.builtIn.indexOf(id) >= 0 ? D.base + id + '/' : API + encodeURIComponent(id) + '/';
  };
  /* one manifest (theme.json as served) into its D.list entry; unknown or broken ones are left out */
  function absorb(raw, builtIn) {
    if (!raw || typeof raw.id !== 'string' || !/^[a-z0-9][a-z0-9-]{0,39}$/.test(raw.id)) return null;
    if (!builtIn && D.builtIn.indexOf(raw.id) >= 0) return null;
    var m = D.list[raw.id] || (D.list[raw.id] = {});
    m.id = raw.id; m.builtIn = !!builtIn;
    m.label = raw.name || raw.id; m.name = m.label;
    m.description = raw.description || ''; m.author = raw.author || '';
    m.api = raw.api || 1; m.extends = raw.extends || null;
    m.shell = /^(rail|topnav|dock)$/.test(raw.shell) ? raw.shell : 'rail';
    m.swatches = Array.isArray(raw.swatches) && raw.swatches.length === 4 ? raw.swatches : ['#e4def9', '#f7f4ff', '#445796', '#2a0809'];
    m.modes = Array.isArray(raw.modes) && raw.modes.length ? raw.modes : ['day', 'night'];
    m.preview = !!raw.preview; m.slots = Array.isArray(raw.slots) ? raw.slots : [];
    m.coverVariety = !!raw.coverVariety;
    return m;
  }
  function sortOrder() {
    var mine = Object.keys(D.list).filter(function (id) { return !D.list[id].builtIn; })
      .sort(function (a, b) { return D.list[a].label.localeCompare(D.list[b].label); });
    D.order.splice.apply(D.order, [0, D.order.length].concat(D.builtIn, mine));
  }
  function cacheList() {
    var mine = {};
    Object.keys(D.list).forEach(function (id) { var m = D.list[id]; if (!m.builtIn && m.shell) mine[id] = raw(m); });
    store('themes', mine);
  }
  function raw(m) {
    return { id: m.id, name: m.label, description: m.description, author: m.author, api: m.api, extends: m.extends,
      shell: m.shell, swatches: m.swatches, modes: m.modes, preview: m.preview, slots: m.slots, coverVariety: m.coverVariety };
  }
  /* the built-ins are always known by name (each manifest arrives with its designs/<id>.js) */
  D.builtIn.forEach(function (id) { D.list[id] = { id: id, builtIn: true, label: id, swatches: [], slots: [] }; });
  var cached = store('themes') || {};
  Object.keys(cached).forEach(function (id) { absorb(cached[id], false); });
  sortOrder();

  /* designs/<id>.js calls this with the built-in's theme.json and its slot templates (trusted: they ship with lull) */
  var waiting = {};                        // built-in id -> resolve, while its script loads
  D.add = function (manifest, slots) {
    var m = absorb(manifest, true);
    if (!m) return;
    templates[m.id] = slots; trusted[m.id] = true;
    sortOrder();
    if (waiting[m.id]) waiting[m.id]();
  };
  var scripts = {};
  function builtInScript(id, blocking) {
    return scripts[id] || (scripts[id] = new Promise(function (ok, fail) {
      waiting[id] = ok;
      var s = document.createElement('script');
      s.src = D.base + id + '.js';
      if (blocking) s.setAttribute('blocking', 'render');     // no first paint without the shell (Chromium; else a frame)
      s.onerror = function () { delete scripts[id]; fail(new Error(s.src + ' failed to load')); };
      document.head.appendChild(s);
    }));
  }

  function getJson(url) { return get(url).then(function (r) { return r.json(); }); }
  function getText(url) { return get(url).then(function (r) { return r.text(); }); }
  function get(url) {
    return fetch(url, { credentials: 'same-origin' }).then(function (r) {
      if (!r.ok) { var e = new Error(url + ': ' + r.status); e.status = r.status; throw e; }
      return r;
    });
  }
  function manifest(id) {
    var m = D.list[id];
    if (D.builtIn.indexOf(id) >= 0) return templates[id] ? Promise.resolve(m) : builtInScript(id).then(function () { return D.list[id]; });
    if (m && m.shell) return Promise.resolve(m);
    return getJson(D.packageUrl(id) + 'theme.json').then(function (j) {
      var got = absorb(j, false);
      if (!got) throw new Error('Invalid theme.json for ' + id);
      sortOrder(); cacheList();
      return got;
    });
  }
  /* GET /api/themes: every manifest (the built-ins plus the listener's), for Settings > Appearance. Resolves to D.list. */
  D.refresh = function () {
    return getJson(API.replace(/\/$/, '')).then(function (j) {
      var seen = {};
      (j.items || []).forEach(function (it) { var m = absorb(it, !!it.builtIn); if (m) seen[m.id] = true; });
      Object.keys(D.list).forEach(function (id) { if (!D.list[id].builtIn && !seen[id]) delete D.list[id]; });
      sortOrder(); cacheList();
      return D.list;
    });
  };

  D.valid = function (id) { return Object.prototype.hasOwnProperty.call(D.list, id); };
  /* the theme to show: design= in the hash (#/route?design=x or #view=x&design=x) or query, else the stored pref, else the
     fallback. ?theme=safe: the fallback, whatever is stored. */
  D.wanted = function () {
    if (safe) return D.fallback;
    var m = /(?:^|[#?&])design=([a-z0-9-]+)/.exec(location.hash + '&' + location.search);
    if (m && known(m[1])) return m[1];
    var p = store('prefs');
    return p && known(p.design) ? p.design : D.fallback;
  };
  /* a theme this page knows, or a stored one it can still ask the hub for (a first visit has no cache yet) */
  function known(id) { return D.valid(id) || (typeof id === 'string' && /^[a-z0-9][a-z0-9-]{0,39}$/.test(id)); }

  /* ---------- theme: a stored 'day' | 'night' | 'system' always becomes an explicit data-theme ----------
     'system' is resolved from the OS (prefers-color-scheme) here and again whenever the OS flips, so CSS only
     ever sees :root[data-theme="day"] or [data-theme="night"] and needs no prefers-color-scheme copy.
     A theme with a single mode keeps that mode. :root[data-theme-pref] keeps the choice. */
  /* A design may animate a flip (lull's dusk) under :root[data-theme-fade]. It is set only for a flip the listener can
     watch: after the page has loaded and while it is visible. A page loading into its theme, or one the OS flips in the
     background, cuts straight to it, and the attribute's removal ends a fade a hidden page never ran. */
  var themePref = 'system', fadeTimer = 0, settled = false;
  var darkOs = window.matchMedia ? window.matchMedia('(prefers-color-scheme: dark)') : null;
  window.addEventListener('load', function () { setTimeout(function () { settled = true; }, 1000); });
  D.applyTheme = function (pref) {
    themePref = pref === 'day' || pref === 'night' ? pref : 'system';
    var t = themePref !== 'system' ? themePref : darkOs && darkOs.matches ? 'night' : 'day';
    var modes = D.current && D.list[D.current] && D.list[D.current].modes;
    if (modes && modes.length === 1) t = modes[0];
    if (settled && document.visibilityState === 'visible' && root.getAttribute('data-theme') !== t) {
      root.setAttribute('data-theme-fade', '');
      clearTimeout(fadeTimer);
      fadeTimer = setTimeout(function () { root.removeAttribute('data-theme-fade'); }, 1400);
    }
    root.setAttribute('data-theme', t);
    root.setAttribute('data-theme-pref', themePref);
    return t;
  };
  if (darkOs) {
    var onOs = function () { if (themePref === 'system') D.applyTheme('system'); };
    if (darkOs.addEventListener) darkOs.addEventListener('change', onOs); else if (darkOs.addListener) darkOs.addListener(onOs);
  }

  /* default brand markup (the `brand` slot's fallback): .brand holds the visible name, .brand__orn an optional
     ornament (the sparkle). Decorative: the link or element around it carries the accessible name (D.product). */
  D.brandMarkup = function () {
    return '<span class="wordmark brand" aria-hidden="true"><span class="brand__name">' + D.product + '</span>' +
      '<span class="brand__orn wordmark__spark"></span></span>';
  };

  /* ---------- slots ----------
     Designs.slot(name, ctx, fallback) -> the theme's template for that slot, rendered with ctx, else fallback(ctx).
     A template is looked up as slots/<name>.<variant>.html, then slots/<name>.html, in the theme and then in the design
     it extends. An empty template renders nothing. coverArt has no template: the manifest's coverVariety adds the
     per-item hooks to the default cover instead (it runs for every cover, so it stays a string edit). */
  var templates = {};                      // theme id -> { 'nowHero.full': '<markup>' }
  var trusted = {};                        // theme id -> true for the built-ins (no sanitizer needed)
  D.slot = function (name, ctx, fallback) {
    ctx = ctx || {};
    if (name === 'coverArt') return coverArt(ctx, fallback);
    var tpl = template(name, VARIANT[name] ? VARIANT[name](ctx) : '');
    if (tpl == null) return fallback(ctx);
    try { return renderSlot(name, tpl.text, ctx, tpl.trusted); } catch (e) { console.error('[theme slot ' + name + ']', e); }
    return fallback(ctx);
  };
  var VARIANT = {
    nowHero: function (c) { return c.variant || 'full'; },
    homeLead: function (c) { return c.track ? 'playing' : 'idle'; },
    miniPlayer: function (c) { return c.style || 'device'; }
  };
  function chain(id) {
    var out = [], m = D.list[id];
    while (m && out.indexOf(m.id) < 0) { out.push(m.id); m = m.extends ? D.list[m.extends] : null; }
    return out;
  }
  function template(name, variant) {
    var ids = chain(D.current);
    for (var i = 0; i < ids.length; i++) {
      var t = templates[ids[i]] || {}, k = variant && t[name + '.' + variant] != null ? name + '.' + variant : name;
      if (t[k] != null) return { text: t[k], trusted: !!trusted[ids[i]] };
    }
    return null;
  }
  function loadTemplates(id) {
    if (templates[id]) return Promise.resolve();
    var m = D.list[id], got = {};
    return Promise.all((m.slots || []).map(function (s) {
      return getText(D.packageUrl(id) + 'slots/' + s + '.html').then(function (t) { got[s] = t; });
    })).then(function () { templates[id] = got; });
  }

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; });
  }
  /* sanitize: no scripts, event handlers, forms, embeds, <use>, and links only inside the app (#/...). DOMPurify checks
     every attribute value against ALLOWED_URI_REGEXP; the hook keeps that to links, so data-title="Error: x" stays. */
  var PURIFY = {
    RETURN_DOM: true, ADD_TAGS: ['lull-part'], ADD_ATTR: ['pick', 'inner', 'size', 'bars', 'icon', 'cls', 'label', 'focusable'],
    FORBID_TAGS: ['style', 'form', 'input', 'textarea', 'select', 'iframe', 'object', 'embed', 'foreignobject'],
    FORBID_ATTR: ['src', 'srcset', 'action', 'formaction'],
    ALLOWED_URI_REGEXP: /^#/
  };
  var hooked = false;
  D.sanitize = function (markup) {
    var P = window.DOMPurify;
    if (!hooked) {
      hooked = true;
      P.addHook('uponSanitizeAttribute', function (node, a) {
        var name = a.attrName;
        if (name === 'href' || name === 'xlink:href' || /^on/.test(name) || PURIFY.FORBID_ATTR.indexOf(name) >= 0) return;
        if (a.allowedAttributes[name] || /^data-[a-z0-9-]+$/.test(name)) a.forceKeepAttr = true;
      });
    }
    return P.sanitize(markup, PURIFY);
  };
  function parsed(markup) { var b = document.createElement('div'); b.innerHTML = markup; return b; }
  /* fields fill {{name}} (always escaped, so a song title can never become markup); then the sanitizer; then every
     <lull-part name="..."> becomes the live markup the app hands the slot (ctx.parts), which is the app's own */
  function renderSlot(name, tpl, ctx, isTrusted) {
    tpl = tpl.replace(/<!--[\s\S]*?-->/g, '').replace(/>\s*\n\s*</g, '><').trim();   // indent freely: line breaks between tags go
    if (!tpl) return '';
    var f = FIELDS[name] ? FIELDS[name](ctx) : {};
    f.product = D.product;
    var filled = tpl.replace(/\{\{\s*([a-zA-Z][a-zA-Z0-9]*)\s*\}\}/g, function (all, k) {
      if (!Object.prototype.hasOwnProperty.call(f, k)) { console.warn('[theme slot ' + name + '] unknown field', k); return ''; }
      return esc(f[k]);
    });
    var body = isTrusted ? parsed(filled) : D.sanitize(filled);
    var parts = body.querySelectorAll('lull-part'), made = {};
    for (var i = 0; i < parts.length; i++) parts[i].outerHTML = part(name, parts[i], ctx, made);
    return body.innerHTML;
  }
  /* made: what this render already built, so ten picks from one device build it once */
  function part(slot, el, ctx, made) {
    var name = el.getAttribute('name'), make = PARTS[name];
    if (!make) { console.warn('[theme slot ' + slot + '] unknown part', name); return ''; }
    var key = [name, el.getAttribute('size'), el.getAttribute('bars'), el.getAttribute('icon'), el.getAttribute('label')].join('|');
    var out = made[key] != null ? made[key] : (made[key] = String(make(ctx, el, slot) || ''));
    var pick = el.getAttribute('pick');
    if (pick) {
      var host = document.createElement('div'); host.innerHTML = out;
      var n = host.querySelector(pick);
      out = !n ? '' : el.hasAttribute('inner') ? n.innerHTML : n.outerHTML;
    }
    var cls = el.getAttribute('cls');
    if (cls && out) out = out.replace(/^(<[a-z]+[^>]*?\sclass=")/, '$1' + esc(cls) + ' ');
    return out;
  }
  function ic(name, cls) { return window.icon ? String(window.icon(name, null, cls)) : ''; }
  function num(el, attr, dflt) { var v = parseFloat(el.getAttribute(attr)); return isNaN(v) ? dflt : v; }
  function size(el, dflt) { var v = el.getAttribute('size'); return !v ? dflt : /^\d+$/.test(v) ? +v : v; }
  function isOn(v) { return v ? 'true' : 'false'; }

  /* ---------- fields: plain strings for {{...}}, per slot (docs/themes.md lists them) ---------- */
  var MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december'];
  var DAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'];
  function trackFields(t) {
    return { title: t ? t.title : '', artist: t ? t.artist : '', album: t ? t.album || '' : '', trackId: t ? t.id : '' };
  }
  function playerFields(p, t) {
    p = p || {};
    var rep = p.repeat || 'off', liked = !!(t && window.App && App.isLiked(t.id));
    return {
      playing: isOn(p.playing), shuffle: isOn(p.shuffle), repeat: rep,
      repeatOn: isOn(rep !== 'off'), repeatLabel: { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[rep] || 'Repeat off',
      repeatIcon: rep === 'one' ? 'repeat-one' : 'repeat', status: p.status || 'idle',
      liked: isOn(liked), likeClass: liked ? 'is-liked' : '', likeIcon: liked ? 'heart-fill' : 'heart',
      error: p.status === 'failed' && p.error ? p.error : 'Couldn\'t play this song',
      radio: p.radio ? p.radio.name : '', from: playingFrom()
    };
  }
  /* the name of the place the song plays from ("Liked songs", "Night drive"), as the player bar says it */
  function playingFrom() { var f = window.App && App.player && App.player.playingFrom(); return f ? App.placeName(f) : ''; }
  var FIELDS = {
    brand: function (c) { return { where: c.where || '' }; },
    /* noun: what is missing, from a "No liked songs yet" title ("liked songs"); empty for other titles */
    emptyArt: function (c) {
      var m = /^no ([a-z ]+?)(?: yet| here| right now)?$/i.exec(String(c.title || ''));
      return { art: /^(moon|slot|star|wave)$/.test(c.art) ? c.art : 'moon', title: c.title || '', noun: m ? m[1].toLowerCase() : '' };
    },
    homeLead: function (c) {
      var t = c.track, d = new Date(), q = t ? [t].concat(c.queue || []).slice(0, 8) : [], F = window.App && App.fmt;
      var secs = 0, mb = 0;
      q.forEach(function (x) { secs += x.duration || 0; mb += x.sizeMB || 0; });
      var line = t && c.parts.formatLine ? String(c.parts.formatLine(t)) : '';
      var week = weekStats();
      return Object.assign(trackFields(t), playerFields(c.player, t), {
        format: line, codec: line.split(',')[0],
        date: String(d.getDate()), month: MONTHS[d.getMonth()], weekday: DAYS[d.getDay()],
        queueCount: String(q.length), queueDuration: F ? F.clock(secs) : '', queueSize: F ? F.size(mb) : '',
        next: q[1] ? q[1].title : 'end of queue',
        week: isOn(week), weekTime: week ? Math.floor(week.minutes / 60) + ' h ' + week.minutes % 60 + ' min' : '',
        weekPlays: week ? String(week.plays) : '', weekArtists: week ? String(week.artists) : '',
        weekPeak: week ? String(week.clock.indexOf(Math.max.apply(null, week.clock))) : ''
      });
    },
    nowHero: function (c) { return Object.assign(trackFields(c.track), playerFields(c.player, c.track), { variant: c.variant || 'full' }); },
    miniPlayer: function (c) {
      var t = c.track || {};
      return Object.assign(trackFields(c.track), {
        format: t.format || '', quality: t.lossless ? 'll' : 'ly', playing: isOn(c.playing), liked: isOn(c.liked),
        playLabel: c.playing ? 'Pause' : 'Play', playIcon: c.playing ? 'pause' : 'play', playIconClass: c.playing ? '' : 'ic-play',
        likeIcon: c.liked ? 'heart-fill' : 'heart', likeClass: c.liked ? 'is-liked' : '', opaqueClass: c.opaque ? 'dk-mini--opaque' : ''
      });
    }
  };
  /* the 7-day listening stats the Stats page already read (none until it has) */
  function weekStats() {
    var w = window.App && App.data && App.data.stats && App.data.stats.byPeriod && App.data.stats.byPeriod['7d'];
    return w && w.clock && w.clock.length ? w : null;
  }

  /* ---------- parts: live markup from the app, by <lull-part name> (docs/themes.md lists them) ----------
     Any part takes pick="selector" (keep one element of it; with `inner`, only its content) and cls="classes" (added
     to its root element). */
  function hasPart(c, k) { return c.parts && typeof c.parts[k] === 'function'; }
  var PARTS = {
    icon: function (c, el) { return ic(el.getAttribute('icon') || 'sparkle', el.getAttribute('size') === '16' ? 'ic-16' : ''); },   // size="16": .ic-16
    device: function (c) { return c.parts.device(); },
    compact: function (c) { return c.parts.compact(); },
    strip: function (c) { return c.parts.strip(); },
    screen: function (c) { return c.parts.screen(); },
    facts: function (c) { return c.parts.facts(); },
    state: function (c) { return esc(c.parts.state()); },
    wave: function (c, el) { return c.parts.wave(el.hasAttribute('bars') ? num(el, 'bars', 26) : undefined); },
    'play-button': function (c, el) { return c.parts.playButton(num(el, 'size', 56)); },
    /* nowHero: the gloss like key (size); homeLead: the icon like button for the playing song */
    'like-button': function (c, el, slot) {
      return slot === 'homeLead' ? c.parts.likeButton(c.track) : c.parts.likeButton(num(el, 'size', 52));
    },
    /* the song's cover (an empty slot when nothing plays); homeLead and miniPlayer hand their own cover part */
    cover: function (c, el, slot) {
      if (slot === 'miniPlayer') return c.parts.cover(num(el, 'size', 84));
      if (hasPart(c, 'cover')) return c.track ? c.parts.cover(c.track, size(el, 64)) : '';
      return c.track && window.App ? App.ui.cover(c.track, size(el, 56), { hero: true }) : '<span class="now-dev__slot" aria-hidden="true"></span>';
    },
    seek: function () { return window.App ? App.ui.seekBar({ bind: 'player', label: 'Seek' }) : ''; },
    /* where the song plays from: the link Now Playing shows (App.ui.playingFrom), refreshed in place by now.js */
    'playing-from': function (c) {
      var from = c.track && window.App ? String(App.ui.playingFrom('', 'playing from')) : '';
      return from || ic('sparkle', 'ic-16') + '<span>nothing queued</span>';
    },
    /* the playing song and up next, as a dense track list (homeLead) */
    'track-list': function (c, el) {
      var list = [c.track].concat(c.queue || []).slice(0, 8);
      return c.parts.trackList(list, { label: el.getAttribute('label') || 'Up next', showSources: false });
    },
    /* listening by hour of day this week: 24 <i style="--v:0..1">, the busiest hour .is-peak (homeLead) */
    'week-clock': function () {
      var w = weekStats(); if (!w) return '';
      var max = Math.max.apply(null, w.clock) || 1, peak = w.clock.indexOf(max);
      return w.clock.map(function (v, h) {
        return '<i style="--v:' + (Math.round(v / max * 100) / 100) + '"' + (h === peak ? ' class="is-peak"' : '') + '></i>';
      }).join('');
    },
    /* the queue as a constellation: the playing song and up to six next ones as .sky-star (left / top in %, --i), joined
       by .sky-line, a polyline in a 0-100 box; each star names its song in a hover title (homeLead) */
    'queue-sky': function (c) {
      if (!c.track) return '';
      var songs = [c.track].concat(c.queue || []).slice(0, 7), n = songs.length, pts = [];
      songs.forEach(function (s, i) {
        pts.push({ s: s, x: n > 1 ? 3 + i * 94 / (n - 1) : 50, y: 18 + (i % 2 ? 40 : 0) + fnv(String(s.id)) % 24 });
      });
      return '<svg class="sky-line" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true"><polyline points="' +
        pts.map(function (p) { return p.x + ',' + p.y; }).join(' ') + '" vector-effect="non-scaling-stroke"/></svg>' +
        pts.map(function (p, i) {
          var tip = (i ? 'up next: ' : 'playing: ') + p.s.title + ' by ' + p.s.artist;
          return '<span class="sky-star' + (i ? '' : ' sky-star--now') + '" style="left:' + p.x + '%;top:' + p.y + '%;--i:' + i + '" title="' + esc(tip) + '"></span>';
        }).join('');
    }
  };

  /* ---------- covers: coverVariety gives every cover stable per-item hooks for CSS ----------
     data-look="0".."5" (pick one of n looks with data-look % n: 2, 3 and 6 divide evenly) and --cover-flip (1 | -1),
     --cover-hue (-28deg..28deg), --cover-zoom (1 | 1.25 | 1.45), --cover-dx / --cover-dy (a crop offset that stays in
     the zoomed art). Seeded from the cover key, so an album looks the same on every screen. */
  function fnv(s) {
    var h = 2166136261;
    for (var i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
    return h >>> 0;
  }
  function look(s) {
    var h = 0;
    for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
    return ((h % 6) + 6) % 6;
  }
  function coverArt(ctx, fallback) {
    var out = String(fallback(ctx)), m = D.list[D.current];
    if (!m || !m.coverVariety || !ctx.key) return out;
    var key = String(ctx.key), h = fnv(key), z = [1, 1.25, 1.45][(h >>> 4) % 3];
    function shift(bits) { return Math.round((((h >>> bits) % 3) - 1) * (z - 1) * 50) + '%'; }
    var vars = '--cover-flip:' + ((h & 1) ? -1 : 1) + ';--cover-hue:' + (((h >>> 1) % 5) - 2) * 14 + 'deg;--cover-zoom:' + z +
      ';--cover-dx:' + shift(6) + ';--cover-dy:' + shift(8);
    out = out.indexOf('style="') > 0 ? out.replace('style="', 'style="' + vars + ';') : out.replace('<span ', '<span style="' + vars + '" ');
    return out.replace('<span ', '<span data-look="' + look(key) + '" ');
  }

  /* ---------- loading ---------- */
  var userCss = null;                       // <style id="user-css">: @import of the listener's theme.css, layer user
  var stamp = {};                           // theme id -> cache buster, bumped on theme.changed
  function designOf(id) {
    var m = D.list[id];
    if (!m) return null;
    if (m.builtIn) return id;
    return m.extends && D.list[m.extends] && D.list[m.extends].builtIn ? m.extends : null;
  }
  function userImport(id) {
    var m = D.list[id];
    return m && !m.builtIn ? '@import url("' + D.packageUrl(id) + 'theme.css?v=' + (stamp[id] || 0) + '") layer(user);' : '';
  }
  function setUserCss(id) {
    var css = userImport(id);
    if (!css) { if (userCss) { userCss.remove(); userCss = null; } return; }
    if (!userCss) {
      userCss = document.createElement('style');
      userCss.id = 'user-css';
      userCss.setAttribute('blocking', 'render');
      (link && link.parentNode ? link.parentNode.insertBefore(userCss, link.nextSibling) : document.head.appendChild(userCss));
    }
    if (userCss.textContent !== css) userCss.textContent = css;
  }
  function applyAttrs(id) {
    var m = D.list[id];
    D.current = id;
    root.setAttribute('data-design', designOf(id) || id);
    root.setAttribute('data-shell', m.shell || 'rail');
    if (m.modes && m.modes.length === 1) D.applyTheme(themePref);
  }
  /* everything a theme needs before the app renders with it: its manifest (and its design's), DOMPurify, slot templates */
  var purify = null;
  function loadPurify() {
    if (window.DOMPurify) return Promise.resolve();
    return purify || (purify = new Promise(function (ok, fail) {
      var s = document.createElement('script');
      s.src = D.base + 'purify.min.js';
      s.onload = ok; s.onerror = function () { fail(new Error('purify.min.js failed to load')); };
      document.head.appendChild(s);
    }));
  }
  function prepare(id) {
    return manifest(id).then(function (m) {
      return (m.extends && D.list[m.extends] ? manifest(m.extends) : Promise.resolve()).then(function () {
        var ids = chain(id), untrusted = ids.some(function (x) { return !trusted[x] && D.list[x].slots.length; });
        return (untrusted ? loadPurify() : Promise.resolve()).then(function () { return Promise.all(ids.map(loadTemplates)); });
      });
    });
  }

  /* before first paint (inline script in <head>): the design's stylesheet and its designs/<id>.js start loading side by
     side (the script blocks rendering like the stylesheet, so the shell is right on the first frame). A listener's theme
     uses its cached manifest; on its first visit the page stays hidden until it has one (base.css, :root[data-booting]). */
  var booting = null;
  D.boot = function () {
    if (safe) pick(D.fallback);                     // so the next normal load doesn't bring the broken theme back
    var id = D.wanted(), m = D.list[id], design = designOf(id);
    D.current = id;
    if (design) { root.setAttribute('data-design', design); if (link) link.setAttribute('href', D.base + design + '/theme.css'); }
    if (design) builtInScript(design, true);
    if (m && m.shell) applyAttrs(id);
    else if (!m || !m.builtIn) root.setAttribute('data-booting', '');
    if (link && !design && m && m.shell) link.disabled = true;
    setUserCss(id);
    if (m && !m.builtIn && m.shell) getJson(D.packageUrl(id) + 'theme.json').catch(function (e) { if (e.status === 404) forget(id); });
    booting = prepare(id).then(function () {
      applyAttrs(id); setUserCss(id);
      if (link) { var d = designOf(id); link.disabled = !d; if (d) link.setAttribute('href', D.base + d + '/theme.css'); }
    }, function (e) {
      console.error('Theme ' + id + ' failed to load, using ' + D.fallback, e);
      if (e && e.status === 404) forget(id);
      return prepare(D.fallback).then(function () { applyAttrs(D.fallback); setUserCss(D.fallback); if (link) { link.disabled = false; link.setAttribute('href', D.base + D.fallback + '/theme.css'); } });
    }).then(function () { root.removeAttribute('data-booting'); });
  };
  /* this device's theme ('lull:prefs'.design); app.js keeps the rest of the prefs */
  function pick(id) {
    var p = store('prefs') || {};
    if (p.design !== id) { p.design = id; store('prefs', p); }
  }
  /* the hub says a cached theme is gone (deleted from another device): the next load neither shows it nor asks for it */
  function forget(id) {
    var c = store('themes') || {};
    if (c[id]) { delete c[id]; store('themes', c); }
    if ((store('prefs') || {}).design === id) pick(D.fallback);
  }
  /* run fn once the current theme's slots and manifest are ready */
  D.ready = function (fn) { (booting || prepare(D.current || D.wanted())).then(fn, fn); };

  /* switch without a reload: the new design stylesheet loads next to the old one, then they swap, so there is no
     unstyled frame. onDone runs after the attributes changed (re-render there). If anything fails to load, the current
     theme stays and onFail(id) runs. */
  D.set = function (id, onDone, onFail) {
    if (!D.valid(id)) { console.warn('Unknown theme', id); return false; }
    if (id === D.current) { if (onDone) onDone(); return true; }
    prepare(id).then(function () {
      var design = designOf(id), cur = link && !link.disabled ? link.getAttribute('href') : null;
      var want = design ? D.base + design + '/theme.css' : null;
      return swapDesignCss(cur, want).then(function () { setUserCss(id); applyAttrs(id); if (onDone) onDone(); });
    }).catch(function (e) {
      console.error('Theme ' + id + ' failed to load', e);
      if (onFail) onFail(id);
    });
    return true;
  };
  function swapDesignCss(cur, want) {
    if (!link) return Promise.resolve();
    if (!want) { link.disabled = true; return Promise.resolve(); }
    if (cur === want) { link.disabled = false; return Promise.resolve(); }
    return new Promise(function (ok, fail) {
      var next = document.createElement('link');
      next.rel = 'stylesheet'; next.href = want;
      next.onload = function () { var old = link; link = next; old.remove(); next.id = 'design-css'; ok(); };
      next.onerror = function () { next.remove(); fail(new Error('Design stylesheet failed to load: ' + want)); };
      link.parentNode.insertBefore(next, link.nextSibling);
    });
  }

  /* a stored theme's files changed on the hub (event theme.changed): reload its manifest, CSS and slots if it is in use.
     Resolves true when the page must re-render (slots or shell may differ). */
  D.reload = function (id) {
    if (!D.valid(id) || D.list[id].builtIn) return Promise.resolve(false);
    stamp[id] = Date.now();
    delete templates[id];
    var inUse = chain(D.current).indexOf(id) >= 0;
    D.list[id].shell = null;                        // manifest() fetches it again
    return manifest(id).then(function () {
      if (!inUse) return false;
      return prepare(D.current).then(function () { setUserCss(D.current); applyAttrs(D.current); return true; });
    });
  };
})();
