/* parts.js: a theme's markup at every region's and screen's edge, and lull's catalogue of parts for any slot (ADR-0101;
   docs/themes.md section 6). A theme pays only for what it uses: designs.js loads this file only for a design or theme
   whose slots include a region or screen slot (slots/before.<name>.html, after.<name>.html) or name a part only the
   catalogue draws (designs.js wantsParts). The built-ins use neither, so they never fetch it, and app.js's hooks for it
   (App.edges) do nothing. Runs in lull's page only: the desktop app's mini windows have no App, and their slots keep the
   parts their slot hands. It can load before app.js has run (with the theme, in the page's head): then it hooks in once
   the page has. */
(function () {
  'use strict';
  function hook() {
    if (!window.App || !App.on || App.parts) return;
    var P = App.player, ps = P.state, ui = App.ui, html = App.html, icon = App.icon, $ = App.$, $$ = App.$$, patch = App.patch;
    function empty() { return ''; }

    /* ---------- edges: the theme's markup before and after any region, any screen, the whole shell ----------
       slots/before.<name>.html and after.<name>.html, drawn in a .region-slot[data-slot] box (keyed slot:<name>):
       - a region, any element lull draws with [data-region="<name>"] (the shell's rail, header, player bar...; Home's
         shelves, Search's results...): the box is its first or last child, drawn with the shell, with the screen that
         holds it, and kept when lull draws the region again (App.patch keeps a theme's boxes: app.js patchChildren);
       - a screen (SCREENS, as .app[data-screen] names it): inside its own top container (SLOTS_IN: Now Playing's grid;
         else the first element it draws), so the box is an item of the screen's grid;
       - `app`, the whole shell: the box is #app's sibling before or after it, outside its grid; the theme's CSS makes
         room for it (lull sets no layout for any box).
       The hub takes the names lull draws (crates/themes package.rs EDGES, held to this file and to the markup's regions
       by its tests). A theme with none draws nothing here. */
    var SCREENS = ['home', 'search', 'now', 'library', 'liked', 'downloads', 'history', 'following', 'album', 'artist', 'tag',
      'playlists', 'playlist', 'outside', 'stats', 'settings'];
    var SLOTS_IN = { now: '.now__grid' };   // where a screen's slots go when it isn't the first element it draws
    /* the edge slots of the design or theme in use, with what it extends */
    function edgeSlots() {
      var out = [], seen = {}, m = Designs.list[Designs.current];
      while (m && !seen[m.id]) {
        seen[m.id] = 1;
        (m.slots || []).forEach(function (n) { if (/^(before|after)\./.test(n) && out.indexOf(n) < 0) out.push(n); });
        m = m.extends ? Designs.list[m.extends] : null;
      }
      return out;
    }
    function edgeOf(slot) { var i = slot.indexOf('.'); return [slot.slice(0, i), slot.slice(i + 1)]; }
    function makeBox(slot, inner) {
      var box = document.createElement('div');
      box.className = 'region-slot'; box.setAttribute('data-slot', slot); box.setAttribute('data-key', 'slot:' + slot);
      box.innerHTML = inner;
      return box;
    }
    /* a box at an element's edge: drawn when missing, its markup brought up to date when there */
    function place(host, slot, before, markup) {
      var box = null;
      for (var n = host.firstElementChild; n; n = n.nextElementSibling) if (n.classList.contains('region-slot') && n.getAttribute('data-slot') === slot) box = n;
      if (box) { patch(box, markup); return; }
      host.insertBefore(makeBox(slot, markup), before ? host.firstChild : null);
    }
    /* the theme's boxes at every region's edges within `root` (and root itself), by the slots it has */
    function within(root, slots) {
      (slots || edgeSlots()).forEach(function (slot) {
        var e = edgeOf(slot), name = e[1];
        if (name === 'app' || SCREENS.indexOf(name) >= 0) return;
        var hosts = Array.prototype.slice.call(root.querySelectorAll('[data-region="' + name + '"]'));
        if (root.matches && root.matches('[data-region="' + name + '"]')) hosts.unshift(root);
        if (!hosts.length) return;
        var markup = String(App.slot(slot, {}, empty));
        hosts.forEach(function (h) { if (!h.closest('.region-slot')) place(h, slot, e[0] === 'before', markup); });
      });
    }
    /* with the shell (a design or theme switch: app.js renderShell): the boxes the theme in use no longer has go, the
       shell's own edges are drawn, and every region's on the page */
    function regions() {
      var slots = edgeSlots(), app = $('#app');
      $$('.region-slot').forEach(function (b) { if (slots.indexOf(b.getAttribute('data-slot')) < 0 && !b.parentNode.closest('.region-slot')) b.remove(); });
      ['before', 'after'].forEach(function (edge) {
        var slot = edge + '.app', box = document.querySelector('body > .region-slot[data-slot="' + slot + '"]');
        if (slots.indexOf(slot) < 0 || !app) return;
        var markup = String(App.slot(slot, {}, empty));
        if (box) patch(box, markup);
        else app.parentNode.insertBefore(makeBox(slot, markup), edge === 'before' ? app : app.nextSibling);
      });
      if (app) within(app, slots);
    }
    /* a screen's markup as it is drawn (app.js renderRoute): its own slots in its top container, and every region's
       within it. Returns the markup as it was when the theme has none for this screen */
    function screen(markup, name) {
      var slots = edgeSlots();
      if (!slots.length) return markup;
      var tpl = document.createElement('template');
      tpl.innerHTML = String(markup || '');
      if (SCREENS.indexOf(name) >= 0) {
        var host = SLOTS_IN[name] ? tpl.content.querySelector(SLOTS_IN[name]) : tpl.content.firstElementChild;
        ['before.' + name, 'after.' + name].forEach(function (slot) {
          var inner = slots.indexOf(slot) >= 0 && host ? String(App.slot(slot, {}, empty)) : '';
          if (inner) host.insertBefore(makeBox(slot, inner), slot.indexOf('before.') === 0 ? host.firstChild : null);
        });
      }
      within(tpl.content, slots);
      return tpl.innerHTML;
    }
    App.edges = { regions: regions, screen: screen, within: within };

    /* ---------- the catalogue ----------
       Every <lull-part name> a slot of lull's page names that its slot doesn't hand itself (homeLead's and nowHero's
       own, section 6; designs.js catalogued) is drawn from here into the <lull-part> element, which stays as its box
       (display: contents). Each is lull's own markup, wired as lull's own copy is, and kept current the same way: the
       keys, Like, Save, the seek and the volume by their data-bind and data-seek (app.js renderBar, syncRows,
       updateSeeks find a copy anywhere); a part drawn from the song is drawn again in place (App.patch) at the player's
       changes its `on` names. make(attr): attr(name) is the <lull-part>'s own attribute. */
    function size(v, dflt) { return !v ? dflt : /^\d+$/.test(v) ? +v : v; }
    /* the copy playing, or null while nothing does */
    function copyNow() { var t = P.current(); return t && ps.status !== 'idle' ? P.playingCopy() : null; }
    var REPEAT = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' };
    /* Now Playing's builders (now.js), read when a part is drawn: this file may run before now.js has */
    function now() { return App.nowParts; }
    var PLAYED = ['track', 'source', 'status', 'play', 'ended'], COPY = ['track', 'source', 'status'];
    /* a part keeps its <lull-part> as its box, out of the layout, so the part is an item of whatever holds it (a grid, a
       row); a theme that wants the box gives it a display (its layers come after base) */
    var css = document.createElement('style');
    css.textContent = '@layer base { lull-part { display: contents; } lull-row { display: none; } }';
    document.head.appendChild(css);
    var defs = {
      'play-button': { make: function (a) { var s = +a('size'); return ui.playButton([40, 48, 52, 56, 64].indexOf(s) >= 0 ? s : 56); } },
      /* the player bar's own keys (index.html): shuffle and repeat kept by their data-bind (app.js renderBar) */
      prev: { make: function () { return html`<button class="btn-icon btn-icon--bare" data-act="prev" aria-label="Previous" data-tip="Previous (P)">${icon('prev')}</button>`; } },
      next: { make: function () { return html`<button class="btn-icon btn-icon--bare" data-act="next" aria-label="Next" data-tip="Next (N)">${icon('next')}</button>`; } },
      shuffle: { make: function () {
        return html`<button class="btn-icon btn-icon--bare" data-act="shuffle" data-bind="shuffle" aria-pressed="${String(!!ps.shuffle)}" aria-label="Shuffle" data-tip="Shuffle (S)">${icon('shuffle')}</button>`;
      } },
      repeat: { make: function () {
        var label = REPEAT[ps.repeat];
        return html`<button class="btn-icon btn-icon--bare" data-act="repeat" data-bind="repeat" aria-pressed="${String(ps.repeat !== 'off')}" aria-label="${label}" data-tip="${label} (R)">${icon(ps.repeat === 'one' ? 'repeat-one' : 'repeat')}</button>`;
      } },
      /* Like for whatever plays, as data-bind="like" keeps it (App.syncRows): the song it likes follows the player; with
         nothing playing it does nothing (the action) */
      'like-button': { make: function () {
        var t = P.current(), on = !!t && App.isLiked(t.id);
        return html`<button class="btn-icon btn-icon--bare ${on ? 'is-liked' : ''}" data-act="like" data-bind="like" data-id="${t ? t.id : ''}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Like" data-tip="Like (L)">${icon(on ? 'heart-fill' : 'heart')}</button>`;
      } },
      /* the player's Save for the playing song (data-bind="save"), disabled until one plays */
      'save-button': { make: function () {
        var t = P.current();
        return t ? ui.playerSave(t) : html`<button class="btn-icon btn-icon--bare" data-act="save" data-bind="save" aria-pressed="false" aria-label="Save to library (Shift+S)" data-tip="Save to library (Shift+S)" disabled>${icon('save')}</button>`;
      } },
      seek: { make: function (a) { return ui.seekBar({ bind: 'player', label: a('label') || 'Seek', vertical: a('orientation') === 'vertical' }); } },
      volume: { make: function (a) { return ui.volume({ vertical: a('orientation') === 'vertical' }); } },
      cover: { on: ['track'], make: function (a) { var t = P.current(); return t ? ui.cover(t, size(a('size'), 56), { hero: true }) : ''; } },
      /* Saved or Soulseek, as the player bar's (a YouTube copy has none there either) */
      'source-badge': { on: COPY, make: function () { var t = P.current(), c = copyNow(); return c && c.where !== 'youtube' ? ui.sourceBadge(t, { button: true }) : ''; } },
      'quality-chip': { on: COPY, make: function () {
        var t = P.current(), c = copyNow();
        return c && c.format ? ui.qualityChip(c.format, { solid: true, button: true, id: t.id, better: P.switchedCopy() }) : '';
      } },
      'playing-from': { on: ['track', 'queue'], make: function (a) { return ui.playingFrom('', a('label')); } },
      /* Now Playing's: the hero's facts and state words, the next songs as the queue's rows (count, label), and the
         lyrics as the Lyrics card draws them (their own instance, 'slot': no ids, so as many as a theme places) */
      facts: { on: PLAYED, make: function () { return now().facts(); } },
      state: { on: PLAYED, make: function () { return App.esc(now().state()); } },
      'up-next': { on: ['track', 'queue', 'ended'], make: function (a) {
        return now().next(App.clamp(parseInt(a('count'), 10) || 5, 1, 20), a('label') || 'Up next');
      } },
      lyrics: { on: ['track', 'lyrics'], make: function () { return now().lyrics('slot'); } }
    };

    /* ---------- sections and lists: what lull's screens register (ADR-0101 amended, B) ----------
       App.sections[name] = { make(attr), on: [kinds], data: path }: a section a screen draws (Home's shelves, Now
       Playing's queue and source card...), placed by a theme anywhere as <lull-part name="<name>">, drawn as its screen
       draws it. App.lists[name] = { items(), on, data }: a list lull holds, by record (live.json records), for the list
       part. A `data` path is fetched only where a part names it, and its parts are drawn again when it arrives. */
    var sections = App.sections, lists = App.lists, asked = {};
    function needs(path) {
      if (!path || asked[path]) return;
      asked[path] = 1;
      App.data.whenRead(path).then(function () { asked[path] = 2; App.parts.redraw('data:' + path); });
    }
    lists.queue = { on: ['track', 'queue', 'ended'], items: function () {
      var G = { next: 'yours', context: 'place', radio: 'radio' };
      return (ps.queue || []).slice((ps.index || 0) + 1).map(function (q) {
        var t = App.data.track(q.trackId); return t ? Object.assign({}, t, { group: G[q.group] || q.group || '' }) : null;
      }).filter(Boolean);
    } };
    /* `where="group=yours,kind=album"`: every field equal (a value may list choices: group=place|radio) */
    function matches(where) {
      var tests = (where || '').split(',').map(function (w) { var i = w.indexOf('='); return i > 0 ? [w.slice(0, i).trim(), w.slice(i + 1).split('|')] : null; }).filter(Boolean);
      return function (it) { return tests.every(function (t) { return t[1].indexOf(String(it[t[0]])) >= 0; }); };
    }
    /* a row of the theme's own (<lull-row>), each {{item.<field> | pipes}} filled from the item, escaped */
    function fill(row, it) {
      return row.replace(/\{\{\s*item((?:\.[a-zA-Z0-9-]+)+)\s*((?:\|[^|}]*)*)\}\}/g, function (all, path, pipes) {
        var v = it; path.slice(1).split('.').forEach(function (k) { v = v == null ? v : v[k]; });
        return App.esc(window.Live && Live.pipe ? Live.pipe(v, Live.parse('item' + pipes)[1]) : v == null ? '' : String(v));
      });
    }
    function linkOf(it) { return it.kind === 'artist' ? '#/artist/' + it.id : it.kind === 'album' ? '#/album/' + it.id : '#/album/' + it.albumId; }
    var list = { make: function (a, el) {
      var src = lists[a('of')]; if (!src) return '';
      needs(src.data);
      /* the theme's row stays in the part, never shown (base layer below), so each drawing finds it */
      var tpl = el.querySelector('lull-row'), row = tpl ? tpl.innerHTML : '', keep = tpl ? tpl.outerHTML : '';
      var items = (src.items() || []).filter(matches(a('where'))).slice(0, App.clamp(parseInt(a('count'), 10) || 20, 1, 100));
      var label = a('label') || '';
      if (!items.length) return keep + (a('empty') ? html`<p class="lull-list__empty">${a('empty')}</p>` : '');
      if (row) {
        return keep + '<ol class="lull-list" role="list"' + (label ? ' aria-label="' + App.esc(label) + '"' : '') + '>' + items.map(function (it, i) {
          return '<li class="lull-list__row" data-id="' + App.esc(it.id || '') + '" data-kind="' + App.esc(it.kind || 'track') + '" data-i="' + i + '">' + fill(row, it) + '</li>';
        }).join('') + '</ol>';
      }
      if (items.every(function (it) { return !it.kind || it.kind === 'track'; })) return ui.trackList(items, { dense: true, label: label || 'Songs', showSources: false });
      return html`<ul class="lull-list" role="list" aria-label="${label}">${items.map(function (it) { return html`<li class="lull-list__row"><a href="${linkOf(it)}">${it.title || it.name || ''}</a></li>`; })}</ul>`;
    } };
    function def(name) {
      return Object.prototype.hasOwnProperty.call(defs, name) ? defs[name] : name === 'list' ? list : sections[name] || null;
    }
    /* what a part is drawn again on: its own kinds, its list's, and the arrival of the data it reads */
    function drawnOn(d, host, kind) {
      var src = d === list ? lists[host.getAttribute('of')] : d;
      return !!src && ((src.on && src.on.indexOf(kind) >= 0) || (src.data && kind === 'data:' + src.data));
    }
    App.parts = {
      has: function (name) { return !!def(name); },
      /* a <lull-part>'s markup, with its pick and cls; the part's own attributes are make()'s to read */
      draw: function (el) {
        var d = def(el.getAttribute('name'));
        if (d.data) needs(d.data);
        return Designs.finish(el, String(d.make(function (a) { return el.getAttribute(a); }, el) || ''));
      },
      /* the parts drawn from what changed (a player change's kind, Now Playing's own: 'lyrics', data arriving:
         'data:<path>'), drawn again where shown */
      redraw: function (kind) {
        if (!Designs.partsDrawn) return;
        App.$$kept('lull-part').forEach(function (host) {
          var d = def(host.getAttribute('name'));
          if (d && drawnOn(d, host, kind)) patch(host, App.parts.draw(host));
        });
      }
    };
    App.on('player', App.parts.redraw);
    App.on('lyrics', function () { App.parts.redraw('lyrics'); });   // now.js, when a song's lyrics arrive
  }
  if (window.App && App.on) hook();
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', hook);
})();
