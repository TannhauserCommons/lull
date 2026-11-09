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
    var P = App.player, ps = P.state, ui = App.ui, html = App.html, icon = App.icon, $ = App.$, patch = App.patch;
    function empty() { return ''; }

    /* ---------- region slots ----------
       slots/before.<region>.html and after.<region>.html, drawn as the region's first or last child in a
       .region-slot[data-slot] box. lull never draws into a region's own element, only into the hosts inside it
       (#pbar-now, #ctx-body, #rail-playlists...), so a box stays where it is; it is drawn again only with the shell (a
       design or theme switch: app.js renderShell), which also takes its boxes away when the theme now in use has none.
       The hub takes exactly these names (crates/themes package.rs EDGES). */
    var REGIONS = ['rail', 'header', 'content', 'ctx', 'nowcol', 'playerbar', 'mini', 'tabbar'];
    function regions() {
      REGIONS.forEach(function (r) {
        var region = $('#app > [data-region="' + r + '"]'); if (!region) return;
        ['before', 'after'].forEach(function (edge) {
          var name = edge + '.' + r, box = $(':scope > .region-slot[data-slot="' + name + '"]', region);
          var markup = String(App.slot(name, {}, empty));   // nothing without a template
          if (!markup) { if (box) box.remove(); return; }
          if (!box) {
            box = document.createElement('div'); box.className = 'region-slot'; box.setAttribute('data-slot', name);
            region.insertBefore(box, edge === 'before' ? region.firstChild : null);
          }
          patch(box, markup);
        });
      });
    }
    /* ---------- screen slots ----------
       slots/before.<screen>.html and after.<screen>.html, inside the screen's own top container (SLOTS_IN: Now Playing's
       grid, beside the hero and the cards; else the first element it draws: .disc-home, .view__inner...), so the
       theme's boxes are items of the screen's grid beside lull's parts. Drawn with the screen, into its markup (app.js
       renderRoute), so a refresh keeps them as it keeps the rest (App.patch, by their data-key). Returns the markup as it
       was when the theme has none for this screen. */
    var SCREENS = ['home', 'search', 'now', 'library', 'liked', 'downloads', 'history', 'following', 'album', 'artist', 'tag',
      'playlists', 'playlist', 'outside', 'stats', 'settings'];
    var SLOTS_IN = { now: '.now__grid' };   // where a screen's slots go when it isn't the first element it draws
    function screen(markup, name) {
      if (SCREENS.indexOf(name) < 0) return markup;
      var slots = ['before.' + name, 'after.' + name].map(function (slot) { return [slot, String(App.slot(slot, {}, empty))]; })
        .filter(function (s) { return s[1]; });
      if (!slots.length) return markup;
      var tpl = document.createElement('template');
      tpl.innerHTML = String(markup || '');
      var host = SLOTS_IN[name] ? tpl.content.querySelector(SLOTS_IN[name]) : tpl.content.firstElementChild;
      if (!host) return markup;
      slots.forEach(function (s) {
        var slot = s[0], inner = s[1], box = document.createElement('div');
        box.className = 'region-slot'; box.setAttribute('data-slot', slot); box.setAttribute('data-key', 'slot:' + slot);
        box.innerHTML = inner;
        host.insertBefore(box, slot.indexOf('before.') === 0 ? host.firstChild : null);
      });
      return tpl.innerHTML;
    }
    App.edges = { regions: regions, screen: screen };

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
    css.textContent = '@layer base { lull-part { display: contents; } }';
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
    App.parts = {
      has: function (name) { return Object.prototype.hasOwnProperty.call(defs, name); },
      /* a <lull-part>'s markup, with its pick and cls; the part's own attributes are make()'s to read */
      draw: function (el) {
        return Designs.finish(el, String(defs[el.getAttribute('name')].make(function (a) { return el.getAttribute(a); }) || ''));
      },
      /* the parts drawn from what changed (a player change's kind, or Now Playing's own: 'lyrics'), drawn again where shown */
      redraw: function (kind) {
        if (!Designs.partsDrawn) return;
        App.$$kept('lull-part').forEach(function (host) {
          var def = defs[host.getAttribute('name')];
          if (def && def.on && def.on.indexOf(kind) >= 0) patch(host, App.parts.draw(host));
        });
      }
    };
    App.on('player', App.parts.redraw);
    App.on('lyrics', function () { App.parts.redraw('lyrics'); });   // now.js, when a song's lyrics arrive
  }
  if (window.App && App.on) hook();
  else if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', hook);
})();
