/* screens/library.js: Library and Playlists.
   Routes: /library/:segment?, /album/:id, /artist/:id, /tag/:id, /liked, /history, /following,
           /playlists, /playlist/:id.
   Hooks owned here: App.hooks.addToPlaylist (picker sheet), App.hooks.importFile (M3U preview).
   Data: library lists, album / artist / tag pages, likes, history and follows are per-route reads (M.get);
   playlists live in App.store('playlists') and change only through the server (docs/context/web.md).
   Reading order: helpers, library frame + segments, album, artist, tag, liked, history, following,
   playlists list, playlist page (table, reorder, edit, export, delete), picker, M3U import, actions, routes. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, raw = App.raw, icon = App.icon, $ = App.$, $$ = App.$$, fmt = App.fmt;

  /* =====================================================================
     1. Small helpers
     ===================================================================== */
  function plural(n, one, many) { return n + ' ' + (n === 1 ? one : (many || one + 's')); }
  function sumDuration(ids) { return ids.reduce(function (s, id) { var t = M.track(id); return s + (t ? t.duration : 0); }, 0); }
  function isSaved(id) { return App.isSaved(id); }
  function isLiked(id) { return App.isLiked(id); }
  function playlists() { return App.store.get('playlists') || []; }
  function playlist(id) { return playlists().filter(function (p) { return p.id === id; })[0] || null; }
  function plIds(p) { return (p.items || []).map(function (i) { return i.trackId; }); }
  function jellyfinDown() { var s = App.store.get('services'); return !!(s && s.library && s.library.state === 'down'); }
  /* lists name entities by id (or by an object carrying one); ids the cache can't describe are left out */
  function idList(items) { return (items || []).map(function (v) { return typeof v === 'string' ? v : v && v.id; }).filter(Boolean); }
  function resolve(items, lookup) { return idList(items).map(lookup).filter(Boolean); }
  function count(d) { return d.total != null ? d.total : (d.items || []).length; }
  /* playlist dates may be ISO dates or unix seconds; newest first */
  function stamp(when) { return typeof when === 'number' ? when * 1000 : Date.parse(when) || 0; }
  function newestFirst(a, b) { return stamp(b.updated) - stamp(a.updated); }
  function albumPath(id) { return '/api/albums/' + encodeURIComponent(id); }
  function artistPath(id) { return '/api/artists/' + encodeURIComponent(id); }
  function tagPath(id) { return '/api/tags/' + encodeURIComponent(id); }
  function playlistPath(id) { return '/api/playlists/' + encodeURIComponent(id); }

  /* re-render the route and put focus back on the same control (matched by data-act + data-id, or by row),
     so live updates (follow, like, unfollow) never drop keyboard focus */
  function refreshKeepFocus() {
    var view = $('#view'), a = document.activeElement, sel = null, rowIndex = -1, act = null, actIndex = -1;
    if (a && view.contains(a) && a !== view) {
      var row = a.closest('.trow');
      if (row) rowIndex = $$('.trow', row.parentElement).indexOf(row);
      if (a.getAttribute('data-act')) {
        act = '[data-act="' + a.getAttribute('data-act') + '"]'; actIndex = $$(act, view).indexOf(a);
        sel = act + (a.getAttribute('data-id') ? '[data-id="' + CSS.escape(a.getAttribute('data-id')) + '"]' : '');
        if (row) sel = '.trow[data-track-id="' + row.getAttribute('data-track-id') + '"] ' + sel;
      } else if (row) sel = '.trow[data-track-id="' + row.getAttribute('data-track-id') + '"]';
    }
    App.refresh();
    if (!sel && rowIndex < 0) return;
    var n = sel && $(sel, view), same;
    if (!n && rowIndex >= 0) { var rows = $$('.trow', view); n = rows[Math.min(rowIndex, rows.length - 1)]; }
    if (!n && act && (same = $$(act, view)).length) n = same[Math.min(actIndex, same.length - 1)];   // e.g. the next "Following" after an unfollow
    if (n) n.focus({ preventScroll: true });
  }
  /* re-render when any of these store keys change (unsubscribed when the route changes). `services` changes with every
     report from the hub (the cache's size moves while a song streams), but these pages draw only Jellyfin being offline
     and Last.fm's state: a page re-renders, every row of it, only when one of those moved */
  function servicesShown() { var lf = App.store.get('services').lastfm || {}; return [jellyfinDown(), lf.state, lf.queued || 0].join('|'); }
  function liveOn(ctx, keys, fn) {
    var shown = servicesShown();
    ctx.onCleanup(App.store.subscribe(function (changed) {
      var hit = changed.some(function (k) { return k !== 'services' && keys.indexOf(k) >= 0; });
      if (!hit && changed.indexOf('services') >= 0 && keys.indexOf('services') >= 0) hit = servicesShown() !== shown;
      if (!hit) return;
      shown = servicesShown();
      (fn || refreshKeepFocus)();
    }));
  }

  /* page states: loading, not found, a failed read (the server's sentence + retry) */
  function notFound(kind) {
    return html`<div class="view__inner lib"><div class="card">${ui.emptyState({ art: 'star', title: 'That ' + kind + ' isn\'t here',
      text: 'It may have been removed or renamed. Your library still has everything you saved.', action: { label: 'Open library', href: '#/library' } })}</div></div>`;
  }
  function pageError(err, path) { return html`<div class="view__inner lib">${App.errorCard(err, { path: path })}</div>`; }
  function pageSkeleton() {
    return html`<div class="view__inner lib"><section class="card lib-page" aria-label="Loading">
      <div class="lib-hero"><div class="skel skel--cover" aria-hidden="true"></div>${ui.skeleton('text', 4)}</div>
      ${ui.skeleton('row', 6)}</section></div>`;
  }
  /* the page for one entity read: not found, error, skeleton, or render() once the read has answered */
  function entityPage(kind, path, render) {
    var r = M.get(path);
    if (r.error) return r.error.status === 404 ? notFound(kind) : pageError(r.error, path);
    if (r.loading) return pageSkeleton();
    return render(r.data || {}) || notFound(kind);
  }
  /* inside a card: the failed read's sentence and a retry */
  function readError(err, path, title) {
    return ui.emptyState({ art: 'star', title: title || 'Couldn\'t load this', text: err.message, action: { label: 'Try again', act: 'reload-read', id: path } });
  }
  /* a discovery page's own entity ({ album } / { artist }): put through the cache so it gets the normalised shape */
  function pageEntity(d, field, mapKey, lookup) {
    var e = d[field]; if (!e || !e.id) return null;
    var map = {}; map[mapKey] = {}; map[mapKey][e.id] = e;
    M.absorb(map);
    return lookup(e.id);
  }
  /* a section of a discovery page that failed on the server (`errors: { section: message }`): its sentence + retry */
  function sectionError(d, section, path) {
    var msg = d.errors && d.errors[section];
    return msg ? readError({ message: msg }, path, 'Couldn\'t load this part') : '';
  }
  function gridSkeleton(n) { return html`<div class="lib-skel-grid">${ui.skeleton('tile', n || 8)}</div>`; }
  function listSkeleton(label) { return html`<section class="card lib-card" aria-label="${label}">${ui.skeleton('row', 6)}</section>`; }

  /* shared bits of markup */
  function pageHead(title, sub, tools) {
    return html`<header class="lib-head">
      <div class="lib-head__text"><h1 class="t-display">${title}</h1>${sub ? html`<p class="lib-head__sub">${sub}</p>` : ''}</div>
      ${tools ? html`<div class="lib-head__tools">${tools}</div>` : ''}
    </header>`;
  }
  /* a section label with tools at the end (sort, play) */
  function ruleWith(text, id, end) {
    return html`<div class="label-rule lib-rule"><h2 id="${id}">${text}</h2>${end ? html`<div class="label-rule__end">${end}</div>` : ''}</div>`;
  }
  /* Jellyfin offline (service down) or a list served from the read-only cache (`stale`); retry re-reads `path` */
  function jellyfinNotice(path, stale) {
    if (!stale && !jellyfinDown()) return '';
    return html`<div class="lib-notice s-card" role="status">${ui.statusGlyph('down', 'Jellyfin offline')}
      <p><b>Jellyfin is offline.</b> This is your library as last synced; saved songs can't play until it's back. Soulseek still works.</p>
      <button class="btn btn--sm" data-act="reload-read" data-id="${path}">${icon('refresh')}Retry connection</button></div>`;
  }
  /* The library list sends each album with its whole tracklist when the hub knows it (`saved: "partial"` when some
     are missing), else with only the songs the library holds (`saved: "all"` = all of those): the real tracklist
     then arrives with the album's page. Until it does, a tile can say how many songs are saved, not "Saved".
     `held` = the list's own count per album, `whole` = albums whose full tracklist is known. */
  var held = {}, whole = {};
  function noteHeld(d) {
    var albums = (d && d.entities && d.entities.albums) || {};
    (d.items || []).forEach(function (id) {
      var a = albums[id]; if (!a || !a.trackIds) return;
      if (a.saved === 'partial') whole[id] = true; else held[id] = a.trackIds.length;
    });
  }
  function heldOnly(a) { return held[a.id] > 0 && !whole[a.id] && a.trackIds.length <= held[a.id]; }
  /* how much of an album is saved (all | partial | none): counted from its songs when known, so it follows the
     save pipeline live; else the album's own `saved` summary (library lists send only the album head) */
  function savedState(a) {
    var total = a.trackIds.length, n = a.trackIds.filter(isSaved).length;
    if (heldOnly(a)) return { state: 'partial', text: held[a.id] + ' saved', label: plural(held[a.id], 'song') + ' saved in your library' };
    if (!total) return { state: a.saved || 'none', text: 'partly saved' };
    return { state: n === total ? 'all' : n ? 'partial' : 'none', text: n + ' of ' + total, label: n + ' of ' + total + ' songs saved' };
  }
  function albumStatus(a) {
    var s = savedState(a);
    if (s.state === 'all') return ui.savedBadge();
    if (s.state === 'partial') return html`<span class="badge badge-saved lib-partial" aria-label="${s.label || 'Partly saved'}">${icon('saved')}${s.text}</span>`;
    return ui.soulseekBadge();
  }
  function followButton(id) {
    var on = App.isFollowed(id);
    return html`<button class="btn lib-toggle" data-act="follow" data-id="${id}" aria-pressed="${on ? 'true' : 'false'}">${icon(on ? 'check' : 'follow')}${on ? 'Following' : 'Follow'}</button>`;
  }
  function likeAllButton(ids) {
    var all = ids.length && ids.every(isLiked);
    return html`<button class="btn-icon ${all ? 'is-liked' : ''}" data-act="lib-like-all" data-ids="${ids.join(',')}" aria-pressed="${all ? 'true' : 'false'}" aria-label="Like every song" data-tip="${all ? 'Every song liked' : 'Like every song'}">${icon(all ? 'heart-fill' : 'heart')}</button>`;
  }
  /* "Save" for a group of songs (album, playlist, liked): idle / missing / saving / failed / in library */
  function groupSaveButton(ids, key) {
    var saves = App.store.get('saves'), saved = ids.filter(isSaved).length;
    var running = ids.filter(function (id) { return saves[id] && saves[id].status === 'saving'; });
    var failed = ids.filter(function (id) { return saves[id] && saves[id].status === 'failed'; });
    var st = 'idle', label, ic = 'save', act = 'lib-save-group', style = '';
    if (saved === ids.length) { st = 'library'; label = 'In library'; ic = 'saved'; act = 'noop-saved'; }
    else if (running.length) {
      var p = running.reduce(function (s, id) { return s + saves[id].step; }, 0) / (running.length * 5);
      st = 'saving'; label = html`Saving <span class="px">${running.length}</span> ${running.length === 1 ? 'song' : 'songs'}`; style = '--p:' + p.toFixed(2);
    } else if (failed.length) { st = 'failed'; label = 'Retry ' + failed.length + ' failed'; ic = 'warning'; act = 'lib-retry-group'; }
    else if (saved) label = 'Save ' + (ids.length - saved) + ' missing';
    else label = ids.length === 1 ? 'Save' : 'Save all';
    return html`<button class="btn btn-save" data-state="${st}" data-lib-save="${key}" data-act="${act}" data-ids="${ids.join(',')}" ${style ? raw('style="' + style + '" aria-live="polite"') : ''}>${icon(ic)}<span>${label}</span></button>`;
  }
  /* keep every group save button on the page in step with the save pipeline, without a full re-render */
  function liveSaveButtons(ctx) {
    ctx.onCleanup(App.store.subscribe(function (keys) {
      if (keys.indexOf('saves') < 0) return;
      $$('[data-lib-save]', ctx.el).forEach(function (b) {
        var focused = document.activeElement === b, key = b.getAttribute('data-lib-save');
        b.outerHTML = String(groupSaveButton(b.getAttribute('data-ids').split(','), key));
        if (focused) $('[data-lib-save="' + key + '"]', ctx.el).focus();
      });
      $$('[data-lib-album-status]', ctx.el).forEach(function (el) {
        var a = M.album(el.getAttribute('data-lib-album-status'));
        if (a) el.innerHTML = String(albumStatus(a));
      });
    }));
  }

  /* tiles */
  function albumTile(a) {
    var key = 'album:' + a.id;
    return html`<div class="tile lib-tile${ui.placeCls(key)}" data-album-id="${a.id}" data-place="${key}" draggable="true">
      <div class="tile__art"><a href="#/album/${a.id}" aria-label="${a.title} by ${a.artist}">${ui.cover(a, '100%')}</a>
        <button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/album/${a.id}" data-place-at>${ui.placeMark(key)}${a.title}</a></div>
      <div class="tile__by"><a href="#/artist/${a.artistId}">${a.artist}</a></div>
      <div class="lib-tile__meta"><span data-lib-album-status="${a.id}">${albumStatus(a)}</span><span class="px">${a.type.toLowerCase() === 'album' ? a.year : a.type.toLowerCase()}</span></div>
    </div>`;
  }
  function artistTile(ar) {
    var key = 'artist:' + ar.id;
    return html`<div class="tile lib-tile lib-tile--artist${ui.placeCls(key)}" data-place="${key}">
      <div class="tile__art"><a href="#/artist/${ar.id}" aria-label="${ar.name}">${ui.cover(ar, '100%', { round: true })}</a>
        <button class="tile__play" data-act="start-radio" data-kind="artist" data-id="${ar.id}" aria-label="Start ${ar.name} radio" data-tip="Artist radio">${icon('radio')}</button></div>
      <div class="tile__title"><a href="#/artist/${ar.id}" data-place-at>${ui.placeMark(key)}${ar.name}</a></div>
      <div class="tile__by">${artistMeta(ar)}</div>
    </div>`;
  }
  /* following, else Last.fm listeners when known */
  function artistMeta(ar) {
    if (App.isFollowed(ar.id)) return html`${icon('check', null, 'ic-16')} following`;
    return ar.listeners ? App.discover.thousands(ar.listeners) + ' listeners' : '';
  }

  /* =====================================================================
     2. Library home: frame + segments (Albums, Artists, Songs, Genres & tags, Liked, History, Playlists, Following)
        Segments read GET /api/library/{seg}?sort=&limit= ({ items, total, entities, stale }).
     ===================================================================== */
  var SEGMENTS = [
    { id: 'albums', label: 'Albums', href: '#/library/albums' },
    { id: 'artists', label: 'Artists', href: '#/library/artists' },
    { id: 'songs', label: 'Songs', href: '#/library/songs' },
    { id: 'genres', label: 'Genres & tags', href: '#/library/genres' },
    { id: 'liked', label: 'Liked', href: '#/liked' },
    { id: 'history', label: 'History', href: '#/history' },
    { id: 'playlists', label: 'Playlists', href: '#/playlists' },
    { id: 'following', label: 'Following', href: '#/following' },
    { id: 'downloads', label: 'Downloads', href: '#/downloads' }
  ];
  /* the server's sort keys (an unknown one is a 400); artists sort by name, genres by song count (its defaults) */
  var ALBUM_SORTS = { recent: 'Recently saved', title: 'Title', artist: 'Artist', year: 'Release year' };
  var SONG_SORTS = { album: 'Album order', title: 'Title', artist: 'Artist', recent: 'Recently saved' };
  /* stored per device; a value this version doesn't know falls back to the default */
  function pref(key, known, fallback) { var v = App.storage(key); return known.indexOf(v) >= 0 ? v : fallback; }
  /* "Save the songs too" for every import (file, link, a preview's copy): one choice per device, shown where it applies */
  function saveSongsPref() { return App.storage('libImportSaveSongs') === true; }
  function setSaveSongsPref(on) { App.storage('libImportSaveSongs', !!on); }
  function saveSongsSwitch(help) {
    return html`<label class="switch-row lib-imp__opt" data-save-songs><span><span class="switch-row__label">Save the songs too</span>
      <span class="switch-row__help">${help}</span></span>
      <button class="switch" type="button" role="switch" aria-checked="${String(saveSongsPref())}"></button></label>`;
  }
  /* a click on a "Save the songs too" row flips it and remembers the choice */
  document.addEventListener('click', function (e) {
    var row = e.target.closest && e.target.closest('[data-save-songs]'); if (!row) return;
    e.preventDefault();
    var sw = $('.switch', row), on = sw.getAttribute('aria-checked') !== 'true';
    sw.setAttribute('aria-checked', String(on)); setSaveSongsPref(on);
  });
  var prefs = {
    albumSort: pref('libAlbumSort', Object.keys(ALBUM_SORTS), 'recent'),
    songSort: pref('libSongSort', Object.keys(SONG_SORTS), 'album')
  };
  /* lists come 200 at a time; "Show all" raises a list's limit to its total (the server's cap is 5000) */
  var MAX_LIMIT = 5000, limits = {};
  function withLimit(path, key) { return limits[key] ? path + (path.indexOf('?') < 0 ? '?' : '&') + 'limit=' + limits[key] : path; }
  function showAll(key, d, noun) {
    var shown = (d.items || []).length, total = count(d);
    if (shown >= total || shown >= MAX_LIMIT) return '';
    return html`<p class="lib-more"><button class="btn btn--quiet btn--sm" data-act="lib-show-all" data-id="${key}" data-value="${Math.min(total, MAX_LIMIT)}">Show all ${plural(total, noun)}</button></p>`;
  }
  function libraryPath(seg) {
    var q = seg === 'albums' ? '?sort=' + prefs.albumSort : seg === 'songs' ? '?sort=' + prefs.songSort : '';
    return withLimit('/api/library/' + seg + q, seg);
  }

  /* `path` is the read the page shows (the notice's retry re-reads it); `stale` = served from the offline cache */
  function libraryFrame(seg, body, path, stale) {
    var tools = html`<button class="btn" data-act="lib-import">${icon('import')}Import</button>
      ${ui.iconButton('refresh', 'Rescan your library', { 'data-act': 'jellyfin-rescan' })}`;
    return html`<div class="view__inner lib">
      ${pageHead('library', 'Songs you save from Soulseek are tagged and filed in your library.', tools)}
      <nav class="lib-segs" aria-label="Library sections"><ul>${SEGMENTS.map(function (s) {
        return html`<li><a class="pill-filter" href="${s.href}" ${s.id === seg ? raw('aria-current="page"') : ''}>${s.label}</a></li>`;
      })}</ul></nav>
      ${jellyfinNotice(path, stale)}
      ${body}
    </div>`;
  }
  function emptyLibraryCard() {
    return html`<section class="card lib-card" aria-label="Library">${ui.emptyState({ art: 'slot', title: 'Nothing saved yet',
      text: 'Save songs to keep them: they are tagged, filed and added to Jellyfin. Until then everything streams from Soulseek.',
      action: { label: 'Search Soulseek', href: '#/search' }, secondary: { label: 'Import a playlist', act: 'lib-import' } })}</section>`;
  }
  /* a segment's list inside its card: skeleton, the honest error (503: Jellyfin down and nothing cached), or the list */
  function listBody(r, path, skeleton, fn) {
    if (r.loading) return skeleton;
    if (r.error) return readError(r.error, path, r.error.status === 503 ? 'Your library is offline' : null);
    return fn(r.data);
  }
  function sortButton(act, label, id) {
    return html`<button class="btn btn--quiet btn--sm lib-sort" data-act="${act}" ${id ? raw('data-id="' + id + '"') : ''} aria-haspopup="menu">${icon('sort')}${label}${icon('chevron-down', null, 'ic-16')}</button>`;
  }

  /* every library album is saved, so there is no saved-state filter: the list, sorted, with its count */
  function albumsSegment(r, path) {
    return html`<section class="card lib-card" aria-labelledby="lib-albums-h">
      ${ruleWith('albums', 'lib-albums-h', html`${r.data ? html`<span class="lib-count">${count(r.data)}</span>` : ''}${sortButton('lib-album-sort', ALBUM_SORTS[prefs.albumSort])}`)}
      ${listBody(r, path, gridSkeleton(), function (d) {
        noteHeld(d);
        return html`<div class="lib-grid">${resolve(d.items, M.album).map(albumTile)}</div>${showAll('albums', d, 'album')}`;
      })}
    </section>`;
  }
  /* the server sorts by name; artists you follow come first */
  function artistsSegment(r, path) {
    return html`<section class="card lib-card" aria-labelledby="lib-artists-h">
      ${ruleWith('artists', 'lib-artists-h', html`<a class="btn btn--quiet btn--sm" href="#/following">Manage following</a>`)}
      ${listBody(r, path, gridSkeleton(), function (d) {
        var list = resolve(d.items, M.artist).sort(function (a, b) { return App.isFollowed(b.id) - App.isFollowed(a.id); });
        return html`<div class="lib-grid lib-grid--artists">${list.map(artistTile)}</div>${showAll('artists', d, 'artist')}`;
      })}
    </section>`;
  }
  /* date added and plays for a wide list (ADR-0007): GET /api/library/songs sends them per song, outside the entity */
  function songFacts(list, songs) {
    if (!songs) return {};
    function fact(t) { return songs[t.id] || {}; }
    return { dated: true, added: list.map(function (t) { return fact(t).addedAt ? String(fact(t).addedAt).slice(0, 10) : null; }),
      plays: list.map(function (t) { return fact(t).plays != null ? fact(t).plays : null; }) };
  }
  function songsSegment(r, path) {
    return html`<section class="card lib-card" aria-labelledby="lib-songs-h">
      ${ruleWith('saved songs', 'lib-songs-h', sortButton('lib-song-sort', SONG_SORTS[prefs.songSort]))}
      ${listBody(r, path, ui.skeleton('row', 8), function (d) {
        var list = resolve(d.items, M.track), ids = list.map(function (t) { return t.id; });
        return html`<div class="lib-bar">
          <button class="btn btn--primary" data-act="play-ids" data-ids="${ids.join(',')}">${icon('play')}Play all</button>
          <button class="btn" data-act="lib-shuffle-ids" data-ids="${ids.join(',')}">${icon('shuffle')}Shuffle</button>
          <span class="lib-facts px">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</span>
        </div>
        ${ui.trackList(list, Object.assign({ label: 'Saved songs', unavailable: jellyfinDown() || !!d.stale }, songFacts(list, d.songs)))}
        ${showAll('songs', d, 'song')}`;
      })}
    </section>`;
  }
  /* genre items are { id, name, artists, songs, trackIds (a few, for the mosaic) }; a tag page that has been read
     leaves its description in the tag cache */
  function genresSegment(r, path) {
    return html`<section class="card lib-card" aria-labelledby="lib-genres-h">
      ${ruleWith('genres & tags', 'lib-genres-h', html`<span class="t-micro fg-2">Tags from MusicBrainz and Last.fm</span>`)}
      ${listBody(r, path, ui.skeleton('row', 6), function (d) {
        var tags = (d.items || []).map(function (g) { return Object.assign({}, M.tag(g.id), g); });
        if (!tags.length) return ui.emptyState({ art: 'moon', title: 'No tags yet', text: 'Tags arrive with the songs you save, from MusicBrainz and Last.fm.' });
        return html`<ul class="lib-tags">${tags.map(genreCard)}</ul>${showAll('genres', d, 'tag')}`;
      })}
    </section>`;
  }
  function genreCard(tag) {
    var on = App.isFollowed(tag.id);
    return html`<li class="lib-tagcard inset">
      <a class="lib-tagcard__art" href="#/tag/${encodeURIComponent(tag.id)}" tabindex="-1" aria-hidden="true">${ui.mosaic(tag.trackIds || [], 72)}</a>
      <div class="lib-tagcard__main">
        <h3><a href="#/tag/${encodeURIComponent(tag.id)}">${tag.name}</a></h3>
        <p class="lib-tagcard__meta">${plural(tag.artists || 0, 'artist')}, ${plural(tag.songs || 0, 'saved song')}</p>
        ${tag.desc ? html`<p class="lib-tagcard__desc">${tag.desc}</p>` : ''}
      </div>
      <div class="lib-tagcard__tools"><button class="chip" data-act="follow-tag" data-id="${tag.id}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Follow ${tag.name}">${icon(on ? 'check' : 'plus')}${on ? 'Following' : 'Follow'}</button>
        ${ui.iconButton('radio', 'Start ' + tag.name + ' radio', { 'data-act': 'start-radio', 'data-kind': 'tag', 'data-id': tag.id, cls: 'btn-icon--sm' })}</div>
    </li>`;
  }

  var SEGMENT_BODY = { albums: albumsSegment, artists: artistsSegment, songs: songsSegment, genres: genresSegment };
  function renderLibrary(ctx) {
    var seg = ctx.params.segment || 'albums';
    if (seg === 'liked') return renderLiked();
    if (seg === 'history') return renderHistory();
    if (seg === 'following') return renderFollowing();
    if (seg === 'saved') seg = 'songs';
    if (!SEGMENT_BODY[seg]) seg = 'albums';
    var path = libraryPath(seg), r = M.get(path), d = r.data;
    var empty = d && !count(d) && seg !== 'genres';      // nothing saved at all
    return libraryFrame(seg, empty ? emptyLibraryCard() : SEGMENT_BODY[seg](r, path), path, !!(d && d.stale));
  }
  function mountLibrary(ctx) {
    if (ctx.params.segment === 'liked') { mountLiked(ctx); return; }
    liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']);
  }

  /* =====================================================================
     3. Album page: one cream card, art left, title block, disc headers (GET /api/albums/{id} -> { album, entities })
     ===================================================================== */
  function renderAlbum(ctx) {
    var path = albumPath(ctx.params.id);
    return entityPage('album', path, function (d) {
      var a = pageEntity(d, 'album', 'albums', M.album); if (!a) return null;
      whole[a.id] = true;                               // this page has the album's full tracklist
      var ids = a.trackIds, tracks = ids.map(M.track).filter(Boolean), down = jellyfinDown(), noTracks = !ids.length, gettingList = noTracks && !!d.partial;
      var discs = []; tracks.forEach(function (t) { var n = (t.disc || 1) - 1; (discs[n] = discs[n] || []).push(t); });
      discs = discs.filter(Boolean);
      return html`<div class="view__inner lib">
        ${jellyfinNotice(path)}
        <article class="card lib-page lib-page--album" aria-labelledby="lib-album-title" data-meta="${albumMeta(a)}" data-tracks="${tracks.length}" data-duration="${a.duration}" data-size="${a.sizeMB}">
          <div class="lib-hero">
            ${ui.cover(a, 'var(--lib-art)', { label: 'Cover of ' + a.title })}
            <div class="lib-hero__info">
              <p class="lib-kicker">${a.type.toLowerCase()}${a.year ? ', ' + a.year : ''}</p>
              <h1 class="t-display lib-title" id="lib-album-title">${a.title}</h1>
              <p class="lib-hero__by"><a href="#/artist/${a.artistId}">${a.artist}</a></p>
              <div class="lib-hero__facts">
                <span data-lib-album-status="${a.id}">${albumStatus(a)}</span>${a.format ? ui.qualityChip(a.format) : ''}${a.isNew ? ui.newBadge() : ''}
                ${noTracks ? '' : html`<span class="px">${plural(tracks.length, 'song')}, ${fmt.length(a.duration || 0)}</span>`}
              </div>
              <p class="lib-hero__fmt" aria-hidden="true">${fmt.albumLine(a)}</p>
              <div class="lib-chips">${a.tags.map(function (g) { return ui.tagChip(g); })}</div>
              <div class="lib-actions">
                ${noTracks ? '' : html`<button class="btn btn--primary" data-act="play-album" data-id="${a.id}">${icon('play', null, 'ic-play')}Play</button>
                <button class="btn" data-act="shuffle-album" data-id="${a.id}">${icon('shuffle')}Shuffle</button>
                ${groupSaveButton(ids, 'album')}`}
                <span class="lib-actions__icons">
                  ${noTracks ? '' : html`${likeAllButton(ids)}
                  ${ui.downloadButton({ kind: 'album', id: a.id }, ids)}
                  ${ui.iconButton('playlists', 'Add album to playlist', { 'data-act': 'add-to-playlist', 'data-ids': ids.join(','), 'aria-haspopup': 'dialog' })}`}
                  ${ui.iconButton('radio', 'Start album radio', { 'data-act': 'start-radio', 'data-kind': 'album', 'data-id': a.id })}
                  ${ui.iconButton('more', 'More album actions', { 'data-act': 'lib-album-menu', 'data-id': a.id, 'aria-haspopup': 'menu' })}
                </span>
              </div>
            </div>
          </div>
          <div class="lib-album__songs">
          ${gettingList ? ui.emptyState({ art: 'wave', title: 'Getting the tracklist…', text: 'The songs show up here as soon as they arrive.' })
            : noTracks ? ui.emptyState({ art: 'moon', title: 'No tracklist found yet', text: 'lull hasn\'t found the songs on this release yet. Soulseek may have them.',
            action: { label: 'Search Soulseek', href: '#/search?q=' + encodeURIComponent(a.artist + ' ' + a.title) } }) : ''}
          ${discs.map(function (list, i) {
            return html`${discs.length > 1 ? html`<div class="label-rule label-rule--sm lib-disc"><h2>disc ${i + 1}</h2><span class="label-rule__end t-micro fg-2">${plural(list.length, 'song')}, ${fmt.length(sumDuration(list.map(function (t) { return t.id; })))}</span></div>` : ''}
            <ol class="tlist tlist--album" data-ids="${ids.join(',')}" aria-label="${discs.length > 1 ? 'Disc ' + (i + 1) + ' of ' + a.title : 'Songs on ' + a.title}">${list.map(function (t, j) {
              return ui.trackRow(t, { index: j + 1, unavailable: down && isSaved(t.id), album: a.id });
            })}</ol>`;
          })}
          ${noTracks ? '' : html`<p class="lib-foot">${albumFoot(a)}</p>`}
          </div>
        </article>
      </div>`;
    });
  }
  /* "Released Oct 29, 2026 on Breakwater Tapes. 250 MB as FLAC 16/44.1, saved songs are in Jellyfin." (known parts only) */
  function albumFoot(a) {
    var released = a.released ? 'Released ' + fmt.date(a.released, true) + (a.label ? ' on ' + a.label : '') + '.' : '';
    var rest = [a.sizeMB ? fmt.size(a.sizeMB) + (a.format ? ' as ' + a.format : '') : a.format || '',
      savedState(a).state !== 'none' ? 'saved songs are in Jellyfin' : 'streaming from Soulseek until you save it'].filter(Boolean).join(', ');
    return (released ? released + ' ' : '') + rest.charAt(0).toUpperCase() + rest.slice(1) + '.';
  }
  /* an album page that answered `partial: true` gets `album.updated` once the full tracklist is stored: read it again */
  function mountAlbum(ctx) {
    liveSaveButtons(ctx); liveOn(ctx, ['liked', 'services']);
    ctx.onCleanup(App.on('server:album.updated', function (e) { if (e.albumId === ctx.params.id) M.reload(albumPath(e.albumId)); }));
  }

  /* =====================================================================
     4. Artist page, laid out like a streaming service's: the hero (photo, or the design's monogram; name, Last.fm
        listeners, play, shuffle, follow, radio), Popular (5 rows, See more for 10, with Last.fm plays), About, the
        discography by group, Appears on, Fans also like.
        (GET /api/artists/{id} -> { artist, topTracks, counts, popularVia, albums, discography: { albums, singles,
        compilations, appearsOn }, similar, pending, errors }). A section named in `pending` shows its skeleton until
        `artist.updated`, when the page is read again and patched in place.
     ===================================================================== */
  var POPULAR_SHOWN = 5;
  var DISCO_GROUPS = [['albums', 'Albums'], ['singles', 'Singles and EPs'], ['compilations', 'Compilations']];
  var POPULAR_VIA = { lastfm: 'by Last.fm plays', deezer: 'by Deezer rank', itunes: 'by Apple Music', listenbrainz: 'by ListenBrainz listens' };
  var artistView = {};                      // artistId -> { more, group }: See more and the discography group, this session
  function renderArtist(ctx) {
    var path = artistPath(ctx.params.id);
    return entityPage('artist', path, function (d) {
      var ar = pageEntity(d, 'artist', 'artists', M.artist); if (!ar) return null;
      var view = artistView[ar.id] = artistView[ar.id] || { more: false, group: null };
      var pending = d.pending || [], top = resolve(d.topTracks, M.track);
      var disco = d.discography || { albums: d.albums || [] };
      var groups = DISCO_GROUPS.map(function (g) { return { id: g[0], label: g[1], items: resolve(disco[g[0]], M.album) }; })
        .filter(function (g) { return g.items.length; });
      var appears = resolve(disco.appearsOn, M.album);
      var similar = resolve(d.similar, M.artist).filter(function (x) { return x.id !== ar.id; });
      return html`<div class="view__inner lib lib-artist">
        ${jellyfinNotice(path)}
        ${artistHero(ar, top, ownReleases(d), pending.indexOf('topTracks') >= 0)}
        <div class="lib-cols lib-artist__top">
          ${popularCard(ar, d, top, view, pending.indexOf('topTracks') >= 0, path)}
          ${aboutCard(ar, pending.indexOf('bio') >= 0)}
        </div>
        ${discographyCard(ar, groups, view, pending.indexOf('albums') >= 0, d, path)}
        ${appears.length ? html`<section class="card lib-card" aria-labelledby="lib-appears-h">${ruleWith('appears on', 'lib-appears-h', html`<span class="t-micro fg-2">${plural(appears.length, 'release')}</span>`)}
          <div class="shelf">${appears.map(albumTile)}</div></section>` : ''}
        ${similar.length || pending.indexOf('similar') >= 0 || sectionError(d, 'similar', path) ? html`<section class="card lib-card" aria-labelledby="lib-similar-h">${ruleWith('fans also like', 'lib-similar-h', html`<span class="t-micro fg-2">via Last.fm similar artists</span>`)}
          ${sectionError(d, 'similar', path) || (similar.length ? html`<div class="shelf">${similar.map(artistTile)}</div>` : shelfSkeleton())}</section>` : ''}
      </div>`;
    });
  }
  /* the hero: a soft glow of the photo behind it (only a real photo; without one the cover is the design's monogram) */
  function artistHero(ar, top, releases, gettingSongs) {
    var ids = top.map(function (t) { return t.id; }).join(',');
    var facts = [ar.listeners ? html`<span><b>${ar.listeners.toLocaleString('en-US')}</b> Last.fm listeners</span>` : '',
      releases ? html`<span>${plural(releases, 'release')}</span>` : ''].filter(String);
    return html`<section class="card lib-page lib-ahero${ar.image ? ' has-photo' : ''}" aria-labelledby="lib-artist-title">
      ${ar.image ? html`<div class="lib-ahero__glow" aria-hidden="true">${ui.cover(ar, '100%')}</div>` : ''}
      <div class="lib-hero lib-hero--round">
        ${ui.cover(ar, 'var(--lib-art)', { round: true, label: ar.image ? 'Photo of ' + ar.name : ar.name })}
        <div class="lib-hero__info">
          <p class="lib-kicker">artist</p>
          <h1 class="t-display lib-title" id="lib-artist-title">${ar.name}</h1>
          ${facts.length ? html`<p class="lib-ahero__facts">${facts}</p>` : ''}
          ${ar.tags.length ? html`<div class="lib-chips">${ar.tags.map(function (g) { return ui.tagChip(g); })}</div>` : ''}
          <div class="lib-actions">
            ${ids || gettingSongs ? html`<button class="btn btn--primary" data-act="play-ids" data-ids="${ids}" ${ids ? '' : raw('disabled')}>${icon('play', null, 'ic-play')}Play</button>
            <button class="btn" data-act="lib-shuffle-ids" data-ids="${ids}" ${ids ? '' : raw('disabled')}>${icon('shuffle')}Shuffle</button>` : ''}
            ${followButton(ar.id)}
            <button class="btn${ids || gettingSongs ? '' : ' btn--primary'}" data-act="start-radio" data-kind="artist" data-id="${ar.id}">${icon('radio')}Radio</button>
            <span class="lib-actions__icons">${ui.iconButton('more', 'More artist actions', { 'data-act': 'lib-artist-menu', 'data-id': ar.id, 'data-ids': ids, 'aria-haspopup': 'menu' })}</span>
          </div>
        </div>
      </div>
    </section>`;
  }
  /* the world's most played songs (Last.fm's plays on the catalog's songs): five, See more shows ten */
  function popularCard(ar, d, top, view, pending, path) {
    var counts = d.counts || {}, shown = view.more ? top : top.slice(0, POPULAR_SHOWN);
    var plays = shown.some(function (t) { return counts[t.id] && counts[t.id].plays != null; })
      ? shown.map(function (t) { var n = counts[t.id] && counts[t.id].plays; return n == null ? null : n === 1 ? 1 : n.toLocaleString('en-US'); }) : null;
    var body = top.length ? html`${ui.trackList(shown, { label: 'Popular songs by ' + ar.name, unavailable: jellyfinDown(), plays: plays })}
        ${top.length > POPULAR_SHOWN ? html`<button class="btn btn--quiet btn--sm lib-popular__more" data-act="lib-popular-more" data-id="${ar.id}" aria-expanded="${view.more ? 'true' : 'false'}">${view.more ? 'Show less' : 'See more'}</button>` : ''}`
      : pending ? ui.skeleton('row', POPULAR_SHOWN)
      : sectionError(d, 'albums', path) || ui.emptyState({ art: 'wave', title: 'No popular songs found', text: 'Artist radio still finds ' + ar.name + ' on Soulseek.' });
    return html`<section class="card lib-card lib-popular" aria-labelledby="lib-top-h">
      ${ruleWith('popular', 'lib-top-h', d.popularVia ? html`<span class="t-micro fg-2">${POPULAR_VIA[d.popularVia] || ''}</span>` : '')}
      ${body}
    </section>`;
  }
  /* About: the listener count and the bio (Last.fm's), only when there is something to say */
  function aboutCard(ar, pending) {
    if (!ar.bio && !pending) return '';
    return html`<section class="card lib-card lib-about" aria-labelledby="lib-about-h">
      ${ruleWith('about', 'lib-about-h')}
      ${ar.bio ? html`<p class="lib-bio" id="lib-bio">${ar.bio}</p>` : ui.skeleton('text', 4)}
    </section>`;
  }
  /* releases by group (albums, singles and EPs, compilations), the group picked by a filter pill */
  function discographyCard(ar, groups, view, pending, d, path) {
    var group = groups.filter(function (g) { return g.id === view.group; })[0] || groups[0];
    var total = groups.reduce(function (n, g) { return n + g.items.length; }, 0);
    var body = group ? html`${groups.length > 1 ? html`<div class="lib-disco__groups" role="group" aria-label="Show">${groups.map(function (g) {
          return html`<button class="pill-filter" data-act="lib-disco-group" data-id="${ar.id}" data-group="${g.id}" aria-pressed="${g === group ? 'true' : 'false'}">${g.label}<span class="lib-count">${g.items.length}</span></button>`;
        })}</div>` : ''}
        <div class="shelf" data-key="disco-${group.id}" aria-label="${group.label}">${group.items.map(discoTile)}</div>`
      : pending ? shelfSkeleton()
      : sectionError(d, 'albums', path) || ui.emptyState({ art: 'moon', title: 'No releases found', text: 'Artist radio still finds their songs on Soulseek.' });
    return html`<section class="card lib-card lib-disco" aria-labelledby="lib-disco-h">
      ${ruleWith('discography', 'lib-disco-h', total ? html`<span class="t-micro fg-2">${plural(total, 'release')}</span>` : '')}
      ${body}
    </section>`;
  }
  /* a release on its artist's page: the year and kind instead of the artist's name */
  function discoTile(a) {
    var kind = a.type.toLowerCase(), key = 'album:' + a.id;
    return html`<div class="tile lib-tile${ui.placeCls(key)}" data-album-id="${a.id}" data-place="${key}" draggable="true">
      <div class="tile__art"><a href="#/album/${a.id}" aria-label="${a.title}, ${kind}${a.year ? ' from ' + a.year : ''}">${ui.cover(a, '100%')}</a>
        <button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/album/${a.id}" data-place-at>${ui.placeMark(key)}${a.title}</a></div>
      <div class="tile__by">${[a.year, kind === 'ep' ? 'EP' : kind].filter(Boolean).join(', ')}</div>
      <div class="lib-tile__meta"><span data-lib-album-status="${a.id}">${albumStatus(a)}</span></div>
    </div>`;
  }
  /* the artist's own releases (appears-on ones aside), as the discography counts them */
  function ownReleases(d) {
    var disco = d.discography; if (!disco) return (d.albums || []).length;
    return ['albums', 'singles', 'compilations'].reduce(function (n, g) { return n + (disco[g] || []).length; }, 0);
  }
  function shelfSkeleton() { return html`<div class="lib-skel-shelf">${ui.skeleton('tile', 6)}</div>`; }
  function mountArtist(ctx) {
    liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']);
    ctx.onCleanup(App.on('server:artist.updated', function (e) { if (e.artistId === ctx.params.id) M.reload(artistPath(e.artistId)); }));
  }
  App.action('lib-popular-more', function (el) {
    var v = artistView[el.getAttribute('data-id')]; if (!v) return;
    v.more = !v.more; refreshKeepFocus();
  });
  App.action('lib-disco-group', function (el) {
    var v = artistView[el.getAttribute('data-id')]; if (!v) return;
    v.group = el.getAttribute('data-group'); refreshKeepFocus();
  });
  App.action('lib-shuffle-ids', function (el) {
    var list = (el.getAttribute('data-ids') || '').split(',').filter(Boolean); if (!list.length) return;
    App.player.setShuffle(true); App.player.playList(list, Math.floor(Math.random() * list.length));
  });

  /* =====================================================================
     5. Genre / tag page: follow, tag radio, top artists, new releases
        (GET /api/tags/{id} -> { tag: { id, name, desc, related }, artists, newReleases (last 90 days), errors? })
     ===================================================================== */
  function renderTag(ctx) {
    var id = ctx.params.id, path = tagPath(id);
    return entityPage('tag', path, function (d) {
      var tag = pageEntity(d, 'tag', 'tags', M.tag) || M.tag(id), related = tag.related || [], on = App.isFollowed(id);
      var artists = resolve(d.artists, M.artist), albums = resolve(d.newReleases, M.album);
      /* the cover mosaic: one known song from each new release */
      var coverIds = albums.map(function (a) { return a.trackIds[0]; }).filter(Boolean);
      return html`<div class="view__inner lib">
        <section class="card lib-page" aria-labelledby="lib-tag-title">
          <div class="lib-hero">
            ${ui.mosaic(coverIds, 'var(--lib-art)', 'Covers tagged ' + tag.name)}
            <div class="lib-hero__info">
              <p class="lib-kicker">${['tag'].concat(d.errors && d.errors.artists ? [] : [plural(artists.length, 'artist')], d.errors && d.errors.newReleases ? [] : [plural(albums.length, 'new release')]).join(', ')}</p>
              <h1 class="t-display lib-title" id="lib-tag-title">${tag.name}</h1>
              <p class="lib-bio">${tag.desc || 'No description yet. Tags come from MusicBrainz and Last.fm.'}</p>
              <div class="lib-actions">
                <button class="btn btn--primary" data-act="start-radio" data-kind="tag" data-id="${id}">${icon('radio')}Tag radio</button>
                <button class="btn lib-toggle" data-act="follow-tag" data-id="${id}" aria-pressed="${on ? 'true' : 'false'}">${icon(on ? 'check' : 'follow')}${on ? 'Following' : 'Follow tag'}</button>
              </div>
              ${related.length ? html`<div class="lib-chips"><span class="t-micro fg-2">related</span>${related.map(function (g) { return ui.tagChip(g); })}</div>` : ''}
            </div>
          </div>
        </section>
        ${artists.length || sectionError(d, 'artists', path) ? html`<section class="card lib-card" aria-labelledby="lib-tagartists-h">${ruleWith('top artists', 'lib-tagartists-h')}
          ${sectionError(d, 'artists', path) || html`<div class="shelf">${artists.map(artistTile)}</div>`}</section>` : ''}
        <section class="card lib-card" aria-labelledby="lib-tagrel-h">
          ${ruleWith('new releases', 'lib-tagrel-h', html`<span class="t-micro fg-2">the last 90 days, newest first</span>`)}
          ${sectionError(d, 'newReleases', path) || (albums.length ? html`<div class="lib-grid lib-grid--tight">${albums.map(albumTile)}</div>`
            : ui.emptyState({ art: 'moon', title: 'No new releases', text: 'Nothing new in this tag lately. Tag radio still finds songs on Soulseek.' }))}
        </section>
      </div>`;
    });
  }
  function mountTag(ctx) { liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']); }

  /* =====================================================================
     6. Liked, History, Following (library segments with their own routes)
     ===================================================================== */
  /* GET /api/likes gives the order (newest first) and the songs; the store says what is liked right now, so an
     unlike drops out at once and a new like arrives with the refetch mountLiked asks for */
  function likedPath() { return withLimit('/api/likes', 'liked'); }
  function renderLiked() {
    var path = likedPath(), r = M.get(path);
    return libraryFrame('liked', r.loading ? listSkeleton('Liked songs') : r.error ? App.errorCard(r.error, { path: path, label: 'Liked songs' }) : likedBody(r.data), path);
  }
  function mountLiked(ctx) {
    liveSaveButtons(ctx);
    liveOn(ctx, ['services']);
    liveOn(ctx, ['liked'], function () { M.reload(likedPath()); refreshKeepFocus(); });
  }
  function likedBody(d) {
    var ids = idList(d.items).filter(function (id) { return isLiked(id) && M.track(id); });
    if (!ids.length) {
      return html`<section class="card lib-card" aria-label="Liked songs">${ui.emptyState({ art: 'star', title: 'No liked songs yet',
        text: 'Tap the heart on any song, or press L while it plays. Likes also steer your recommendations.', action: { label: 'Find something to like', href: '#/home' } })}</section>`;
    }
    return html`<section class="card lib-card" aria-labelledby="lib-liked-h">
      <div class="lib-listhead">
        <span class="cover c5 lib-listhead__art" style="--cover-size-base:96px" aria-hidden="true"></span>
        <div class="lib-listhead__text">
          <h2 class="lib-listhead__title" id="lib-liked-h">liked songs</h2>
          <p class="lib-facts px">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</p>
          <div class="lib-bar">
            <button class="btn btn--primary" data-act="play-ids" data-ids="${ids.join(',')}">${icon('play', null, 'ic-play')}Play</button>
            <button class="btn" data-act="lib-shuffle-ids" data-ids="${ids.join(',')}">${icon('shuffle')}Shuffle</button>
            ${groupSaveButton(ids, 'liked')}
            ${ui.downloadButton({ kind: 'liked' }, Object.keys(App.store.get('liked') || {}))}
          </div>
        </div>
      </div>
      ${ui.trackList(ids, { label: 'Liked songs, newest first', unavailable: jellyfinDown() })}
      ${showAll('liked', d, 'liked song')}
    </section>`;
  }

  /* GET /api/history?tz=&before=: days ({ day: YYYY-MM-DD, items: plays newest first }), `next` = the cursor of the
     next page. "Older plays" adds a page below; a day split across two pages is merged into one. */
  var historyPages = [], historyBase = null;     // cursors of the older pages shown; the first page they follow
  function historyPath(before) {
    return '/api/history?tz=' + (-new Date().getTimezoneOffset()) + (before ? '&before=' + encodeURIComponent(before) : '');
  }
  function renderHistory() {
    var path = historyPath(), r = M.get(path);
    /* a refetched first page may end elsewhere, so the older pages that followed it no longer join up */
    if (r.data !== historyBase) { historyBase = r.data; historyPages = []; }
    return libraryFrame('history', r.loading ? listSkeleton('History') : r.error ? App.errorCard(r.error, { path: path, label: 'History' }) : historyBody(r.data), path);
  }
  /* the loaded pages' days merged, and how the list ends: the next cursor, a page loading, or a failed one */
  function historyDays(first) {
    var days = [], byDay = {}, next = first.next, tail = '';
    [first].concat(historyPages.map(function (cursor) {
      var path = historyPath(cursor), r = M.get(path);
      return r.loading ? { pending: ui.skeleton('row', 3) } : r.error ? { pending: readError(r.error, path) } : r.data;
    })).some(function (page) {
      if (page.pending) { tail = page.pending; next = null; return true; }
      (page.days || []).forEach(function (day) {
        if (byDay[day.day]) byDay[day.day].items = byDay[day.day].items.concat(day.items);
        else { byDay[day.day] = { day: day.day, items: day.items.slice() }; days.push(byDay[day.day]); }
      });
      next = page.next;
      return false;
    });
    return { days: days.filter(function (day) { return day.items.length; }), next: next, tail: tail };
  }
  function lastfmOff() { var lf = App.store.get('services').lastfm; return !!lf && lf.state === 'off'; }
  /* a pending scrobble counts as sent once Last.fm has nothing queued; null = never reported (no badge); with
     Last.fm not connected nothing is scrobbled, so no badges */
  function scrobbleOf(item) {
    var lf = App.store.get('services').lastfm;
    if (lastfmOff()) return null;
    return item.scrobble === 'pending' && lf && lf.state === 'ok' && !lf.queued ? 'done' : item.scrobble;
  }
  /* "Send to Last.fm": the hub says which plays it would take (`sendable`: lull's own, finished, never reported,
     inside Last.fm's rule and 14 days); the button also needs Last.fm connected with scrobbling on and this device
     not private, which the hub re-checks on every send */
  var SEND_MAX = 25;
  function canSend() {
    var lf = M.get('/api/lastfm').data;
    return !!lf && lf.connected && lf.scrobbling && !App.store.get('privateListening');
  }
  function sendBadge(i) {
    return html`<button type="button" class="btn btn--quiet btn--sm lib-send" data-act="lib-history-send" data-id="${i.playId}" aria-label="Send this play to Last.fm" data-tip="Never reached Last.fm. Send it, once">${icon('radio', null, 'ic-16')}Send to Last.fm</button>`;
  }
  /* the cached history pages hold the rows; a play's verdict is changed in them in place */
  function historyCached() {
    return [historyPath()].concat(historyPages.map(historyPath)).map(function (p) { return M.peek(p); }).filter(Boolean);
  }
  function setVerdict(playId, status) {
    var found = false;
    historyCached().forEach(function (page) {
      (page.days || []).forEach(function (day) {
        day.items.forEach(function (i) { if (i.playId === playId) { i.scrobble = status; i.sendable = false; found = true; } });
      });
    });
    return found;
  }
  App.on('server:scrobble', function (e) {
    if (setVerdict(String(e.playId), e.status) && App.current.path === '/history') refreshKeepFocus();
  });
  function sendToLastfm(ids, el) {
    if (el) el.disabled = true;
    return M.api('/api/history/lastfm', { method: 'POST', body: { playIds: ids } }).then(function () {
      ids.forEach(function (id) { setVerdict(id, 'pending'); });
      App.toast(plural(ids.length, 'play') + ' sent to Last.fm', { kind: 'ok', icon: 'check' });
      refreshKeepFocus();
    }, function (err) {
      if (el) el.disabled = false;
      App.toast('Not sent: ' + err.message, { kind: 'error' });
    });
  }
  function two(n) { return String(n).padStart(2, '0'); }
  function clockOf(at) { var d = new Date(at * 1000); return two(d.getHours()) + ':' + two(d.getMinutes()); }
  /* "Today" / "Yesterday" / "Wed 28 Oct" for a YYYY-MM-DD day (already split at the listener's midnight) */
  function dayLabel(iso) {
    var p = iso.split('-'), d = new Date(+p[0], +p[1] - 1, +p[2]), now = new Date();
    var diff = Math.round((new Date(now.getFullYear(), now.getMonth(), now.getDate()) - d) / 864e5);
    return diff === 0 ? 'Today' : diff === 1 ? 'Yesterday' : d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
  }
  function historyBody(first) {
    var h = historyDays(first), days = h.days;
    if (!days.length && !h.tail) {
      return html`<section class="card lib-card" aria-label="History">${ui.emptyState({ art: 'wave', title: 'Nothing played yet',
        text: 'Songs you play show up here by day, with their Last.fm scrobble status.', action: { label: 'Go home', href: '#/home' } })}</section>`;
    }
    var waiting = days.reduce(function (n, day) { return n + day.items.filter(function (i) { return scrobbleOf(i) === 'pending'; }).length; }, 0);
    var off = lastfmOff();
    var status = off
      ? html`${ui.statusGlyph('off', 'Not connected')}<p><b>Last.fm isn't connected, so plays aren't scrobbled.</b> Connect it in Settings to start.</p><a class="btn btn--sm" href="#/settings/lastfm">Set up</a>`
      : waiting
      ? html`${ui.statusGlyph('warn', 'Waiting')}<p><b>${plural(waiting, 'scrobble')} waiting for Last.fm.</b> They stay queued and retry on their own.</p><button class="btn btn--sm" data-act="lastfm-retry">${icon('refresh')}Retry now</button>`
      : html`${ui.statusGlyph('ok', 'Up to date')}<p><b>Every eligible play is scrobbled.</b> Songs under 30 seconds are skipped, as Last.fm asks.</p>`;
    var offer = !off && canSend() ? days.reduce(function (all, day) { return all.concat(day.items.filter(function (i) { return i.sendable; })); }, []) : [];
    var late = offer.length ? html`<div class="lib-status inset">${ui.statusGlyph('warn', 'Not sent')}<p><b>${plural(offer.length, 'play')} never reached Last.fm.</b> They ended before it was connected. Each can be sent once, with its own time.</p>
      <button class="btn btn--sm" data-act="lib-history-send-all" data-ids="${offer.slice(0, SEND_MAX).map(function (i) { return i.playId; }).join(',')}">${icon('radio', null, 'ic-16')}Send ${offer.length > SEND_MAX ? 'the newest ' + SEND_MAX : offer.length === 1 ? 'it' : 'all ' + offer.length}</button></div>` : '';
    return html`<section class="card lib-card" aria-labelledby="lib-history-h">
      ${ruleWith('history', 'lib-history-h', off ? '' : html`<ul class="lib-legend" aria-label="Scrobble status key">
        <li>${icon('check', null, 'ic-16')}scrobbled</li><li>${icon('clock', null, 'ic-16')}pending</li><li>${icon('dash', null, 'ic-16')}too short</li></ul>`)}
      <div class="lib-status inset">${status}</div>
      ${late}
      ${days.map(function (day) {
        var label = dayLabel(day.day), ids = day.items.map(function (i) { return i.trackId; });
        return html`<div class="lib-day">${ui.labelRule(label, { level: 'h3', sm: true })}
          <ol class="tlist lib-history" data-ids="${ids.join(',')}" aria-label="Played ${label}">${day.items.map(function (i) {
            var sc = scrobbleOf(i);
            return ui.trackRow(i.trackId, { index: clockOf(i.at), draggable: false, unavailable: jellyfinDown() && isSaved(i.trackId), badges: sc ? ui.scrobble(sc) : off || !i.sendable || !canSend() ? '' : sendBadge(i) });
          })}</ol></div>`;
      })}
      ${h.tail}
      ${h.next ? html`<p class="lib-more"><button class="btn btn--quiet btn--sm" data-act="lib-history-older" data-id="${h.next}">${icon('clock')}Older plays</button></p>` : ''}
    </section>`;
  }

  /* who is followed comes from the store (live); GET /api/follows carries the artists' entities */
  function renderFollowing() {
    var path = '/api/follows', r = M.get(path);
    return libraryFrame('following', r.loading ? html`<div class="lib-cols">${listSkeleton('Artists you follow')}${listSkeleton('Tags you follow')}</div>`
      : r.error ? App.errorCard(r.error, { path: path, label: 'Following' }) : followingBody(), path);
  }
  function followingBody() {
    var ids = Object.keys(App.store.get('followed'));
    /* the same artist-or-tag rule App.toggleFollow uses */
    var artists = ids.filter(function (id) { return /^ar-/.test(id); }).map(M.artist).filter(Boolean);
    var tags = ids.filter(function (id) { return !/^ar-/.test(id); }).map(M.tag);
    /* tags to try: what the artists you follow are tagged with, that you don't follow yet */
    var suggest = [];
    artists.forEach(function (a) { a.tags.forEach(function (t) { if (!App.isFollowed(t) && suggest.indexOf(t) < 0) suggest.push(t); }); });
    return html`<div class="lib-cols">
      <section class="card lib-card" aria-labelledby="lib-fa-h">
        ${ruleWith('artists you follow', 'lib-fa-h', artists.length ? html`<span class="lib-count">${artists.length}</span>` : '')}
        ${artists.length ? html`<ul class="lib-flist">${artists.map(function (ar) {
          return html`<li class="lib-frow">${ui.cover(ar, 48, { round: true })}
            <div class="lib-frow__main"><a class="lib-frow__name" href="#/artist/${ar.id}">${ar.name}</a><span class="lib-frow__meta">${ar.tags.join(', ')}</span></div>
            ${ui.iconButton('radio', 'Start ' + ar.name + ' radio', { 'data-act': 'start-radio', 'data-kind': 'artist', 'data-id': ar.id, cls: 'btn-icon--bare' })}
            ${followButton(ar.id)}</li>`;
        })}</ul>` : ui.emptyState({ art: 'moon', title: 'No artists followed', text: 'Follow an artist and their new releases appear on Home first.', action: { label: 'Browse artists', href: '#/library/artists' } })}
      </section>
      <section class="card lib-card" aria-labelledby="lib-ft-h">
        ${ruleWith('tags you follow', 'lib-ft-h', tags.length ? html`<span class="lib-count">${tags.length}</span>` : '')}
        ${tags.length ? html`<ul class="lib-flist">${tags.map(function (t) {
          return html`<li class="lib-frow lib-frow--tag">
            <div class="lib-frow__main"><a class="lib-frow__name" href="#/tag/${encodeURIComponent(t.id)}">${t.name}</a><span class="lib-frow__meta">${t.desc}</span></div>
            ${ui.iconButton('radio', 'Start ' + t.name + ' radio', { 'data-act': 'start-radio', 'data-kind': 'tag', 'data-id': t.id, cls: 'btn-icon--bare' })}
            <button class="btn lib-toggle" data-act="follow-tag" data-id="${t.id}" aria-pressed="true">${icon('check')}Following</button></li>`;
        })}</ul>` : ui.emptyState({ art: 'star', title: 'No tags followed', text: 'Follow a few tags to steer discovery toward what you like.' })}
        ${suggest.length ? html`<div class="lib-suggest">${ui.labelRule('more tags to try', { level: 'h3', sm: true })}
          <div class="lib-chips">${suggest.map(function (t) { return ui.tagChip(t, { toggle: true, pressed: false }); })}</div></div>` : ''}
      </section>
    </div>`;
  }

  /* =====================================================================
     6b. Downloads (/downloads): what this device keeps for offline (App.downloads, offline.js), with sizes,
         the device's storage, and removal
     ===================================================================== */
  var DL_KIND = { album: 'album', playlist: 'playlist', liked: 'liked songs', track: 'song' };
  function keptName(k) {
    if (k.kind === 'liked') return 'Liked songs';
    if (k.kind === 'album') { var a = M.album(k.id); return a ? a.title + ', ' + a.artist : 'An album'; }
    if (k.kind === 'playlist') { var p = playlist(k.id); return p ? p.name : 'A playlist'; }
    var t = M.track(k.id); return t ? t.title + ', ' + t.artist : 'A song';
  }
  function keptArt(k) {
    if (k.kind === 'album' && M.album(k.id)) return ui.cover(M.album(k.id), 48);
    if (k.kind === 'track' && M.track(k.id)) return ui.cover(M.track(k.id), 48);
    if (k.kind === 'liked') return html`<span class="cover c5" style="--cover-size-base:48px" aria-hidden="true"></span>`;
    return ui.mosaic(k.ids, 48, '');
  }
  function bytes(n) { return fmt.size((n || 0) / 1e6); }
  function keptRow(k) {
    var D = App.downloads, cov = D.coverage(k.ids), size = k.ids.reduce(function (n, id) { var r = D.state(id); return n + (r && r.state === 'done' ? r.sizeBytes || 0 : 0); }, 0);
    var failed = k.ids.filter(function (id) { var r = D.state(id); return r && r.state === 'failed'; });
    var line = cov.busy ? 'Downloading, ' + cov.done + ' of ' + cov.total : plural(cov.done, 'song') + ', ' + bytes(size);
    return html`<li class="lib-kept__row">
      <span class="lib-kept__art">${keptArt(k)}</span>
      <span class="lib-kept__text"><span class="lib-kept__name">${keptName(k)}</span>
        <span class="lib-kept__sub"><span class="lib-kept__kind">${DL_KIND[k.kind]}</span>${line}${failed.length ? html`, <span class="field__error">${failed.length} failed</span>` : ''}</span></span>
      <span class="lib-kept__acts">${failed.length ? html`<button class="btn btn--sm" data-act="lib-dl-retry" data-ids="${failed.join(',')}">Retry</button>` : ''}
        <button class="btn btn--sm" data-act="lib-dl-unkeep" data-id="${k.key}" aria-label="Remove ${keptName(k)} from this device">${icon('trash', null, 'ic-16')}Remove</button></span></li>`;
  }
  function downloadsBody() {
    var D = App.downloads, why = D.unavailableReason();
    var done = D.list().filter(function (r) { return r.state === 'done' && M.track(r.trackId); }).map(function (r) { return r.trackId; });
    var kept = D.kept().filter(function (k) { return k.kind !== 'track'; });
    if (!done.length && !D.list().length) {
      return html`<section class="card lib-card" aria-label="Downloads">${why ? html`<p class="sys-note sys-note--warn">${icon('lock', null, 'ic-16')}<span>${why}</span></p>` : ''}
        ${ui.emptyState({ art: 'wave', title: 'Nothing downloaded yet', text: 'Use Download on an album, a playlist, Liked songs or any song, and it plays here even when your hub can\'t be reached.', action: { label: 'Go to Liked songs', href: '#/liked' } })}</section>`;
    }
    return html`<section class="card lib-card" aria-labelledby="lib-dl-h">
      <div class="lib-listhead">
        <span class="cover c3 lib-listhead__art" style="--cover-size-base:96px" aria-hidden="true"></span>
        <div class="lib-listhead__text">
          <h2 class="lib-listhead__title" id="lib-dl-h">downloads</h2>
          <p class="lib-facts px">${plural(done.length, 'song')}, ${bytes(D.usedBytes())} on this device</p>
          <p class="lib-dl-storage" id="lib-dl-storage" aria-live="polite"></p>
          <div class="lib-bar">
            <button class="btn btn--primary" data-act="play-ids" data-ids="${done.join(',')}" ${done.length ? '' : raw('aria-disabled="true"')}>${icon('play', null, 'ic-play')}Play</button>
            <button class="btn" data-act="lib-shuffle-ids" data-ids="${done.join(',')}">${icon('shuffle')}Shuffle</button>
            <button class="btn btn--quiet" data-act="lib-dl-clear">${icon('trash')}Remove all</button>
          </div>
        </div>
      </div>
      ${why ? html`<p class="sys-note sys-note--warn">${icon('lock', null, 'ic-16')}<span>${why}</span></p>` : ''}
      ${kept.length ? html`${ruleWith('kept on this device', 'lib-dl-kept-h', html`<span class="t-micro fg-2">follow their source while online</span>`)}
        <ul class="lib-kept" aria-labelledby="lib-dl-kept-h">${kept.map(keptRow)}</ul>` : ''}
      ${ruleWith('songs', 'lib-dl-songs-h', '')}
      ${ui.trackList(done, { label: 'Downloaded songs' })}
    </section>`;
  }
  function renderDownloads() { return libraryFrame('downloads', downloadsBody(), ''); }
  /* the device's own storage: what the browser lets lull use (or the app's free space) */
  function showStorage() {
    App.downloads.estimate().then(function (e) {
      var el = $('#lib-dl-storage'); if (!el || !e) return;
      el.textContent = bytes(e.freeBytes) + ' free for downloads on this device';
    });
  }
  function mountDownloads(ctx) {
    showStorage();
    var t = null;
    ctx.onCleanup(App.on('downloads', function () { clearTimeout(t); t = setTimeout(function () { refreshKeepFocus(); showStorage(); }, 250); }));
    liveOn(ctx, ['playlists']);
  }
  App.action('lib-dl-unkeep', function (el) { App.downloads.unkeep(el.getAttribute('data-id')).then(function () { App.toast('Removed from this device', { icon: 'trash' }); }); });
  App.action('lib-dl-retry', function (el) { App.downloads.retry(el.getAttribute('data-ids').split(',')); });
  App.action('lib-dl-clear', function () {
    App.confirm({ title: 'remove every download?', text: 'All songs leave this device. They still play from your hub when it can be reached.', confirm: 'Remove all', danger: true }, function () {
      App.downloads.remove(App.downloads.list().map(function (r) { return r.trackId; })).then(function () { App.toast('Downloads removed', { icon: 'trash' }); });
    });
  });

  /* =====================================================================
     7. Playlists list (store `playlists`; `playlistsError` once a load failed, null once one succeeded)
     ===================================================================== */
  function playlistTile(p) {
    var ids = plIds(p), key = 'playlist:' + p.id;
    return html`<div class="tile lib-tile lib-tile--pl${ui.placeCls(key)}" data-drop="playlist:${p.id}" data-place="${key}">
      <div class="tile__art"><a href="#/playlist/${p.id}" aria-label="${p.name}, ${plural(ids.length, 'song')}">${ui.playlistArt(p, '100%')}</a>
        <button class="tile__play" data-act="play-playlist" data-id="${p.id}" aria-label="Play ${p.name}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/playlist/${p.id}" data-place-at>${ui.placeMark(key)}${p.name}</a></div>
      <div class="tile__by">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</div>
      ${p.updated ? html`<div class="lib-tile__meta t-micro">updated ${fmt.date(p.updated)}</div>` : ''}
    </div>`;
  }
  function dropZone() {
    return html`<div class="lib-dropzone" data-lib-dropzone>
      ${icon('import', null, 'ic-24')}
      <p><b>Drop an .m3u, .m3u8 or Exportify .csv file</b> anywhere in the window to import it.</p>
      <button class="btn btn--sm" data-act="lib-import-pick">Choose file</button>
    </div>`;
  }
  function renderPlaylists() {
    var list = playlists(), err = App.store.get('playlistsError');
    var songs = list.reduce(function (n, p) { return n + plIds(p).length; }, 0);
    var tools = html`<button class="btn btn--primary" data-act="lib-pl-new">${icon('plus')}New playlist</button>
      <button class="btn" data-act="lib-import">${icon('import')}Import playlist</button>`;
    var body;
    if (err) body = ui.emptyState({ art: 'star', title: 'Couldn\'t load your playlists', text: err, action: { label: 'Try again', act: 'lib-pl-reload' } });
    else if (err === undefined) body = gridSkeleton(4);      // the first load hasn't answered yet
    else if (list.length) {
      body = html`<div class="lib-grid">
        <button class="lib-newtile" data-act="lib-pl-new"><span class="lib-newtile__box">${icon('plus', null, 'ic-24')}</span><span class="tile__title">New playlist</span><span class="tile__by">name it, then add songs</span></button>
        ${list.slice().sort(newestFirst).map(playlistTile)}
      </div>`;
    } else {
      body = ui.emptyState({ art: 'slot', title: 'No playlists yet', text: 'Make a playlist and drag songs into it, or import an M3U from another player.',
        action: { label: 'New playlist', act: 'lib-pl-new' }, secondary: { label: 'Import a playlist', act: 'lib-import' } });
    }
    return html`<div class="view__inner lib">
      ${pageHead('playlists', list.length ? plural(list.length, 'playlist') + ', ' + plural(songs, 'song') + '. Drag songs onto a playlist to add them.' : 'Make one, or bring one over from another player.', tools)}
      <section class="card lib-card" aria-labelledby="lib-pls-h">
        ${ruleWith('your playlists', 'lib-pls-h', html`<span class="t-micro fg-2">recently updated first</span>`)}
        ${body}
        ${bookmarksShelf()}
        <div class="lib-link-card">${linkForm()}</div>
        ${dropZone()}
      </section>
    </div>`;
  }
  function mountPlaylists(ctx) { liveOn(ctx, ['playlists', 'playlistsError']); }

  /* =====================================================================
     7b. Outside playlists (docs/context/collection.md#outside-playlists): someone's playlist on YouTube or Deezer,
         opened from search. It previews like an album (GET /api/outside?link=), every song playable at once (the
         catalog's match, else the song as the playlist names it); songs not matched yet arrive as `outside` events.
         Save bookmarks it (PUT /api/bookmarks?link=), like following a playlist; Make a copy is the link import.
     ===================================================================== */
  var SITE = { youtube: 'YouTube', deezer: 'Deezer', spotify: 'Spotify', soundcloud: 'SoundCloud' };
  function outsidePath(key) { return '/api/outside?link=' + encodeURIComponent(key); }
  function outsideHref(key) { return '#/outside/' + encodeURIComponent(key); }
  function bookmarks() { var r = M.get('/api/bookmarks'); return r.data ? r.data.items || [] : []; }
  function bookmarksShelf() {
    var items = bookmarks();
    if (!items.length) return '';
    return html`${ruleWith('saved from youtube and deezer', 'lib-bm-h', html`<span class="t-micro fg-2">bookmarks: they follow their source</span>`)}
      <div class="lib-grid" aria-labelledby="lib-bm-h">${items.map(bookmarkTile)}</div>`;
  }
  function bookmarkTile(b) {
    var site = SITE[b.site] || b.site, key = 'outside:' + b.id;
    return html`<div class="tile lib-tile lib-tile--pl lib-tile--outside${ui.placeCls(key)}" data-place="${key}">
      <div class="tile__art"><a href="${outsideHref(b.id)}" aria-label="${b.name}, ${site} playlist, ${plural(b.total, 'song')}">${ui.mosaic([], '100%', '', b.cover)}</a>
        <span class="badge lib-site" data-site="${b.site}">${site}</span></div>
      <div class="tile__title"><a href="${outsideHref(b.id)}" data-place-at>${ui.placeMark(key)}${b.name}</a></div>
      <div class="tile__by">${b.owner ? b.owner + ', ' : ''}${plural(b.total, 'song')}</div>
    </div>`;
  }
  var OUT_STATUS = { matched: '', pending: 'Matching', unmatched: 'As named' };
  function renderOutside(ctx) {
    var key = ctx.params.id, path = outsidePath(key), r = M.get(path), d = r.data;
    if (!d) {
      if (r.loading) return pageSkeleton();
      return r.error && r.error.status !== 404 ? pageError(r.error, path) : notFound('playlist');
    }
    var odd = d.songs.filter(function (s) { return !(s.status in OUT_STATUS); })[0];
    if (odd) return pageError(new M.ApiError(0, 'bad_outside', 'The server sent a song status lull doesn\'t understand (' + odd.status + ').'), path);
    var ids = d.songs.map(function (s) { return s.trackId; }), site = SITE[d.site] || d.site;
    var named = d.songs.filter(function (s) { return s.status === 'unmatched'; }).length;
    var pending = d.songs.filter(function (s) { return s.status === 'pending'; }).length;
    var facts = plural(d.total, 'song') + (d.duration ? ', ' + fmt.length(d.duration) : '');
    return html`<div class="view__inner lib">
      <article class="card card--flush lib-page lib-pl lib-out" aria-labelledby="lib-out-title" data-meta="${site} playlist">
        <header class="lib-pl__head">
          ${ui.mosaic(ids, 'var(--lib-art)', 'Playlist cover: ' + d.name, d.cover)}
          <div class="lib-hero__info">
            <p class="lib-kicker">playlist on ${site}</p>
            <h1 class="t-title-xl lib-pl__title" id="lib-out-title">${d.name}</h1>
            ${d.owner ? html`<p class="lib-pl__by">by ${d.owner}</p>` : ''}
            ${d.description ? html`<p class="lib-bio">${d.description}</p>` : ''}
            <p class="lib-pl__facts"><span class="px">${facts}</span>
              <span class="lib-pl__upd">${pending ? 'matching ' + plural(pending, 'song') + ' to lull\'s catalog' : named ? plural(named, 'song') + ' not in lull\'s catalog play as named' : 'every song found in lull\'s catalog'}</span></p>
            ${d.note ? html`<p class="lib-help">${icon('info', null, 'ic-16')}${d.note}</p>` : ''}
          </div>
        </header>
        ${previewNotice(d, site)}
        <div class="lib-pl__bar">
          <button class="btn btn--primary" data-act="play-ids" data-ids="${ids.join(',')}">${icon('play', null, 'ic-play')}Play</button>
          ${ui.iconButton('shuffle', 'Shuffle play', { 'data-act': 'lib-shuffle-ids', 'data-ids': ids.join(',') })}
          ${bookmarkButton(d.id, d.bookmarked)}
          ${d.playlistId ? html`<a class="btn" href="#/playlist/${d.playlistId}">${icon('playlists')}Open your playlist</a>
              <button class="btn btn--quiet" data-act="lib-out-copy" data-id="${d.url}" data-tip="Your playlist of it takes the songs it has now">${icon('refresh')}Update it</button>`
            : html`<button class="btn" data-act="lib-out-copy" data-id="${d.url}" data-tip="Your own playlist of its songs, which you can edit">${icon('import')}Import as playlist</button>`}
          <a class="btn btn--quiet" href="${d.url}" target="_blank" rel="noopener noreferrer">${icon('link')}Open on ${site}</a>
        </div>
        <div class="lib-table">
          <ol class="tlist lib-out-list" data-ids="${ids.join(',')}" aria-label="Songs in ${d.name}">${d.songs.map(function (s, n) {
            var label = OUT_STATUS[s.status];
            var badge = label ? html`<span class="badge lib-out__status" data-status="${s.status}" data-tip="${s.status === 'pending' ? 'Not matched to lull\'s catalog yet: plays as the playlist names it meanwhile' : 'Not in lull\'s catalog: plays as the playlist names it (' + s.artist + ', ' + s.title + ')'}">${label}</span>` : '';
            return ui.trackRow(s.trackId, { index: n + 1, draggable: false, badges: badge });
          })}</ol>
        </div>
      </article>
    </div>`;
  }
  /* what this page is, said plainly: a preview keeps nothing; Add to library follows it; Import as playlist copies it */
  function previewNotice(d, site) {
    if (d.playlistId) {
      return html`<p class="lib-notice lib-out__notice">${icon('check', null, 'ic-16')}<span>You imported this ${site} playlist: it is in Library, Playlists as your own playlist. Update it takes the songs it has now.</span></p>`;
    }
    return d.bookmarked
      ? html`<p class="lib-notice lib-out__notice">${icon('check', null, 'ic-16')}<span>In your library: Library, Playlists shows it, and it follows ${site}. Its songs aren't saved; Import as playlist makes your own copy${saveSongsPref() ? ' and saves them' : ''}.</span></p>`
      : html`<p class="lib-notice lib-out__notice">${icon('info', null, 'ic-16')}<span>A preview of a ${site} playlist: it isn't in your library, and playing it keeps nothing. Add to library follows it; Import as playlist makes your own copy${saveSongsPref() ? ' and saves its songs' : ''}.</span></p>`;
  }
  function bookmarkButton(key, on) {
    return on ? html`<button class="btn" data-act="lib-out-save" data-id="${key}" aria-pressed="true" data-tip="Remove from your library">${icon('check')}In your library</button>`
      : html`<button class="btn" data-act="lib-out-save" data-id="${key}" aria-pressed="false">${icon('plus')}Add to library</button>`;
  }
  App.outside = { href: outsideHref, saved: isBookmarked, site: function (s) { return SITE[s] || s; }, button: bookmarkButton };
  function mountOutside(ctx) {
    var key = ctx.params.id, path = outsidePath(key);
    liveOn(ctx, ['liked', 'services']);
    liveSaveButtons(ctx);
    /* playing a preview says once that it keeps nothing, with the way to keep it */
    var told = false;
    ctx.el.addEventListener('click', function (e) {
      var d = M.peek(path);
      if (told || !d || d.bookmarked || !e.target.closest('[data-act="play-ids"], [data-act="play-track"], [data-act="lib-shuffle-ids"]')) return;
      told = true;
      App.toast('Playing a preview: it isn\'t in your library.', { icon: 'info', action: 'Add to library', onAction: function () { toggleBookmark(d.id); } });
    });
    /* matches arriving: the rows take the catalog's song (app.js has absorbed the event's entities) */
    ctx.onCleanup(App.on('server:outside', function (e) {
      var d = M.peek(path); if (!e || !d || e.id !== d.id) return;
      (e.songs || []).forEach(function (u) { var s = d.songs[u.pos]; if (s) { s.status = u.status; s.trackId = u.trackId; } });
      refreshKeepFocus();
    }));
  }
  function isBookmarked(key) { return bookmarks().some(function (b) { return b.id === key; }); }
  function toggleBookmark(key) {
    var path = outsidePath(key), d = M.peek(path), on = d ? !!d.bookmarked : isBookmarked(key);
    return M.api('/api/bookmarks?link=' + encodeURIComponent(key), { method: on ? 'DELETE' : 'PUT' }).then(function () {
      if (d) d.bookmarked = !on;
      M.invalidate('/api/bookmarks');
      App.refresh();
      App.toast(on ? 'Removed from your library' : 'Added to Library, Playlists. It follows ' + (SITE[d && d.site] || 'its source') + '; its songs play from Soulseek.', { icon: on ? 'close' : 'check' });
    }, function (err) { App.toast('Couldn\'t ' + (on ? 'remove' : 'save') + ' it: ' + err.message, { kind: 'error', icon: 'warning' }); });
  }
  App.action('lib-out-save', function (el) { toggleBookmark(el.getAttribute('data-id')); });
  App.action('lib-out-copy', function (el) { importLink(el.getAttribute('data-id')); });
  /* another device saved or removed one */
  App.on('server:bookmark', function (e) {
    var d = e && M.peek(outsidePath(e.id));
    if (d) d.bookmarked = !!e.saved;
    M.invalidate('/api/bookmarks');
    App.refresh();
  });

  /* =====================================================================
     8. Playlist page (moodboard image 04): mosaic, big title, pixel-face count + duration, table.
        The store copy is live; GET /api/playlists/{id} fills in its songs (and stands in when the store lacks it).
     ===================================================================== */
  /* How a playlist is shown: its own order (custom; descending is "Reversed") or a column, either way round. It is
     the playlist's `sort` on the hub (PUT /api/playlists/{id}/sort), so it is the same on every device and after a
     reload. Each key compares ascending; ties keep the playlist's own order. A new key starts the way people
     expect it: dates newest first, the rest A to Z or shortest first. */
  var text = function (f) { return function (a, b) { return String(f(a) || '').localeCompare(String(f(b) || ''), undefined, { sensitivity: 'base', numeric: true }); }; };
  var SORTS = {
    custom: { label: 'Custom order', reversed: 'Reversed', first: 'asc', fn: function () { return 0; } },
    added: { label: 'Date added', first: 'desc', fn: function (a, b) { return stamp(a.it.added) - stamp(b.it.added); } },
    title: { label: 'Title', first: 'asc', fn: text(function (r) { return r.t.title; }) },
    artist: { label: 'Artist', first: 'asc', fn: text(function (r) { return r.t.artist; }) },
    album: { label: 'Album', first: 'asc', fn: text(function (r) { return r.t.album; }) },
    duration: { label: 'Duration', first: 'asc', fn: function (a, b) { return (a.t.duration || 0) - (b.t.duration || 0); } }
  };
  var DEFAULT_SORT = { key: 'custom', dir: 'asc' };
  function sortOf(p) { var s = p && p.sort; return s && SORTS[s.key] && (s.dir === 'asc' || s.dir === 'desc') ? s : DEFAULT_SORT; }
  /* drag, Alt + arrows and Remove work on the playlist's own order, so only while it shows that order */
  function inOwnOrder(p) { var s = sortOf(p); return s.key === 'custom' && s.dir === 'asc'; }
  /* "Custom order", "Reversed", "Date added, newest first", "Title, A to Z", "Duration, longest first"; `inSentence`
     lowers the column's name ("Sorted by title, A to Z") */
  function sortLabel(s, inSentence) {
    if (s.key === 'custom') return s.dir === 'desc' ? SORTS.custom.reversed : SORTS.custom.label;
    var words = s.key === 'added' ? ['oldest first', 'newest first'] : s.key === 'duration' ? ['shortest first', 'longest first'] : ['A to Z', 'Z to A'];
    var name = SORTS[s.key].label;
    return (inSentence ? name.toLowerCase() : name) + ', ' + words[s.dir === 'asc' ? 0 : 1];
  }
  function playlistOf(id) { return playlist(id) || M.peek(playlistPath(id)); }
  /* the rows on screen, in order, each with its index in p.items: songs the cache can't describe aren't
     drawn, but they stay in the playlist (every order sent is built from p.items, not from the screen) */
  function shownRows(p) {
    var s = sortOf(p), sign = s.dir === 'desc' ? -1 : 1;
    var rows = (p.items || []).map(function (it, i) { return { it: it, i: i, t: M.track(it.trackId) }; }).filter(function (r) { return r.t; });
    return rows.sort(function (a, b) { return sign * (SORTS[s.key].fn(a, b) || a.i - b.i); });
  }
  /* screen row n -> its index in p.items (past the last row: the end of the playlist) */
  function rowToItem(p, shown) { return function (n) { return n < shown.length ? shown[n].i : plIds(p).length; }; }
  /* a column header: the sorted one says which way (aria-sort, and an arrow); clicking it again turns it round */
  function headCell(p, key, label, cls) {
    var s = sortOf(p), on = s.key === key, next = on ? (s.dir === 'asc' ? 'desc' : 'asc') : SORTS[key].first;
    var name = key === 'custom' ? 'playlist order' : SORTS[key].label.toLowerCase();
    return html`<span class="lib-th ${cls}" role="columnheader" aria-sort="${on ? (s.dir === 'asc' ? 'ascending' : 'descending') : 'none'}">
      <button class="lib-th__btn" data-act="lib-pl-sort" data-id="${p.id}" data-value="${key}" data-dir="${next}" aria-label="${on ? 'Sorted by ' + name + ', ' + (s.dir === 'asc' ? 'ascending' : 'descending') + '. Reverse' : 'Sort by ' + name}">${label}${on ? icon(s.dir === 'asc' ? 'chevron-up' : 'chevron-down', null, 'ic-16') : ''}</button></span>`;
  }
  /* shows the playlist sorted at once and keeps it on the hub; if the hub can't keep it, it stays sorted here and
     says it won't be next time */
  function setSort(p, sort) {
    var before = sortOf(p);
    if (before.key === sort.key && before.dir === sort.dir) return;
    App.setPlaylistSort(p.id, sort);
    App.announce(sort.key === 'custom' ? sortLabel(sort) : 'Sorted by ' + sortLabel(sort, true));
  }
  function renderPlaylist(ctx) {
    var id = ctx.params.id, path = playlistPath(id), r = M.get(path), p = playlist(id) || r.data;
    if (!p) {
      if (r.loading) return pageSkeleton();
      return r.error && r.error.status !== 404 ? pageError(r.error, path) : notFound('playlist');
    }
    var ids = plIds(p);
    if (r.loading && !ids.every(function (x) { return M.track(x); })) return pageSkeleton();
    var rows = shownRows(p), custom = inOwnOrder(p), sort = sortOf(p), down = jellyfinDown();
    var missing = ids.filter(function (x) { return !isSaved(x); }).length;
    var table = ids.length ? html`<div class="lib-table">
        <div role="table" aria-label="Sort songs"><div class="lib-thead" role="row">
          ${headCell(p, 'custom', '#', 'lib-th--num')}${headCell(p, 'title', 'title', 'lib-th--title')}${headCell(p, 'album', 'album', 'lib-th--album')}
          ${headCell(p, 'added', 'date added', 'lib-th--added')}<span class="lib-th lib-th--like" role="columnheader">${icon('heart', 'Liked', 'ic-16')}</span>
          ${headCell(p, 'duration', icon('clock', 'Duration', 'ic-16'), 'lib-th--dur')}
        </div></div>
        <ol class="tlist tlist--dated lib-pl-list" data-ids="${rows.map(function (x) { return x.it.trackId; }).join(',')}" data-drop="lib-pl:${p.id}" data-pl="${p.id}"
          aria-label="Songs in ${p.name}${custom ? '. Alt plus arrow keys reorder, Delete removes' : ''}">${rows.map(function (x, n) {
          return ui.trackRow(x.t, { index: n + 1, added: x.it.added, draggable: custom, unavailable: down && isSaved(x.t.id) });
        })}</ol>
      </div>`
      : html`<div class="lib-dropzone lib-dropzone--songs" data-drop="playlist:${p.id}">
          ${ui.emptyState({ art: 'slot', title: 'Drag songs here',
            text: 'Drop songs from search, an album or the queue, or use Add to playlist on any song.', action: { label: 'Find songs', href: '#/search' }, secondary: { label: 'Import a playlist', act: 'lib-import' } })}
        </div>`;
    return html`<div class="view__inner lib">
      ${jellyfinNotice(path)}
      <article class="card card--flush lib-page lib-pl" aria-labelledby="lib-pl-title" data-meta="${playlistMeta(p)}">
        <header class="lib-pl__head">
          ${ui.playlistArt(p, 'var(--lib-art)', 'Playlist cover: ' + p.name)}
          <div class="lib-hero__info">
            <p class="lib-kicker">playlist</p>
            <h1 class="t-title-xl lib-pl__title" id="lib-pl-title">${p.name}</h1>
            ${p.desc ? html`<p class="lib-bio">${p.desc}</p>` : html`<button class="btn btn--quiet btn--sm lib-adddesc" data-act="lib-pl-edit" data-id="${p.id}">${icon('edit')}Add a description</button>`}
            <p class="lib-pl__facts"><span class="px">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</span>${p.updated ? html`<span class="lib-pl__upd">updated ${fmt.date(p.updated, true)}</span>` : ''}</p>
          </div>
        </header>
        <div class="lib-pl__bar">
          <button class="btn btn--primary" data-act="play-playlist" data-id="${p.id}" ${ids.length ? '' : raw('aria-disabled="true"')}>${icon('play', null, 'ic-play')}Play</button>
          ${ui.iconButton('shuffle', 'Shuffle play', { 'data-act': 'lib-shuffle-ids', 'data-ids': ids.join(',') })}
          ${ids.length && missing ? groupSaveButton(ids, 'playlist') : ''}
          ${ids.length ? ui.downloadButton({ kind: 'playlist', id: p.id }, ids) : ''}
          ${ui.iconButton('edit', 'Edit name, description and cover', { 'data-act': 'lib-pl-edit', 'data-id': p.id, 'aria-haspopup': 'dialog' })}
          ${ui.iconButton('export', 'Export as M3U', { 'data-act': 'lib-pl-export', 'data-id': p.id, 'aria-haspopup': 'dialog' })}
          ${ui.iconButton('more', 'More playlist actions', { 'data-act': 'lib-pl-menu', 'data-id': p.id, 'aria-haspopup': 'menu' })}
          <span class="spacer"></span>
          ${ids.length ? sortButton('lib-pl-sortmenu', sortLabel(sort), p.id) : ''}
        </div>
        ${!custom && ids.length ? html`<p class="lib-sortnote">${icon('info', null, 'ic-16')}${sort.key === 'custom' ? 'Shown reversed.' : 'Sorted by ' + sortLabel(sort, true) + '.'} Dragging, moving and removing songs work on the playlist's own order: <button class="btn btn--quiet btn--sm" data-act="lib-pl-sort" data-id="${p.id}" data-value="custom" data-dir="asc">Show custom order</button></p>` : ''}
        ${table}
      </article>
    </div>`;
  }

  /* playlist changes: the server takes the full new order (App.savePlaylistOrder) and answers with the playlist,
     which lands in the store, so this page and the rail re-render with what the server holds. One change at a
     time: a move made while the last one is in flight would be computed from an order that's about to change. */
  var saving = false;
  function saveOrder(p, ids) {
    if (saving) return Promise.resolve(false);
    saving = true;
    return App.savePlaylistOrder(p.id, ids).then(function () { saving = false; return true; }, function (err) {
      saving = false;
      App.toast('Couldn\'t change ' + p.name + ': ' + err.message, { kind: 'error' });
      return false;
    });
  }
  function focusRow(i) {
    var rows = $$('.lib-pl-list .trow'), r = rows[App.clamp(i, 0, rows.length - 1)];
    if (r) { rows.forEach(function (x) { x.classList.remove('is-selected'); }); r.classList.add('is-selected'); r.focus(); }
  }
  /* move the items at `from` (indices into p.items) so they land before item index `to` */
  function moveItems(p, from, to) {
    var ids = plIds(p), moving = from.map(function (i) { return ids[i]; });
    var rest = ids.filter(function (x, i) { return from.indexOf(i) < 0; }), at = to - from.filter(function (i) { return i < to; }).length;
    rest.splice.apply(rest, [at, 0].concat(moving));
    saveOrder(p, rest).then(function (ok) {
      if (!ok) return;
      App.announce('Moved to position ' + (at + 1) + ' of ' + rest.length);
      focusRow(at);
    });
  }
  function insertItems(p, ids, at) {
    var before = plIds(p), next = before.slice();
    next.splice.apply(next, [at, 0].concat(ids));
    saveOrder(p, next).then(function (ok) {
      if (ok) App.toast('Added ' + plural(ids.length, 'song') + ' to ' + p.name, { icon: 'playlists', action: 'Undo', onAction: function () { saveOrder(p, before); } });
    });
  }
  /* remove item index i; `row` is where it was on screen, so focus stays there */
  function removeItem(p, i, row) {
    var before = plIds(p), next = before.slice(), t = M.track(before[i]);
    next.splice(i, 1);
    saveOrder(p, next).then(function (ok) {
      if (!ok) return;
      focusRow(row);
      App.toast('Removed ' + (t ? t.title : 'a song') + ' from ' + p.name, { icon: 'trash', action: 'Undo',
        onAction: function () { saveOrder(p, before).then(function (back) { if (back) focusRow(row); }); } });
    });
  }
  /* a new playlist with songs in it, in one POST (App.createPlaylist says why when it fails, and answers null).
     `added`: when each song was added (unix seconds or null), so a copy or an import keeps "Date added" */
  function makePlaylist(name, desc, ids, imported, added) {
    return App.createPlaylist({ name: name, desc: desc || '', trackIds: ids, imported: imported, added: added });
  }
  function addedOf(p) { return (p.items || []).map(function (i) { return i.added; }); }
  function saveDetails(p, name, desc) { return App.updatePlaylist(p.id, { name: name, desc: desc }); }
  function deletePlaylist(p) {
    var ids = plIds(p);
    App.deletePlaylist(p.id).then(function () {
      M.invalidate(playlistPath(p.id));
      if (App.current.path === '/playlist/' + p.id) App.go('/playlists');
      /* undo makes it again from what it held (a new playlist, same name, description and songs) */
      App.toast('Deleted ' + p.name, { icon: 'trash', action: 'Undo', onAction: function () {
        makePlaylist(p.name, p.desc, ids, undefined, addedOf(p)).then(function (back) { if (back) App.go('/playlist/' + back.id); });
      } });
    }, function (err) { App.toast('Couldn\'t delete ' + p.name + ': ' + err.message, { kind: 'error' }); });
  }

  /* drag reorder (custom order only): an insertion line follows the pointer; the drop itself arrives
     through the shell's 'drop' event because the list is a [data-drop] target. Screen row n is item shown[n].i. */
  function mountPlaylist(ctx) {
    var p = playlistOf(ctx.params.id); if (!p) return;
    liveOn(ctx, ['playlists', 'services']);   // a like changes only its heart, which App.syncRows flips in place
    liveSaveButtons(ctx);
    // row "more" buttons get a playlist-aware menu (adds move and remove)
    $$('.lib-pl-list .trow__more [data-act="track-menu"]', ctx.el).forEach(function (b) { b.setAttribute('data-act', 'lib-pl-row-menu'); });
    var list = $('.lib-pl-list', ctx.el), shown = shownRows(p), itemAt = rowToItem(p, shown);
    if (list && inOwnOrder(p)) {
      var line = document.createElement('li'); line.className = 'drop-line'; line.setAttribute('aria-hidden', 'true');
      var drag = { from: null, at: shown.length };
      var clear = function () { if (line.parentNode) line.parentNode.removeChild(line); };
      ctx.on(list, 'dragstart', function (e) {
        var row = e.target.closest('.trow'); if (!row) return;
        var rows = $$('.trow', list), sel = rows.filter(function (r) { return r.classList.contains('is-selected'); });
        drag.from = (row.classList.contains('is-selected') && sel.length > 1 ? sel : [row]).map(function (r) { return rows.indexOf(r); });
      });
      ctx.on(list, 'dragover', function (e) {
        var rows = $$('.trow', list), at = rows.length;
        for (var i = 0; i < rows.length; i++) { var b = rows[i].getBoundingClientRect(); if (e.clientY < b.top + b.height / 2) { at = i; break; } }
        drag.at = at;
        if (at < rows.length) list.insertBefore(line, rows[at]); else list.appendChild(line);
      });
      ctx.on(list, 'dragleave', function (e) { if (!list.contains(e.relatedTarget)) clear(); });
      var onEnd = function () { clear(); drag.from = null; };
      document.addEventListener('dragend', onEnd);
      ctx.onCleanup(function () { document.removeEventListener('dragend', onEnd); });
      ctx.onCleanup(App.on('drop', function (d) {
        if (d.dest !== 'lib-pl:' + p.id) return;
        clear();
        if (drag.from) moveItems(p, drag.from.map(itemAt), itemAt(drag.at)); else insertItems(p, d.ids, itemAt(drag.at));
        drag.from = null;
      }));
    }
    // keyboard reorder and remove (the shell emits these from a focused row)
    ctx.onCleanup(App.on('row:move', function (d) {
      if (d.list !== list) return;
      if (!inOwnOrder(p)) { sortNeeded(p); return; }
      var to = d.index + d.dir; if (to < 0 || to >= shown.length) return;
      moveItems(p, [itemAt(d.index)], itemAt(d.dir > 0 ? d.index + 2 : d.index - 1));
    }));
    ctx.onCleanup(App.on('row:delete', function (d) {
      if (d.list !== list) return;
      if (!inOwnOrder(p)) { sortNeeded(p); return; }
      removeItem(p, itemAt(d.index), d.index);
    }));
  }
  function sortNeeded(p) {
    App.toast('Show the custom order to move or remove songs', { icon: 'sort', action: 'Show it', onAction: function () { setSort(p, DEFAULT_SORT); } });
  }

  /* the playlist's own cover: PUT the picture (the hub checks it, cuts it square and keeps it) or DELETE it; the
     playlist then shows its songs' covers. A picture is checked here first only to say what's wrong sooner. */
  var COVER_TYPES = ['image/jpeg', 'image/png', 'image/webp'], COVER_MAX = 10 * 1024 * 1024;
  function coverProblem(file) {
    if (COVER_TYPES.indexOf(file.type) < 0) return file.name + ' isn\'t a JPEG, PNG or WebP picture.';
    if (file.size > COVER_MAX) return file.name + ' is larger than 10 MB.';
    return '';
  }
  function saveCover(plId, cover) {
    var path = playlistPath(plId) + '/cover';
    var sent = cover.file ? M.api(path, { method: 'PUT', body: cover.file }) : cover.remove ? M.api(path, { method: 'DELETE' }) : null;
    if (!sent) return Promise.resolve();
    return sent.then(function () { M.invalidate(playlistPath(plId)); return App.loadPlaylists(); },
      function (er) { App.toast('Couldn\'t ' + (cover.file ? 'set' : 'remove') + ' the cover: ' + er.message, { kind: 'error', icon: 'warning' }); });
  }

  /* create / rename / edit description and cover: the dialog stays open until the server has answered */
  function openEdit(p) {
    var formId = App.uid('lib-edit'), cover = { file: null, remove: false, preview: null };
    function coverArt() {
      var shown = cover.preview || (cover.remove ? null : p && p.cover);
      return ui.mosaic(p ? plIds(p) : [], 96, 'Cover', shown);
    }
    return App.openDialog({
      title: p ? 'edit playlist' : 'new playlist',
      body: html`<form class="lib-form" id="${formId}" novalidate>
        <div class="field"><label class="field__label" for="${formId}-name">Name</label>
          <input class="input" id="${formId}-name" name="name" maxlength="60" autocomplete="off" value="${p ? p.name : ''}" placeholder="late train home" aria-describedby="${formId}-err">
          <p class="field__error" id="${formId}-err" hidden>${icon('warning', null, 'ic-16')}<span></span></p></div>
        <div class="field"><label class="field__label" for="${formId}-desc">Description <span class="fg-2">(optional)</span></label>
          <textarea class="input lib-textarea" id="${formId}-desc" name="desc" rows="3" maxlength="300" placeholder="What it's for, when to play it">${p ? p.desc : ''}</textarea></div>
        <div class="field"><span class="field__label">Cover</span>
          <div class="lib-cover">
            <span data-cover-art>${coverArt()}</span>
            <div class="lib-cover__acts">
              <label class="btn btn--sm">${icon('import')}Choose a picture<input class="vh" type="file" accept="image/jpeg,image/png,image/webp" data-cover-file></label>
              <button class="btn btn--sm btn--quiet" type="button" data-cover-remove ${p && p.cover ? '' : raw('hidden')}>${icon('close')}Remove cover</button>
            </div>
          </div>
          <span class="field__help">A JPEG, PNG or WebP up to 10 MB, cut to a square. Without one, the playlist shows its songs' covers.</span></div>
      </form>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button><button class="btn btn--primary" type="submit" form="${formId}">${p ? 'Save' : 'Create playlist'}</button>`,
      onMount: function (el, close) {
        var form = $('form', el), name = form.elements.name, err = $('.field__error', el), submit = $('[type="submit"]', el);
        function showError(msg) { err.hidden = !msg; $('span', err).textContent = msg; name.setAttribute('aria-invalid', msg ? 'true' : 'false'); if (msg) name.focus(); }
        function paintCover() {
          $('[data-cover-art]', el).innerHTML = String(coverArt());
          $('[data-cover-remove]', el).hidden = !(cover.file || (p && p.cover && !cover.remove));
        }
        $('[data-cover-file]', el).addEventListener('change', function (e) {
          var file = e.target.files[0]; e.target.value = '';
          if (!file) return;
          var problem = coverProblem(file);
          if (problem) { App.toast(problem, { kind: 'error', icon: 'warning' }); return; }
          if (cover.preview) URL.revokeObjectURL(cover.preview);
          cover = { file: file, remove: false, preview: URL.createObjectURL(file) };
          paintCover();
        });
        $('[data-cover-remove]', el).addEventListener('click', function () {
          if (cover.preview) URL.revokeObjectURL(cover.preview);
          cover = { file: null, remove: !!(p && p.cover), preview: null };
          paintCover();
        });
        name.select();
        form.addEventListener('submit', function (e) {
          e.preventDefault();
          if (submit.disabled) return;
          var v = name.value.trim(), desc = form.elements.desc.value.trim();
          var clash = playlists().filter(function (x) { return (!p || x.id !== p.id) && x.name.toLowerCase() === v.toLowerCase(); })[0];
          showError(!v ? 'Give the playlist a name.' : clash ? 'You already have a playlist called ' + clash.name + '.' : '');
          if (!v || clash) return;
          submit.disabled = true;
          var old = p && { name: p.name, desc: p.desc || '' };
          (p ? saveDetails(p, v, desc).then(function () { return p; }) : App.createPlaylist({ name: v, desc: desc })).then(function (done) {
            if (!done) { submit.disabled = false; return; }  // App.createPlaylist has said why
            return saveCover(done.id, cover).then(function () { return done; });
          }).then(function (done) {
            submit.disabled = false;
            if (!done) return;
            if (cover.preview) URL.revokeObjectURL(cover.preview);
            close();
            if (!p) { App.go('/playlist/' + done.id); App.toast('Created ' + v + '. Drag songs in, or use Add to playlist.', { icon: 'playlists' }); return; }
            App.toast(old.name !== v ? 'Renamed to ' + v : 'Playlist details saved', { icon: 'edit', action: 'Undo', onAction: function () {
              saveDetails(p, old.name, old.desc).catch(function (er) { App.toast('Couldn\'t undo: ' + er.message, { kind: 'error' }); });
            } });
          }, function (er) { submit.disabled = false; showError(er.message); });
        });
      }
    });
  }

  /* export: the server writes the .m3u8 (GET /api/playlists/{id}.m3u8); the dialog previews it and downloads it */
  function openExport(p) {
    var file = p.name.replace(/[\\/:*?"<>|]+/g, '') + '.m3u8', path = playlistPath(p.id) + '.m3u8';
    var d = App.openDialog({
      title: 'export playlist', wide: true,
      body: html`<div class="lib-form"><div class="field"><span class="field__label">Preview of ${file}</span>
        <div data-lib-m3u aria-live="polite">${ui.skeleton('text', 4)}</div></div></div>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button><a class="btn btn--primary" href="${M.url(path)}" download="${file}" data-close data-lib-download>${icon('export')}Download ${file}</a>`
    });
    M.api(path).then(function (text) {
      var lines = String(text).split('\n');
      $('[data-lib-m3u]', d.el).innerHTML = String(html`<pre class="lib-m3u">${lines.slice(0, 7).join('\n') + (lines.length > 8 ? '\n...' : '')}</pre>`);
    }, function (err) {
      $('[data-lib-m3u]', d.el).innerHTML = String(html`<p class="field__error">${icon('warning', null, 'ic-16')}${err.message}</p>`);
      var a = $('[data-lib-download]', d.el); if (a) a.remove();
    });
  }

  /* =====================================================================
     9. Add-to-playlist picker (App.hooks.addToPlaylist): search, new playlist, recent playlists
     ===================================================================== */
  function openPicker(ids) {
    ids = [].concat(ids).filter(Boolean); if (!ids.length) return;
    var first = M.track(ids[0]), what = ids.length === 1 && first ? first.title : plural(ids.length, 'song');
    var recent = playlists().slice().sort(newestFirst);
    function rowMeta(p) {
      var have = ids.filter(function (id) { return plIds(p).indexOf(id) >= 0; }).length;
      if (have === ids.length) return html`<span class="lib-pick__have">${icon('check', null, 'ic-16')}already added</span>`;
      if (have) return html`<span class="lib-pick__have">${have} of ${ids.length} already in</span>`;
      return plural(plIds(p).length, 'song');
    }
    var inputId = App.uid('lib-pick');
    App.openSheet({
      title: 'add to playlist', className: 'lib-picker',
      body: html`<p class="lib-picker__what">Adding <b>${what}</b></p>
        <div class="search-field lib-picker__search">${icon('search')}<label class="vh" for="${inputId}">Find a playlist</label>
          <input class="input" id="${inputId}" type="search" placeholder="Find a playlist" autocomplete="off" autofocus></div>
        <button class="lib-pick lib-pick--new" type="button" data-pick-new><span class="lib-pick__plus">${icon('plus')}</span><span class="lib-pick__text"><b>New playlist</b><span>with ${what}</span></span></button>
        <form class="lib-pick-new" hidden><label class="vh" for="${inputId}-new">New playlist name</label>
          <input class="input" id="${inputId}-new" maxlength="60" placeholder="Name it" autocomplete="off"><button class="btn btn--primary" type="submit">Create</button></form>
        ${recent.length ? ui.labelRule('recent', { level: 'h3', sm: true }) : ''}
        <ul class="lib-pick-list">${recent.map(function (p) {
          return html`<li data-name="${p.name.toLowerCase()}"><button class="lib-pick" type="button" data-pick="${p.id}">${ui.playlistArt(p, 44)}<span class="lib-pick__text"><b>${p.name}</b><span>${rowMeta(p)}</span></span></button></li>`;
        })}</ul>
        <p class="lib-pick-none" hidden>No playlist matches that. Press Enter to make a new one with that name.</p>`,
      onMount: function (el, close) {
        var input = $('#' + inputId, el), form = $('.lib-pick-new', el), none = $('.lib-pick-none', el);
        function pick(id) {
          var p = playlist(id); if (!p) return;
          var missing = ids.filter(function (x) { return plIds(p).indexOf(x) < 0; });
          close();
          if (!missing.length) { App.toast('Already in ' + p.name, { icon: 'check' }); return; }
          App.addToPlaylist(p.id, missing);       // shell toast with Undo
        }
        function create(name) {
          name = name.trim() || 'new playlist';
          close();
          makePlaylist(name, '', ids).then(function (p) {
            if (p) App.toast('Created ' + name + ' with ' + what, { icon: 'playlists', action: 'Open', onAction: function () { App.go('/playlist/' + p.id); } });
          });
        }
        function showForm() { form.hidden = false; $('input', form).value = input.value; $('input', form).focus(); }
        input.addEventListener('input', function () {
          var q = input.value.trim().toLowerCase(), shown = 0;
          $$('.lib-pick-list li', el).forEach(function (li) { li.hidden = q && li.getAttribute('data-name').indexOf(q) < 0; if (!li.hidden) shown++; });
          none.hidden = shown > 0 || !q;
        });
        input.addEventListener('keydown', function (e) {
          if (e.key !== 'Enter') return; e.preventDefault();
          var b = $$('.lib-pick-list li:not([hidden]) [data-pick]', el)[0];
          if (b && input.value.trim()) pick(b.getAttribute('data-pick')); else if (input.value.trim()) create(input.value);
        });
        el.addEventListener('click', function (e) {
          var b = e.target.closest('[data-pick]'); if (b) { pick(b.getAttribute('data-pick')); return; }
          if (e.target.closest('[data-pick-new]')) showForm();
        });
        form.addEventListener('submit', function (e) { e.preventDefault(); create($('input', form).value); });
      }
    });
  }
  App.hooks.addToPlaylist = function (ids) { openPicker(ids); };

  /* =====================================================================
     10. Imports (App.hooks.importFile + the Import buttons).
         Playlists: an M3U (POST /api/playlists/import) or a Spotify playlist exported by Exportify (POST
         /api/import/exportify) gives the same preview (library | will-search | not-found; it creates nothing);
         importing POSTs a playlist of every line with a song id (will-search songs are found on Soulseek when played).
         Listening history: Last.fm by user name, or a ListenBrainz export file (POST /api/import/history); the plays
         arrive as a `job` row in Activity.
     ===================================================================== */
  var PLAYLIST_FILE = /\.(m3u8?|csv)$/i;
  function openImportStart() {
    var lf = M.peek('/api/lastfm'), userId = App.uid('lib-lfu');
    App.openDialog({
      title: 'import', wide: true, className: 'lib-import',
      body: html`<div class="lib-form">
        ${ui.labelRule('a playlist link', { level: 'h3', sm: true })}
        ${linkForm()}
        ${ui.labelRule('a playlist file', { level: 'h3', sm: true })}
        ${dropZone()}
        <p class="lib-help">Each song is matched to your library first, then searched on Soulseek. Nothing downloads until you play or save it. For a Spotify playlist of more than 100 songs, export it with Exportify and drop the .csv here.</p>
        ${ui.labelRule('listening history', { level: 'h3', sm: true })}
        <p class="lib-help">Past plays join History, Stats and your taste profile. They never scrobble.</p>
        <form class="lib-hist" data-form="lib-lastfm-import">
          <div class="field"><label class="field__label" for="${userId}">Last.fm user</label>
            <input class="input" id="${userId}" value="${lf && lf.user || ''}" autocomplete="off" spellcheck="false" placeholder="Your Last.fm user name"><span class="field__help">The profile has to be public.</span></div>
          <button class="btn" type="submit">${icon('radio')}Import from Last.fm</button>
        </form>
        <div class="lib-hist"><div class="field"><span class="field__label">ListenBrainz</span><span class="field__help">An export from your ListenBrainz settings (listens as JSON).</span></div>
          <button class="btn" type="button" data-act="lib-lb-pick">${icon('import')}Choose export file</button></div>
      </div>`,
      onMount: function (el, close) {
        $('[data-form="lib-lastfm-import"]', el).addEventListener('submit', function (e) {
          e.preventDefault();
          var user = $('#' + userId, el).value.trim();
          if (!user) { $('#' + userId, el).focus(); return; }
          importHistory({ source: 'lastfm', user: user }, 'Last.fm', close);
        });
      }
    });
  }
  /* POST /api/import/history: 202 { jobId }; Activity follows the job */
  function importHistory(body, from, close) {
    M.api('/api/import/history', { method: 'POST', body: body }).then(function () {
      if (close) close();
      App.toast('Importing your ' + from + ' history. Activity shows how it goes.', { icon: 'import', action: 'Activity', onAction: function () { App.openPanel('activity'); } });
      App.loadJobs();
    }, function (err) { App.toast('Couldn\'t import from ' + from + ': ' + err.message, { kind: 'error' }); });
  }
  function pickFile(accept, then) {
    var input = document.createElement('input');
    input.type = 'file'; input.accept = accept || '.m3u,.m3u8,.csv,audio/x-mpegurl,text/csv'; input.hidden = true;
    input.addEventListener('change', function () { if (input.files[0]) (then || App.hooks.importFile)(input.files[0]); input.remove(); });
    document.body.appendChild(input); input.click();
  }
  var IMPORT_LABEL = { library: 'In your library', 'will-search': 'Searched on Soulseek when played', 'not-found': 'Not found' };
  /* the preview { name, lines: [{ n, location, artist, title, status, trackId }], counts, libraryChecked }, checked:
     a status this version doesn't know fails the import rather than guess */
  function checkPreview(res) {
    res.lines.forEach(function (l) {
      if (!IMPORT_LABEL[l.status]) throw new M.ApiError(0, 'bad_preview', 'The server sent an import line lull doesn\'t understand (' + l.status + ').');
    });
    return res;
  }
  function importRow(l) {
    var t = l.trackId && M.track(l.trackId), status;
    if (l.status === 'library') status = ui.savedBadge();
    else if (l.status === 'will-search') status = html`${ui.soulseekBadge()}<span class="lib-imp__src">when played</span>`;
    else status = html`<span class="lib-imp__none">${icon('no-source', null, 'ic-16')}not found</span><a class="btn btn--quiet btn--sm" href="#/search?q=${encodeURIComponent(((l.artist || '') + ' ' + (l.title || '')).trim() || l.location)}" data-close>Search</a>`;
    return html`<li class="lib-imp__row" data-status="${l.status}">
      <span class="lib-imp__n">${l.n}</span>
      ${t ? ui.cover(t, 40) : html`<span class="lib-imp__blank" aria-hidden="true">${icon('no-source')}</span>`}
      <span class="lib-imp__main"><b>${t ? t.title : l.title || l.location}</b><span>${t ? t.artist : l.artist || 'unknown artist'}</span><small>${l.location}</small></span>
      <span class="lib-imp__status"><span class="vh">${IMPORT_LABEL[l.status]}: </span>${status}</span>
    </li>`;
  }
  function openImportPreview(fileName, text) {
    if (!/^[ \t]*[^#\s]/m.test(text)) { App.toast(fileName + ' has no songs in it', { kind: 'error', icon: 'warning' }); return; }
    var endpoint = /\.csv$/i.test(fileName) ? '/api/import/exportify' : '/api/playlists/import';
    var cancelled = false;
    var wait = App.openDialog({
      title: 'import ' + fileName, wide: true, className: 'lib-import',
      body: html`<div class="lib-form"><p class="lib-help">Matching each line to your library and Soulseek.</p>${ui.skeleton('row', 4)}</div>`,
      onClose: function () { cancelled = true; }
    });
    M.api(endpoint, { method: 'POST', body: String(text) }).then(checkPreview).then(function (preview) {
      if (!cancelled) showImportPreview(fileName, preview);
    }, function (err) {
      if (cancelled) return;
      wait.close();
      App.toast('Couldn\'t import ' + fileName + ': ' + err.message, { kind: 'error', icon: 'warning' });
    });
  }
  function showImportPreview(fileName, preview) {
    var lines = preview.lines, counts = preview.counts;
    var tally = { all: lines.length, library: counts.library, 'will-search': counts.willSearch, 'not-found': counts.notFound };
    var usable = lines.filter(function (l) { return l.trackId; });
    var searchIds = usable.filter(function (l) { return l.status === 'will-search'; }).map(function (l) { return l.trackId; });
    var nameId = App.uid('lib-imp'), base = preview.name || fileName.replace(PLAYLIST_FILE, '');
    function filterPill(key, label, ic) {
      return html`<button class="pill-filter" data-imp-filter="${key}" aria-pressed="${key === 'all' ? 'true' : 'false'}">${ic ? icon(ic) : ''}${label}<span class="lib-count">${tally[key]}</span></button>`;
    }
    App.openDialog({
      title: 'import ' + fileName, wide: true, className: 'lib-import',
      body: html`<div class="lib-form">
        <div class="field"><label class="field__label" for="${nameId}">Playlist name</label>
          <input class="input" id="${nameId}" maxlength="60" value="${base}" autocomplete="off">
          <span class="field__help" data-imp-same aria-live="polite">${sameNameHelp(base)}</span></div>
        <div class="lib-imp__sum" role="group" aria-label="Show lines">
          ${filterPill('all', 'All')}${filterPill('library', 'In library', 'saved')}${filterPill('will-search', 'On Soulseek', 'soulseek')}${filterPill('not-found', 'Not found', 'no-source')}
        </div>
        ${preview.libraryChecked ? '' : html`<p class="lib-help">${icon('warning', null, 'ic-16')}Your library couldn't be checked just now, so songs you've saved may show as found on Soulseek.</p>`}
        <ol class="lib-imp__list" aria-label="Lines in ${fileName}">${lines.map(importRow)}</ol>
        ${searchIds.length ? saveSongsSwitch(plural(searchIds.length, 'song') + ' not in your library yet. Otherwise they play from Soulseek when you play them.') : ''}
        ${!usable.length ? html`<p class="field__error">${icon('warning', null, 'ic-16')}Nothing in this file matched. Try searching for the songs instead.</p>` : ''}
      </div>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button>
        <button class="btn btn--primary" type="button" data-imp-go ${usable.length ? '' : raw('disabled')}>${icon('import')}Import ${plural(usable.length, 'song')}</button>`,
      onMount: function (el, close) {
        $('#' + nameId, el).addEventListener('input', function (e) {
          $('[data-imp-same]', el).innerHTML = String(sameNameHelp(e.target.value.trim() || base));
        });
        el.addEventListener('click', function (e) {
          var f = e.target.closest('[data-imp-filter]');
          if (f) {
            var v = f.getAttribute('data-imp-filter');
            $$('[data-imp-filter]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(b === f)); });
            $$('.lib-imp__row', el).forEach(function (r) { r.hidden = v !== 'all' && r.getAttribute('data-status') !== v; });
            return;
          }
          if (!e.target.closest('[data-imp-go]')) return;
          var name = $('#' + nameId, el).value.trim() || base;
          var saveToo = !!searchIds.length && saveSongsPref();
          var same = sameName(name), ids = usable.map(function (l) { return l.trackId; });
          close();
          var imported = { file: fileName, library: counts.library, willSearch: counts.willSearch, notFound: counts.notFound };
          /* the same file again updates the playlist it made, instead of a second one */
          var done = same ? App.savePlaylistOrder(same.id, ids).then(function () { return same; })
            : makePlaylist(name, 'Imported from ' + fileName + '.', ids, imported, usable.map(function (l) { return l.addedAt == null ? null : l.addedAt; }));
          done.then(function (p) {
            if (!p) return;
            if (saveToo) App.saves.start(searchIds);
            App.go('/playlist/' + p.id);
            App.toast((same ? 'Updated ' + p.name + ' in your playlists: ' : 'Imported ' + p.name + ' into your playlists: ') + plural(usable.length, 'song')
              + (counts.notFound ? ', ' + counts.notFound + ' not found' : '') + (saveToo ? ', saving ' + plural(searchIds.length, 'song') : ''), { kind: 'ok', icon: 'import' });
          }, function (err) { App.toast('Couldn\'t import ' + fileName + ': ' + err.message, { kind: 'error', icon: 'warning' }); });
        });
      }
    });
  }
  /* a playlist already named so (case aside): importing under its name updates it */
  function sameName(name) {
    var n = String(name || '').trim().toLowerCase();
    return playlists().filter(function (p) { return p.name.trim().toLowerCase() === n; })[0] || null;
  }
  function sameNameHelp(name) {
    var p = sameName(name);
    return p ? 'You have a playlist called ' + p.name + ': importing updates its songs. Rename to make a new one.' : 'Imported as a new playlist in Library, Playlists.';
  }
  App.hooks.importFile = function (file) {
    if (!PLAYLIST_FILE.test(file.name)) { App.toast('Can\'t import ' + file.name + ': only .m3u, .m3u8 and Exportify .csv playlists', { kind: 'error', icon: 'warning' }); return; }
    var reader = new FileReader();
    reader.onload = function () { openImportPreview(file.name, String(reader.result)); };
    reader.onerror = function () { App.toast('Couldn\'t read ' + file.name, { kind: 'error', icon: 'warning' }); };
    reader.readAsText(file);
  };

  /* ---------- a playlist link (POST /api/import/link, docs/context/collection.md#playlist-links) ----------
     A Spotify, Deezer, YouTube, YouTube Music or SoundCloud link becomes a playlist that plays on demand. The hub reads
     and matches it as a `job` (Activity); when this tab's import ends, a toast offers the review, where songs the catalog
     didn't match are found in search, kept as the link names them, or skipped (PUT /api/import/links/{id}/songs/{pos}).
     Importing the same link again updates its playlist and keeps those choices. */
  var linkJobs = {};                  // jobId -> true for the imports this tab started
  var SITE_NAME = { spotify: 'Spotify', deezer: 'Deezer', youtube: 'YouTube', soundcloud: 'SoundCloud' };
  var LINK_STATUS = { matched: 'Matched', fixed: 'Your pick', skipped: 'Skipped', unmatched: 'Not found' };
  function linkForm() {
    var id = App.uid('lib-link');
    return html`<form class="lib-link" data-form="lib-link-import">
      <label class="field__label" for="${id}">Import from a link</label>
      <div class="lib-link__row">
        <input class="input" id="${id}" type="url" inputmode="url" autocomplete="off" spellcheck="false" placeholder="A Spotify, YouTube, Deezer or SoundCloud playlist link">
        <button class="btn btn--primary" type="submit">${icon('import')}Import</button>
      </div>
      <span class="field__help">It becomes a playlist in Library, Playlists. Spotify lets lull read the first 100 songs of a playlist. Importing a link again updates its playlist.</span>
      ${saveSongsSwitch('Otherwise they play from Soulseek when you play them.')}
    </form>`;
  }
  /* every link import, from the box or a preview's "Import as playlist", carries the device's "Save the songs too" */
  function importLink(url, onStarted) {
    var save = saveSongsPref();
    return M.api('/api/import/link', { method: 'POST', body: { url: url, saveSongs: save } }).then(function (res) {
      linkJobs[res.jobId] = true;
      if (onStarted) onStarted();
      App.toast('Importing it as a playlist' + (save ? ' and saving its songs' : '') + '. Activity shows how it goes.', { icon: 'import', action: 'Activity', onAction: function () { App.openPanel('activity'); } });
      App.loadJobs();
    }, function (err) { App.toast('Couldn\'t import that link: ' + err.message, { kind: 'error', icon: 'warning' }); });
  }
  document.addEventListener('submit', function (e) {
    var form = e.target.closest && e.target.closest('[data-form="lib-link-import"]');
    if (!form) return;
    e.preventDefault();
    var input = $('input', form), btn = $('button[type="submit"]', form), url = input.value.trim();
    if (!url) { input.focus(); return; }
    btn.disabled = true;
    importLink(url, function () { input.value = ''; if (form.closest('.lib-import')) App.closeTop(); })
      .then(function () { btn.disabled = false; });
  });
  /* the hub made or changed a playlist: the Playlists page and the playlist's own page read it again */
  function linkPlaylistChanged(playlistId) {
    if (playlistId) M.invalidate(playlistPath(playlistId));
    App.loadPlaylists();
  }
  App.on('server:job', function (j) {
    if (!j || !j.link || j.state === 'running') return;
    if (j.state === 'done') linkPlaylistChanged(j.playlistId);
    if (!linkJobs[j.jobId]) return;
    delete linkJobs[j.jobId];
    if (j.state === 'failed') { App.toast('Couldn\'t import ' + j.name + ': ' + (j.error || 'the import failed'), { kind: 'error', icon: 'warning' }); return; }
    App.toast('Imported ' + j.name + ' into your playlists: ' + plural(j.matched || 0, 'song') + (j.unmatched ? ', ' + j.unmatched + ' not found' : '') + (j.saving ? ', saving ' + j.saving : ''),
      { kind: 'ok', icon: 'import', action: j.unmatched ? 'Review' : 'Open', onAction: function () {
        if (j.unmatched) openLinkReview(j.link); else App.go('/playlist/' + j.playlistId);
      } });
  });

  /* the review: { id, url, site, name, playlistId, note, importedAt, running, counts, songs: [{ pos, artist, title, album,
     duration, status: matched|fixed|skipped|unmatched, trackId }] }; a status this version doesn't know fails loudly */
  function openLinkReview(id) {
    var st = { data: null, filter: 'unmatched', finding: null, results: null, searching: false };
    var d = App.openDialog({
      title: 'review import', wide: true, className: 'lib-import lib-linkrev',
      body: html`<div class="lib-form" data-linkrev>${ui.skeleton('row', 4)}</div>`,
      foot: html`<button class="btn" type="button" data-close>Close</button><span class="lib-linkrev__foot" data-linkrev-foot></span>`
    });
    function paint() {
      $('[data-linkrev]', d.el).innerHTML = String(reviewBody(st));
      $('[data-linkrev-foot]', d.el).innerHTML = String(reviewFoot(st.data));
      var q = $('[data-rev-q]', d.el); if (q && !st.results && !st.searching) q.focus();
    }
    function showError(message) {
      $('[data-linkrev]', d.el).innerHTML = String(html`<p class="field__error">${icon('warning', null, 'ic-16')}${message}</p>`);
    }
    function took(res) {
      var odd = res.songs.filter(function (s) { return !LINK_STATUS[s.status]; })[0];
      if (odd) { showError('The server sent a song status lull doesn\'t understand (' + odd.status + ').'); return; }
      st.data = res;
      if (st.filter === 'unmatched' && !res.counts.unmatched) st.filter = 'all';
      paint();
    }
    function failed(err) { App.toast('Couldn\'t change that song: ' + err.message, { kind: 'error', icon: 'warning' }); }
    function choose(pos, body) {
      st.finding = null; st.results = null;
      M.api('/api/import/links/' + encodeURIComponent(id) + '/songs/' + pos, { method: 'PUT', body: body }).then(function (res) {
        took(res); linkPlaylistChanged(res.playlistId);
      }, failed);
    }
    M.api('/api/import/links/' + encodeURIComponent(id)).then(took, function (err) { showError(err.message); });
    d.el.addEventListener('click', function (e) {
      var b = e.target.closest('button'); if (!b || !st.data) return;
      var pos = b.getAttribute('data-pos');
      if (b.hasAttribute('data-rev-filter')) { st.filter = b.getAttribute('data-rev-filter'); paint(); }
      else if (b.hasAttribute('data-rev-find')) { st.finding = +pos; st.results = null; paint(); }
      else if (b.hasAttribute('data-rev-cancel')) { st.finding = null; st.results = null; paint(); }
      else if (b.hasAttribute('data-rev-choose')) choose(pos, { choice: b.getAttribute('data-rev-choose') });
      else if (b.hasAttribute('data-rev-pick')) choose(pos, { choice: 'track', trackId: b.getAttribute('data-id') });
      else if (b.hasAttribute('data-rev-update')) importLink(st.data.url, d.close);
      else if (b.hasAttribute('data-rev-open')) { d.close(); App.go('/playlist/' + st.data.playlistId); }
    });
    d.el.addEventListener('submit', function (e) {
      if (!e.target.matches('[data-rev-search]')) return;
      e.preventDefault();
      var q = $('[data-rev-q]', e.target).value.trim(); if (!q) return;
      st.searching = true; st.query = q; paint();
      M.api('/api/search?q=' + encodeURIComponent(q)).then(function (res) {
        st.searching = false; st.results = (res.tracks || []).slice(0, 6); paint();
      }, function (err) { st.searching = false; st.results = []; failed(err); paint(); });
    });
  }
  function reviewBody(st) {
    var r = st.data; if (!r) return ui.skeleton('row', 4);
    var c = r.counts, inList = c.matched + c.fixed;
    var tally = { all: r.songs.length, unmatched: c.unmatched, matched: inList, skipped: c.skipped };
    var shown = r.songs.filter(function (s) {
      return st.filter === 'all' || (st.filter === 'matched' ? s.status === 'matched' || s.status === 'fixed' : s.status === st.filter);
    });
    function pill(key, label, ic) {
      return html`<button class="pill-filter" type="button" data-rev-filter="${key}" aria-pressed="${String(st.filter === key)}">${ic ? icon(ic) : ''}${label}<span class="lib-count">${tally[key]}</span></button>`;
    }
    return html`<p class="lib-help">${SITE_NAME[r.site] || r.site}: <a href="${r.url}" target="_blank" rel="noopener noreferrer">${r.name}</a>.
        ${plural(inList, 'song')} in the playlist${r.running ? '. Importing now.' : r.importedAt ? ', imported ' + fmt.date(r.importedAt) + '.' : '.'}</p>
      ${r.note ? html`<p class="lib-help">${icon('info', null, 'ic-16')}${r.note}</p>` : ''}
      <div class="lib-imp__sum" role="group" aria-label="Show songs">
        ${pill('unmatched', 'Not found', 'no-source')}${pill('matched', 'In the playlist', 'check')}${pill('skipped', 'Skipped', 'close')}${pill('all', 'All')}
      </div>
      ${shown.length ? html`<ol class="lib-imp__list" aria-label="Songs in ${r.name}">${shown.map(function (s) { return reviewRow(s, st); })}</ol>`
        : html`<p class="now-next__none">${icon('check', null, 'ic-16')}${st.filter === 'unmatched' ? 'Every song was found.' : 'Nothing here.'}</p>`}
      <p class="lib-help">Choices stay when you import the link again. Songs you add or remove in the playlist itself are replaced by the link's songs then: skip a song here to keep it out.</p>`;
  }
  function reviewRow(s, st) {
    var t = s.trackId && M.track(s.trackId), acts;
    if (s.status === 'unmatched') {
      acts = html`<button class="btn btn--sm" type="button" data-rev-find data-pos="${s.pos}">${icon('search')}Find</button>
        <button class="btn btn--sm btn--quiet" type="button" data-rev-choose="as-is" data-pos="${s.pos}" data-tip="Searched on Soulseek by this artist and title when played">Keep as named</button>
        <button class="btn btn--sm btn--quiet" type="button" data-rev-choose="skip" data-pos="${s.pos}">Skip</button>`;
    } else if (s.status === 'matched') {
      acts = html`<button class="btn btn--sm btn--quiet" type="button" data-rev-find data-pos="${s.pos}">Change</button>
        <button class="btn btn--sm btn--quiet" type="button" data-rev-choose="skip" data-pos="${s.pos}">Skip</button>`;
    } else {
      acts = html`<button class="btn btn--sm btn--quiet" type="button" data-rev-choose="none" data-pos="${s.pos}">Undo</button>`;
    }
    var found = t ? html`<small>${LINK_STATUS[s.status]}: ${t.artist}, ${t.title}${t.album ? ' (' + t.album + ')' : ''}</small>` : html`<small>${LINK_STATUS[s.status]}</small>`;
    return html`<li class="lib-imp__row lib-linkrev__row" data-status="${s.status}">
      <span class="lib-imp__n">${s.pos + 1}</span>
      ${t ? ui.cover(t, 40) : html`<span class="lib-imp__blank" aria-hidden="true">${icon('no-source')}</span>`}
      <span class="lib-imp__main"><b>${s.title}</b><span>${s.artist}${s.duration ? ', ' + fmt.time(s.duration) : ''}</span>${found}</span>
      <span class="lib-imp__status">${acts}</span>
      ${st.finding === s.pos ? findPanel(s, st) : ''}
    </li>`;
  }
  function findPanel(s, st) {
    var results;
    if (st.searching) results = ui.skeleton('row', 2);
    else if (st.results && !st.results.length) results = html`<p class="lib-help">Nothing found. Try other words.</p>`;
    else if (st.results) {
      results = html`<ul class="lib-linkrev__hits">${st.results.map(function (id) {
        var t = M.track(id); if (!t) return '';
        return html`<li><button class="lib-linkrev__hit" type="button" data-rev-pick data-pos="${s.pos}" data-id="${id}">
          ${ui.cover(t, 40)}<span class="lib-imp__main"><b>${t.title}</b><span>${t.artist}${t.album ? ', ' + t.album : ''}${t.duration ? ', ' + fmt.time(t.duration) : ''}</span></span></button></li>`;
      })}</ul>`;
    } else results = '';
    return html`<div class="lib-linkrev__find">
      <form class="lib-link__row" data-rev-search role="search">
        <input class="input" data-rev-q aria-label="Search for this song" value="${st.query && st.results ? st.query : (s.artist + ' ' + s.title)}" autocomplete="off" spellcheck="false">
        <button class="btn btn--sm" type="submit">${icon('search')}Search</button>
        <button class="btn btn--sm btn--quiet" type="button" data-rev-cancel>Cancel</button>
      </form>
      ${results}
    </div>`;
  }
  function reviewFoot(r) {
    if (!r) return '';
    return html`<button class="btn" type="button" data-rev-update ${r.running ? raw('disabled') : ''}>${icon('refresh')}Update from the link</button>
      ${r.playlistId ? html`<button class="btn btn--primary" type="button" data-rev-open>${icon('play')}Open playlist</button>` : ''}`;
  }
  App.action('lib-link-review', function (el) { openLinkReview(el.getAttribute('data-id')); });

  /* =====================================================================
     11. Actions
     ===================================================================== */
  function idsOf(el) { return (el.getAttribute('data-ids') || el.getAttribute('data-id') || '').split(',').filter(Boolean); }
  function shuffled(ids) {
    var a = ids.slice();
    for (var i = a.length - 1; i > 0; i--) { var j = Math.floor(Math.random() * (i + 1)), t = a[i]; a[i] = a[j]; a[j] = t; }
    return a;
  }
  App.action('lib-shuffle-ids', function (el) {
    var ids = idsOf(el); if (!ids.length) { App.toast('Nothing to shuffle yet', { icon: 'info' }); return; }
    App.player.playList(shuffled(ids), 0); App.player.setShuffle(true);
  });
  App.action('lib-save-group', function (el) { App.saves.start(idsOf(el)); });
  App.action('lib-retry-group', function (el) {
    var saves = App.store.get('saves');
    idsOf(el).forEach(function (id) { if (saves[id] && saves[id].status === 'failed') App.saves.retry(id); });
  });
  /* like (or unlike) every song; undo puts back only the ones this changed */
  App.action('lib-like-all', function (el) {
    var ids = idsOf(el); if (!ids.length) return;
    var on = !ids.every(isLiked), changed = ids.filter(function (id) { return isLiked(id) !== on; });
    App.setLiked(changed, on);
    App.toast(on ? 'Liked ' + plural(changed.length, 'song') : 'Removed ' + plural(changed.length, 'like'), { icon: on ? 'heart-fill' : 'heart', action: 'Undo',
      onAction: function () { App.setLiked(changed, !on); } });
  });
  App.action('lib-history-send', function (el) { sendToLastfm([el.getAttribute('data-id')], el); });
  App.action('lib-history-send-all', function (el) { sendToLastfm(el.getAttribute('data-ids').split(','), el); });
  App.action('lib-history-older', function (el) { historyPages.push(el.getAttribute('data-id')); refreshKeepFocus(); });
  App.action('lib-show-all', function (el) { limits[el.getAttribute('data-id')] = +el.getAttribute('data-value'); App.refresh(); });
  function sortMenu(el, table, key, store) {
    App.menu(el, Object.keys(table).map(function (k) {
      return { label: table[k], icon: prefs[key] === k ? 'check' : 'dash', run: function () { prefs[key] = k; App.storage(store, k); App.refresh(); } };
    }), { label: 'Sort by' });
  }
  App.action('lib-album-sort', function (el) { sortMenu(el, ALBUM_SORTS, 'albumSort', 'libAlbumSort'); });
  App.action('lib-song-sort', function (el) { sortMenu(el, SONG_SORTS, 'songSort', 'libSongSort'); });
  /* the album, artist and playlist menus, by id: their "…" buttons, and a right-click on their tiles (App.itemMenus) */
  function albumMenu(id, anchor) {
    var a = M.album(id); if (!a) return;
    var items = [
      { label: 'Play next', icon: 'queue', run: function () { App.player.playNext(a.trackIds); App.toast(a.title + ' plays next', { icon: 'queue' }); } },
      { label: 'Add to queue', icon: 'plus', run: function () { App.player.addToQueue(a.trackIds); App.toast('Added ' + a.title + ' to the queue', { icon: 'queue' }); } },
      { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + a.artistId); } },
      App.coverItem('album', a.id)
    ];
    if (a.trackIds[0]) items.push({ label: 'Show sources for track 1', icon: 'soulseek', run: function () { App.hooks.sourcePicker(a.trackIds[0]); } });
    items.push('sep', { label: 'Copy link', icon: 'link', run: function () { App.copyShareLink('album', a.id, '/album/' + a.id); } },
      { label: 'Copy lull link', icon: 'link', run: function () { App.copyLullLink('/album/' + a.id); } });
    App.menu(anchor, items, { label: 'Actions for ' + a.title });
  }
  /* `top`: the artist page's top songs; a tile has none, so its menu has no "Add top songs" */
  function artistMenu(id, top, anchor) {
    var ar = M.artist(id); if (!ar) return;
    var items = [];
    if (top.length) items.push({ label: 'Add top songs to queue', icon: 'queue', run: function () { App.player.addToQueue(top); App.toast('Added ' + ar.name + ' to the queue', { icon: 'queue' }); } });
    items.push({ label: 'Hide from my feed', icon: 'close', run: function () {
      M.api('/api/feedback', { method: 'POST', body: { kind: 'hide-artist', id: ar.id } }).then(function () {
        App.store.update('hiddenArtists', function (h) { h[ar.id] = true; });
        App.toast('Hidden ' + ar.name + ' from your feed', { icon: 'info' });
      }, function (err) { App.toast('Couldn\'t hide ' + ar.name + ': ' + err.message, { kind: 'error' }); });
    } }, 'sep', { label: 'Copy link', icon: 'link', run: function () { App.copyShareLink('artist', ar.id, '/artist/' + ar.id); } },
      { label: 'Copy lull link', icon: 'link', run: function () { App.copyLullLink('/artist/' + ar.id); } });
    App.menu(anchor, items, { label: 'Actions for ' + ar.name });
  }
  App.action('lib-album-menu', function (el) { albumMenu(el.getAttribute('data-id'), el); });
  App.action('lib-artist-menu', function (el) { artistMenu(el.getAttribute('data-id'), (el.getAttribute('data-ids') || '').split(',').filter(Boolean), el); });
  App.itemMenus.album = albumMenu;
  App.itemMenus.artist = function (id, anchor) { artistMenu(id, [], anchor); };
  App.itemMenus.playlist = playlistMenu;
  App.action('lib-import', function () { openImportStart(); });
  App.action('lib-import-pick', function () { pickFile(); });
  App.action('lib-lb-pick', function () {
    pickFile('.json,.jsonl,.txt,application/json', function (file) { App.closeTop(); importHistory(file, 'ListenBrainz'); });
  });
  App.action('lib-pl-reload', function () { App.loadPlaylists(); });
  App.action('lib-pl-new', function () { openEdit(null); });
  App.action('lib-pl-edit', function (el) { var p = playlistOf(el.getAttribute('data-id')); if (p) openEdit(p); });
  App.action('lib-pl-export', function (el) {
    var p = playlistOf(el.getAttribute('data-id')); if (!p) return;
    if (!plIds(p).length) { App.toast('Add a few songs before exporting', { icon: 'info' }); return; }
    openExport(p);
  });
  App.action('lib-pl-sort', function (el) {
    var p = playlistOf(el.getAttribute('data-id')), key = el.getAttribute('data-value');
    if (p && SORTS[key]) setSort(p, { key: key, dir: el.getAttribute('data-dir') === 'desc' ? 'desc' : 'asc' });
  });
  /* the menu: custom order and Reversed, then each column; the column in use offers its other direction */
  App.action('lib-pl-sortmenu', function (el) {
    var p = playlistOf(el.getAttribute('data-id')); if (!p) return;
    var s = sortOf(p), item = function (label, sort, checked) {
      return { label: label, icon: checked ? 'check' : 'dash', run: function () { setSort(p, sort); } };
    };
    var items = [item(SORTS.custom.label, DEFAULT_SORT, inOwnOrder(p)), item(SORTS.custom.reversed, { key: 'custom', dir: 'desc' }, s.key === 'custom' && s.dir === 'desc'), 'sep'];
    ['added', 'title', 'artist', 'album', 'duration'].forEach(function (k) {
      var on = s.key === k, dir = on ? s.dir : SORTS[k].first, other = dir === 'asc' ? 'desc' : 'asc';
      items.push(item(sortLabel({ key: k, dir: dir }), { key: k, dir: dir }, on));
      if (on) items.push(item(sortLabel({ key: k, dir: other }), { key: k, dir: other }, false));
    });
    App.menu(el, items, { label: 'Sort songs' });
  });
  App.action('lib-pl-menu', function (el) { playlistMenu(el.getAttribute('data-id'), el); });
  function playlistMenu(id, anchor) {
    var p = playlistOf(id); if (!p) return;
    var ids = plIds(p);
    App.menu(anchor, [
      { label: 'Rename', icon: 'edit', run: function () { openEdit(p); } },
      { label: 'Play next', icon: 'queue', run: function () { if (!ids.length) return App.toast('This playlist is empty', { icon: 'info' }); App.player.playNext(ids); App.toast(p.name + ' plays next', { icon: 'queue' }); } },
      { label: 'Add to queue', icon: 'plus', run: function () { if (!ids.length) return App.toast('This playlist is empty', { icon: 'info' }); App.player.addToQueue(ids); App.toast('Added ' + p.name + ' to the queue', { icon: 'queue' }); } },
      { label: 'Duplicate', icon: 'playlists', run: function () {
        makePlaylist(p.name + ' (copy)', p.desc, ids, undefined, addedOf(p)).then(function (c) { if (c) { App.go('/playlist/' + c.id); App.toast('Made a copy of ' + p.name, { icon: 'playlists' }); } });
      } },
      App.coverItem('playlist', p.id),
      { label: 'Export as M3U', icon: 'export', run: function () { if (ids.length) openExport(p); else App.toast('Add a few songs before exporting', { icon: 'info' }); } },
      { label: 'Copy lull link', icon: 'link', run: function () { App.copyLullLink('/playlist/' + p.id); } },   // playlists have no public page
      'sep',
      { label: 'Delete playlist', icon: 'trash', danger: true, run: function () { deletePlaylist(p); } }
    ], { label: 'Actions for ' + p.name });
  }
  App.action('lib-pl-row-menu', function (el) {
    var row = el.closest('.trow'), list = row.parentElement, p = playlistOf(list.getAttribute('data-pl')); if (!p) return;
    var shown = shownRows(p), itemAt = rowToItem(p, shown), n = $$('.trow', list).indexOf(row), id = row.getAttribute('data-track-id'), t = M.track(id), custom = inOwnOrder(p);
    var items = [
      { label: 'Play next', icon: 'queue', run: function () { App.player.playNext([id]); } },
      { label: 'Add to queue', icon: 'plus', run: function () { App.player.addToQueue([id]); } },
      { label: isSaved(id) ? 'In your library' : 'Save to library', icon: isSaved(id) ? 'saved' : 'save', run: function () { if (isSaved(id)) App.toast('Already saved to your library', { icon: 'saved' }); else App.saves.start([id]); } },
      { label: 'Add to another playlist', icon: 'playlists', run: function () { openPicker([id]); } },
      { label: 'Start radio', icon: 'radio', run: function () { App.player.startRadio({ kind: 'track', id: id }); } },
      'sep'
    ];
    /* no Move up on the first row, no Move down on the last */
    if (n > 0) items.push({ label: 'Move up', icon: 'chevron-up', run: function () { if (!custom) return sortNeeded(p); moveItems(p, [itemAt(n)], itemAt(n - 1)); } });
    if (n < shown.length - 1) items.push({ label: 'Move down', icon: 'chevron-down', run: function () { if (!custom) return sortNeeded(p); moveItems(p, [itemAt(n)], itemAt(n + 2)); } });
    if (t) {
      items.push({ label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } });
      items.push({ label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + t.artistId); } });
      items.push(App.coverItem('track', id));
      items.push({ label: 'Show sources', icon: 'soulseek', run: function () { App.hooks.sourcePicker(id); } });
    }
    items.push({ label: 'Remove from this playlist', icon: 'trash', danger: true, run: function () { if (!custom) return sortNeeded(p); removeItem(p, itemAt(n), n); } });
    App.menu(el, items, { label: 'Actions for ' + (t ? t.title : 'this song') });
  });

  /* =====================================================================
     12. Routes
     ===================================================================== */
  /* title-bar meta lines (route.meta -> .main[data-meta]; the same text as data-meta on the page card) */
  function albumMeta(a) {
    if (!a.trackIds.length) return '';
    return [plural(a.trackIds.length, 'song'), fmt.length(a.duration || 0)].concat(a.sizeMB ? [fmt.size(a.sizeMB)] : []).join(', ');
  }
  function playlistMeta(p) { var ids = plIds(p); return plural(ids.length, 'song') + ', ' + fmt.length(sumDuration(ids)); }
  function metaOf(fn, line) { return function (ctx) { var x = fn(ctx.params.id); return x ? line(x) : ''; }; }
  /* the page title once the entity is known; the kind until then */
  function titleOf(fn, kind) { return function (ctx) { var x = fn(ctx.params.id); return x ? (x.title || x.name) : kind; }; }
  function artistMetaLine(ctx) {
    var d = M.peek(artistPath(ctx.params.id));
    return d && d.albums && (d.pending || []).indexOf('albums') < 0 ? plural(ownReleases(d), 'release') : '';
  }
  /* route.place(ctx): the page as a place to play from (App.here(), "where the music plays from" in app.js) */
  function placeOf(kind, fn) { return function (ctx) { var x = fn(ctx.params.id); return x ? App.place(kind, ctx.params.id, x.title || x.name) : null; }; }
  App.route('/library/:segment?', { render: renderLibrary, mount: mountLibrary, nav: 'library',
    place: function (ctx) { return ctx.params.segment === 'songs' ? App.place('songs', '', 'Saved songs') : null; },
    title: function (ctx) { var s = SEGMENTS.filter(function (x) { return x.id === ctx.params.segment; })[0]; return 'Library' + (s ? ': ' + s.label : ''); } });
  App.route('/liked', { render: renderLiked, mount: mountLiked, nav: 'library', title: 'Liked songs', place: function () { return App.place('liked', '', 'Liked songs'); } });
  App.route('/downloads', { render: renderDownloads, mount: mountDownloads, nav: 'library', title: 'Downloads', place: function () { return App.place('downloads', '', 'Downloads'); } });
  App.route('/history', { render: renderHistory, mount: mountLibrary, nav: 'library', title: 'History', place: function () { return App.place('history', '', 'History'); } });
  App.route('/following', { render: renderFollowing, mount: mountLibrary, nav: 'library', title: 'Following' });
  App.route('/album/:id', { render: renderAlbum, mount: mountAlbum, nav: 'library', title: titleOf(M.album, 'Album'), meta: metaOf(M.album, albumMeta), place: placeOf('album', M.album) });
  App.route('/artist/:id', { render: renderArtist, mount: mountArtist, nav: 'library', title: titleOf(M.artist, 'Artist'), meta: artistMetaLine, place: placeOf('artist', M.artist) });
  App.route('/tag/:id', { render: renderTag, mount: mountTag, nav: 'library', title: function (ctx) { return M.tag(ctx.params.id).name; }, place: placeOf('tag', M.tag) });
  App.route('/playlists', { render: renderPlaylists, mount: mountPlaylists, nav: 'playlists', title: 'Playlists' });
  App.route('/outside/:id', { render: renderOutside, mount: mountOutside, nav: 'playlists',
    title: function (ctx) { var d = M.peek(outsidePath(ctx.params.id)); return d ? d.name : 'Playlist'; },
    place: placeOf('outside', function (id) { return M.peek(outsidePath(id)); }) });
  App.route('/playlist/:id', { render: renderPlaylist, mount: mountPlaylist, nav: 'playlists', title: titleOf(playlistOf, 'Playlist'), meta: metaOf(playlistOf, playlistMeta), place: placeOf('playlist', playlistOf) });
})();
