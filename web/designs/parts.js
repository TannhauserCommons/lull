/* parts.js: a theme's markup at every region's and screen's edge, lull's catalogue of parts for any slot, and the
   state a theme's own markup keeps (ADR-0101 and its amendments; docs/themes.md section 6). A theme pays only for
   what it uses: designs.js loads this file (after designs/model.js) only for a design or theme whose slots include a
   region or screen slot (slots/before.<name>.html, after.<name>.html), name a part only the catalogue draws, or keep a
   state (designs.js wantsParts). The built-ins use neither, so they never fetch it, and app.js's hooks for it
   (App.edges) do nothing. Runs in lull's page only: the desktop app's mini windows have no App, and their slots keep the
   parts their slot hands. It can load before app.js has run (with the theme, in the page's head): then it hooks in once
   the page has. */
(function () {
  'use strict';
  function hook() {
    if (!window.App || !App.on || App.parts) return;
    var P = App.player, ps = P.state, ui = App.ui, html = App.html, raw = App.raw, icon = App.icon, $ = App.$, $$ = App.$$, patch = App.patch;
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
       changes its `on` names. make(attr, el): attr(name) is the <lull-part>'s own attribute.
       Four kinds, each its own block below: the player's parts (PLAYER), parts for any item (ITEM, `for`), any piece of
       any screen (`screen`) and any list (`list`), and the state a theme's own markup keeps (`state`, an action). */
    var Model = window.LullModel;
    function size(v, dflt) { return !v ? dflt : /^\d+$/.test(v) ? +v : v; }
    /* the copy playing, or null while nothing does */
    function copyNow() { var t = P.current(); return t && ps.status !== 'idle' ? P.playingCopy() : null; }
    var REPEAT = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' };
    /* Now Playing's builders (now.js), read when a part is drawn: this file may run before now.js has */
    function now() { return App.nowParts; }
    var PLAYED = ['track', 'source', 'status', 'play', 'ended'], COPY = ['track', 'source', 'status'];
    /* a part keeps its <lull-part> as its box, out of the layout, so the part is an item of whatever holds it (a grid, a
       row); a theme that wants the box gives it a display (its layers come after base). A list's own row is its
       template, never shown */
    var css = document.createElement('style');
    css.textContent = '@layer base { lull-part { display: contents; } lull-row { display: none; } }';
    document.head.appendChild(css);

    /* ---------- the player's parts: the song playing, its keys and its player ---------- */
    var PLAYER = {
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

    /* ---------- parts for any item: for="<id>" (in a list's row, that row's item, by itself) ----------
       What lull draws for that song, album, artist or playlist, wired by its own actions (data-act with data-id) and kept
       current as lull's rows are (App.syncRows: likes, saves). The ones a part draws for an item */
    function title(it) { return it.title || it.name || ''; }
    var PLAYS = { track: 'play-track', album: 'play-album', playlist: 'play-playlist' };
    /* a song in the queue (qi, its place there) plays where it is, and its menu is the queue's */
    function inQueue(it) { return it.qi != null && ps.queue[it.qi] && ps.queue[it.qi].trackId === it.id; }
    var ITEM = {
      /* a song's or an album's cover, an artist's round, and a playlist's (or a tag's) mosaic of its songs' albums */
      cover: function (it, a) {
        var s = size(a('size'), 56);
        if (it.kind === 'artist') return ui.cover(it.id, s, { round: true });
        if (it.kind === 'track' || it.kind === 'album') return ui.cover(it, s);
        if (it.kind === 'playlist') return ui.playlistArt(it, s, '');   // its own picture, else its songs' mosaic
        return it.trackIds ? ui.mosaic(it.trackIds, s, '') : '';
      },
      'play-button': function (it, a) {
        var s = +a('size'), q = inQueue(it), act = q ? 'now-q-jump' : PLAYS[it.kind], label = (act ? 'Play ' : 'Start radio from ') + title(it);
        return html`<button class="btn-gloss btn-gloss--${[40, 48, 52, 56, 64].indexOf(s) >= 0 ? s : 40}" data-act="${act || 'start-radio'}" data-kind="${it.kind}" data-id="${it.id}" ${q ? raw('data-qi="' + it.qi + '"') : ''} aria-label="${label}" data-tip="${label}">${icon(act ? 'play' : 'radio', null, act ? 'ic-play' : '')}</button>`;
      },
      'like-button': function (it) { return it.kind === 'track' ? ui.likeButton(it.id) : ''; },
      'save-button': function (it) {
        return it.kind === 'track' ? ui.saveButton(it.id) : it.kind === 'album'
          ? html`<button class="btn-icon btn-icon--bare" data-act="save-album" data-id="${it.id}" aria-label="Save ${title(it)} to library" data-tip="Save to library">${icon('save')}</button>` : '';
      },
      'source-badge': function (it) { return it.kind === 'track' ? ui.sourceBadge(it) : ''; },
      'quality-chip': function (it) { return it.kind === 'track' && it.format ? ui.qualityChip(it.format) : ''; },
      menu: function (it) {
        if (it.kind !== 'track') return '';
        var q = inQueue(it);
        return html`<button class="btn-icon btn-icon--bare" data-act="${q ? 'now-q-menu' : 'track-menu'}" data-id="${it.id}" ${q ? raw('data-qi="' + it.qi + '"') : ''} aria-haspopup="menu" aria-label="More for ${title(it)}" data-tip="More">${icon('more')}</button>`;
      }
    };
    /* the item a part is for: its record, with what its list row adds (a queued song's qi and group, a play's at) */
    function itemOf(el) {
      var id = el.getAttribute('for'), row = el.closest('.lull-list__row[data-entry]');
      if (!id || !Model) return null;
      var it = Model.item(id), own = row && row.getAttribute('data-id') === id ? JSON.parse(row.getAttribute('data-entry')) : null;
      return it && own ? Object.assign({}, it, own) : it;
    }
    /* what a list adds to an item's record, kept on its row as JSON: its own plain fields but kind and id */
    function entryOf(it) {
      var own = {}, e = it.entry || {};
      Object.keys(e).forEach(function (k) { var v = e[k]; if (k !== 'kind' && k !== 'id' && v !== null && typeof v !== 'object') own[k] = v; });
      return JSON.stringify(own);
    }

    /* ---------- reads: what a placed part reads is fetched once where it is placed, and drawn again each time lull
       reads it (App.data.onRead), as the screen that shows it would be ---------- */
    var asked = {};
    function needs(path) { if (path && !asked[path]) { asked[path] = 1; App.data.get(path); } }
    App.data.onRead = function (path) { App.parts.redraw('read:' + path); };

    /* ---------- any piece of any screen: <lull-part name="screen" path="/home" pick='[data-region="jump"]'> ----------
       The screen at that path drawn as lull draws it (its route's render, off the page), and the piece `pick` keeps.
       It is drawn again at any read and at the player's changes, never on a tick. Its buttons work by lull's actions;
       what a screen wires when it is shown (its mount: a drag, a scroll) stays on that screen */
    var QUIET = { el: document.createElement('div'), refresh: false, offscreen: true, onCleanup: function () {}, on: function () {} };
    var screenPart = { on: ['track', 'queue', 'ended', 'source', 'status', 'mode', 'lyrics'], make: function (a) {
      var path = a('path') || '/home', at = App.routeOf(path.split('?')[0]);
      if (!at || (at.route.need && at.route.need(at.params, {}))) return '';   // its code still to load: drawn at the next change
      try { return String(at.route.render(Object.assign({}, QUIET, { params: at.params, query: {}, route: at.route })) || ''); }
      catch (e) { console.error('[screen part ' + path + ']', e); return ''; }
    } };

    /* ---------- any list: <lull-part name="list" of="queue" | read="/api/..." key="items" where sort count> ----------
       Its items are records (LullModel.list). With a <lull-row> inside, each item is drawn in the theme's own row:
       {{item.<field> | pipes}} filled, escaped, and every part in it bound to the item; without one, lull's rows */
    function fill(row, it) {
      return row.replace(/\{\{\s*item((?:\.[a-zA-Z0-9-]+)+)\s*((?:\|[^|}]*)*)\}\}/g, function (all, path, pipes) {
        var v = it; path.slice(1).split('.').forEach(function (k) { v = v == null ? v : v[k]; });
        return App.esc(window.Live && Live.pipe ? Live.pipe(v, Live.parse('item' + pipes)[1]) : v == null ? '' : String(v));
      });
    }
    function linkOf(it) { return it.kind === 'artist' ? '#/artist/' + it.id : it.kind === 'album' ? '#/album/' + it.id : it.kind === 'playlist' ? '#/playlist/' + it.id : '#/album/' + it.albumId; }
    function spec(a) { return { of: a('of'), read: a('read'), key: a('key'), where: a('where'), sort: a('sort'), count: a('count') }; }
    var listPart = { make: function (a, el) {
      var got = Model.list(spec(a)); needs(got.read);
      var tpl = el.querySelector('lull-row'), keep = tpl ? tpl.outerHTML : '', label = a('label') || '', items = got.items;
      if (!items.length) return keep + (a('empty') ? html`<p class="lull-list__empty">${a('empty')}</p>` : '');
      if (!tpl) {
        if (items.every(function (it) { return it.kind === 'track'; })) return ui.trackList(items, { dense: true, label: label || 'Songs', showSources: false });
        return html`<ul class="lull-list" role="list" aria-label="${label}">${items.map(function (it) { return html`<li class="lull-list__row"><a href="${linkOf(it)}">${title(it)}</a></li>`; })}</ul>`;
      }
      var box = document.createElement('template');
      /* a row of an album, an artist or a playlist is marked while the song plays from it, as lull's own (data-place:
         .is-playing-place, .is-playing-now, App.syncPlaces) */
      box.innerHTML = '<ol class="lull-list" role="list"' + (label ? ' aria-label="' + App.esc(label) + '"' : '') + '>' + items.map(function (it, i) {
        return '<li class="lull-list__row" data-id="' + App.esc(it.id || '') + '" data-kind="' + App.esc(it.kind || '') + '" data-i="' + i + '"' +
          (it.place ? ' data-place="' + App.esc(it.place) + '"' : '') + ' data-entry="' + App.esc(entryOf(it)) + '">' + fill(tpl.innerHTML, it) + '</li>';
      }).join('') + '</ol>';
      Array.prototype.forEach.call(box.content.querySelectorAll('.lull-list__row'), function (li, i) {
        Array.prototype.forEach.call(li.querySelectorAll('lull-part'), function (p) {
          if (!p.hasAttribute('for') && items[i].id) p.setAttribute('for', items[i].id);
          if (App.parts.has(p.getAttribute('name'))) p.innerHTML = App.parts.draw(p);
        });
      });
      App.syncPlaces(box.content);
      return keep + box.innerHTML;
    } };
    function listOn(host) { var g = Model.list(spec(function (n) { return host.getAttribute(n); })); return { on: g.on.concat(['like', 'save']), read: g.read }; }

    /* ---------- one name to its part ---------- */
    var sections = App.sections;
    function def(name, host) {
      if (host && host.hasAttribute('for') && ITEM[name]) return { item: ITEM[name], on: ['like', 'save'] };
      if (Object.prototype.hasOwnProperty.call(PLAYER, name)) return PLAYER[name];
      return name === 'list' ? listPart : name === 'screen' ? screenPart : sections[name] || null;
    }
    /* what a placed part is drawn again on: its kinds, its list's, and a read it makes coming back */
    function drawnOn(d, host, kind) {
      if (d === listPart) { var l = listOn(host); return l.on.indexOf(kind) >= 0 || kind === 'read:' + l.read; }
      if (d === screenPart) return d.on.indexOf(kind) >= 0 || kind.indexOf('read:') === 0;
      return (d.on && d.on.indexOf(kind) >= 0) || (d.data && kind === 'read:' + d.data);
    }
    App.parts = {
      has: function (name) { return !!def(name) || !!ITEM[name]; },
      /* a <lull-part>'s markup, with its pick and cls; the part's own attributes are make()'s to read */
      draw: function (el) {
        var name = el.getAttribute('name'), d = def(name, el), a = function (n) { return el.getAttribute(n); };
        if (d.item) { var it = itemOf(el); return Designs.finish(el, it ? String(d.item(it, a) || '') : ''); }
        if (d.data) needs(d.data);
        return Designs.finish(el, String(d.make(a, el) || ''));
      },
      /* the parts drawn from what changed (a player change's kind, Now Playing's 'lyrics', 'like' and 'save', a read
         coming back: 'read:<path>'), drawn again where shown; a list's own row is a template, never drawn */
      redraw: function (kind) {
        if (!Designs.partsDrawn) return;
        App.$$kept('lull-part').forEach(function (host) {
          if (host.closest('lull-row')) return;
          var d = def(host.getAttribute('name'), host);
          if (d && drawnOn(d, host, kind)) { patch(host, App.parts.draw(host)); App.syncPlaces(host); }
        });
      }
    };
    App.on('player', App.parts.redraw);
    App.on('lyrics', function () { App.parts.redraw('lyrics'); });   // now.js, when a song's lyrics arrive
    App.on('like', function () { App.parts.redraw('like'); });
    App.on('save', function () { App.parts.redraw('save'); });
    App.on('store', function (keys) { [].concat(keys).forEach(function (k) { App.parts.redraw('store:' + k); }); });

    /* ---------- state a theme's own markup keeps, without script: data-act="state" ----------
       data-set="tab=lyrics" sets a state, data-set="drawer" toggles one, data-set="mode=a|b|c" cycles one. The state is
       kept on the nearest [data-states] (else the slot's box) as name:value words of that attribute, which CSS follows
       ([data-states~="tab:lyrics"]); an element marked data-show="tab:lyrics" (or "drawer") is shown only in that state,
       hidden otherwise, from a screen reader too. A key that sets a state says whether it is on: aria-selected inside a
       role="tablist", else aria-pressed (a cycling key has neither). A set state always has a value: the markup's own
       first, so one tab is always open */
    function hostOf(el) { return el.closest('[data-states]') || el.closest('.region-slot'); }
    function states(h) { return (h.getAttribute('data-states') || '').split(/\s+/).filter(Boolean); }
    function valueOf(h, name) { var w = states(h).filter(function (s) { return s.split(':')[0] === name; })[0]; return w ? (w.indexOf(':') > 0 ? w.slice(name.length + 1) : 'on') : null; }
    function setState(h, name, v) {
      var w = states(h).filter(function (s) { return s.split(':')[0] !== name; });
      if (v != null) w.push(v === 'on' ? name : name + ':' + v);
      h.setAttribute('data-states', w.join(' '));
      applyStates(h);
    }
    function parseSet(s) { var i = s.indexOf('='); return i < 0 ? { name: s.trim(), values: null } : { name: s.slice(0, i).trim(), values: s.slice(i + 1).split('|') }; }
    App.action('state', function (el) {
      var h = hostOf(el), set = parseSet(el.getAttribute('data-set') || ''); if (!h || !set.name) return;
      var cur = valueOf(h, set.name);
      if (!set.values) setState(h, set.name, cur ? null : 'on');
      else if (set.values.length === 1) setState(h, set.name, set.values[0]);
      else setState(h, set.name, set.values[(set.values.indexOf(cur) + 1) % set.values.length]);
    });
    /* every state host within root drawn as its state says: what shows, what is pressed or selected */
    function applyStates(root) {
      var hosts = [].concat(root.matches && root.matches('[data-states]') ? [root] : [], Array.prototype.slice.call(root.querySelectorAll('[data-states]')));
      hosts.forEach(function (h) {
        Array.prototype.forEach.call(h.querySelectorAll('[data-show]'), function (el) {
          if (hostOf(el) !== h) return;
          var want = parseSet(el.getAttribute('data-show').replace(':', '=')), v = valueOf(h, want.name);
          var shown = want.values ? want.values.indexOf(v) >= 0 : !!v;
          if (el.hidden === shown) el.hidden = !shown;
        });
        Array.prototype.forEach.call(h.querySelectorAll('[data-act="state"]'), function (b) {
          if (hostOf(b) !== h) return;
          var set = parseSet(b.getAttribute('data-set') || ''), v = valueOf(h, set.name);
          if (set.values && set.values.length > 1) return;
          var on = set.values ? v === set.values[0] : !!v, tab = !!b.closest('[role="tablist"]');
          b.setAttribute(tab ? 'aria-selected' : 'aria-pressed', String(on));
        });
      });
    }
    App.states = { apply: applyStates };
    applyStates(document.body);
  }
  if (window.App && App.on) hook();
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', hook);
})();
