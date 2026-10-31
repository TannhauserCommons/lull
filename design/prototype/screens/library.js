/* screens/library.js: Library and Playlists.
   Routes: /library/:segment?, /album/:id, /artist/:id, /tag/:id, /liked, /history, /following,
           /playlists, /playlist/:id.
   Hooks owned here: App.hooks.addToPlaylist (picker sheet), App.hooks.importFile (M3U preview).
   All data is mock (App.data); playlists live in App.store('playlists') and are not persisted.
   Reading order: helpers, library frame + segments, album, artist, tag, liked, history, following,
   playlists list, playlist page (table, reorder, edit, export, delete), picker, M3U import, states. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, raw = App.raw, icon = App.icon, $ = App.$, $$ = App.$$, fmt = App.fmt;
  var TODAY = '2026-10-31';            // the prototype's fixed "now" (matches the mock data)

  /* =====================================================================
     1. Small helpers
     ===================================================================== */
  function plural(n, one, many) { return n + ' ' + (n === 1 ? one : (many || one + 's')); }
  function sumDuration(ids) { return ids.reduce(function (s, id) { var t = M.track(id); return s + (t ? t.duration : 0); }, 0); }
  function isSaved(id) { return App.isSaved(id); }
  function isLiked(id) { return App.isLiked(id); }
  function playlists() { return App.store.get('playlists'); }
  function playlist(id) { return playlists().filter(function (p) { return p.id === id; })[0] || null; }
  function plIds(p) { return p.items.map(function (i) { return i.trackId; }); }
  function jellyfinDown() { var s = App.store.get('services'); return !!(s && s.jellyfin && s.jellyfin.state === 'down'); }
  function savedCount(album) { return album.trackIds.filter(isSaved).length; }
  function hasTag(item, tag) { return item.tags.indexOf(tag) >= 0; }
  function byTitle(a, b) { return (a.title || a.name).localeCompare(b.title || b.name); }

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
  /* re-render when any of these store keys change (unsubscribed when the route changes) */
  function liveOn(ctx, keys, fn) {
    ctx.onCleanup(App.store.subscribe(function (changed) {
      if (changed.some(function (k) { return keys.indexOf(k) >= 0; })) (fn || refreshKeepFocus)();
    }));
  }
  function copyLink(path) {
    var url = location.href.split('#')[0] + '#' + path, done = function () { App.toast('Link copied', { icon: 'link' }); };
    try { navigator.clipboard.writeText(url).then(done, done); } catch (e) { done(); }
  }
  function notFound(kind) {
    return html`<div class="view__inner lib"><div class="card">${ui.emptyState({ art: 'star', title: 'That ' + kind + ' isn\'t here',
      text: 'It may have been removed or renamed. Your library still has everything you saved.', action: { label: 'Open library', href: '#/library' } })}</div></div>`;
  }

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
  function jellyfinNotice() {
    if (!jellyfinDown()) return '';
    return html`<div class="lib-notice s-card" role="status">${ui.statusGlyph('down', 'Jellyfin offline')}
      <p><b>Jellyfin is offline.</b> This is your library as last synced; saved songs can't play until it's back. Soulseek still works.</p>
      <button class="btn btn--sm" data-act="lib-jellyfin-retry">${icon('refresh')}Retry connection</button></div>`;
  }
  function albumStatus(a) {
    var n = savedCount(a), total = a.trackIds.length;
    if (n === total) return ui.savedBadge();
    if (n > 0) return html`<span class="badge badge-saved lib-partial" aria-label="${n} of ${total} songs saved">${icon('saved')}${n} of ${total}</span>`;
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
      $$('[data-lib-album-status]', ctx.el).forEach(function (el) { el.innerHTML = String(albumStatus(M.album(el.getAttribute('data-lib-album-status')))); });
    }));
  }

  /* tiles */
  function albumTile(a) {
    return html`<div class="tile lib-tile" data-album-id="${a.id}" draggable="true">
      <div class="tile__art"><a href="#/album/${a.id}" aria-label="${a.title} by ${a.artist}">${ui.cover(a, '100%')}</a>
        <button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/album/${a.id}">${a.title}</a></div>
      <div class="tile__by"><a href="#/artist/${a.artistId}">${a.artist}</a></div>
      <div class="lib-tile__meta"><span data-lib-album-status="${a.id}">${albumStatus(a)}</span><span class="px">${a.type === 'album' ? a.year : a.type.toLowerCase()}</span></div>
    </div>`;
  }
  function artistTile(ar, meta) {
    return html`<div class="tile lib-tile lib-tile--artist">
      <div class="tile__art"><a href="#/artist/${ar.id}" aria-label="${ar.name}">${ui.cover(ar, '100%', { round: true })}</a>
        <button class="tile__play" data-act="start-radio" data-kind="artist" data-id="${ar.id}" aria-label="Start ${ar.name} radio" data-tip="Artist radio">${icon('radio')}</button></div>
      <div class="tile__title"><a href="#/artist/${ar.id}">${ar.name}</a></div>
      <div class="tile__by">${meta}</div>
    </div>`;
  }
  function artistMeta(ar) {
    var n = M.tracksBy(ar.id).filter(isSaved).length;
    return App.isFollowed(ar.id) ? html`${icon('check', null, 'ic-16')} following${n ? ', ' + n + ' saved' : ''}` : n ? plural(n, 'saved song') : (ar.listeners / 1000).toFixed(1) + 'k listeners';
  }

  /* =====================================================================
     2. Library home: frame + segments (Albums, Artists, Songs, Genres & tags, Liked, History, Playlists, Following)
     ===================================================================== */
  var SEGMENTS = [
    { id: 'albums', label: 'Albums', href: '#/library/albums' },
    { id: 'artists', label: 'Artists', href: '#/library/artists' },
    { id: 'songs', label: 'Songs', href: '#/library/songs' },
    { id: 'genres', label: 'Genres & tags', href: '#/library/genres' },
    { id: 'liked', label: 'Liked', href: '#/liked' },
    { id: 'history', label: 'History', href: '#/history' },
    { id: 'playlists', label: 'Playlists', href: '#/playlists' },
    { id: 'following', label: 'Following', href: '#/following' }
  ];
  var prefs = { albumFilter: App.storage('libAlbumFilter') || 'all', albumSort: App.storage('libAlbumSort') || 'recent', songSort: App.storage('libSongSort') || 'album' };

  function libraryFrame(seg, body, ctx) {
    var empty = /* PROTO: demo state */ ctx.state === 'library-empty';
    var saved = empty ? [] : M.tracks.filter(function (t) { return isSaved(t.id); });
    var albums = M.albums.filter(function (a) { return savedCount(a) > 0; }).length;
    var gb = saved.reduce(function (s, t) { return s + t.sizeMB; }, 0) / 1024;
    var sub = empty ? 'Nothing saved yet. Songs you save from Soulseek land here and in Jellyfin.'
      : plural(saved.length, 'song') + ' saved across ' + plural(albums, 'album') + ', ' + gb.toFixed(1) + ' GB on your server.';
    var tools = html`<button class="btn" data-act="lib-import">${icon('import')}Import playlist</button>
      ${ui.iconButton('refresh', 'Rescan Jellyfin library', { 'data-act': 'jellyfin-rescan' })}`;
    return html`<div class="view__inner lib">
      ${pageHead('library', sub, tools)}
      <nav class="lib-segs" aria-label="Library sections"><ul>${SEGMENTS.map(function (s) {
        return html`<li><a class="pill-filter" href="${s.href}" ${s.id === seg ? raw('aria-current="page"') : ''}>${s.label}</a></li>`;
      })}</ul></nav>
      ${jellyfinNotice()}
      ${body}
    </div>`;
  }
  function emptyLibraryCard() {
    return html`<section class="card lib-card" aria-label="Library">${ui.emptyState({ art: 'slot', title: 'Nothing saved yet',
      text: 'Save songs to keep them: they are tagged, filed and added to Jellyfin. Until then everything streams from Soulseek.',
      action: { label: 'Search Soulseek', href: '#/search' }, secondary: { label: 'Import a playlist', act: 'lib-import' } })}</section>`;
  }

  var ALBUM_FILTERS = [['all', 'All'], ['saved', 'Saved'], ['partial', 'Partly saved'], ['none', 'Not saved yet']];
  var ALBUM_SORTS = {
    recent: ['Newest first', function (a, b) { return b.released.localeCompare(a.released); }],
    title: ['Title', byTitle],
    artist: ['Artist', function (a, b) { return a.artist.localeCompare(b.artist); }]
  };
  function albumState(a) { var n = savedCount(a); return n === a.trackIds.length ? 'saved' : n ? 'partial' : 'none'; }
  function sortButton(act, label) {
    return html`<button class="btn btn--quiet btn--sm lib-sort" data-act="${act}" aria-haspopup="menu">${icon('sort')}${label}${icon('chevron-down', null, 'ic-16')}</button>`;
  }
  function albumsSegment() {
    var f = prefs.albumFilter;
    var list = M.albums.filter(function (a) { return f === 'all' || albumState(a) === f; }).sort(ALBUM_SORTS[prefs.albumSort][1]);
    return html`<section class="card lib-card" aria-labelledby="lib-albums-h">
      ${ruleWith('albums', 'lib-albums-h', sortButton('lib-album-sort', ALBUM_SORTS[prefs.albumSort][0]))}
      <div class="lib-filters" role="group" aria-label="Show albums">${ALBUM_FILTERS.map(function (x) {
        var n = M.albums.filter(function (a) { return x[0] === 'all' || albumState(a) === x[0]; }).length;
        return html`<button class="pill-filter" data-act="lib-album-filter" data-value="${x[0]}" aria-pressed="${f === x[0] ? 'true' : 'false'}">${x[1]}<span class="lib-count">${n}</span></button>`;
      })}</div>
      ${list.length ? html`<div class="lib-grid">${list.map(albumTile)}</div>`
        : ui.emptyState({ art: 'slot', title: 'No albums here', text: 'Nothing matches this filter.', action: { label: 'Show all albums', act: 'lib-album-filter', id: 'all' } })}
    </section>`;
  }
  function artistsSegment() {
    var list = M.artists.slice().sort(function (a, b) { return (App.isFollowed(b.id) - App.isFollowed(a.id)) || a.name.localeCompare(b.name); });
    return html`<section class="card lib-card" aria-labelledby="lib-artists-h">
      ${ruleWith('artists', 'lib-artists-h', html`<a class="btn btn--quiet btn--sm" href="#/following">Manage following</a>`)}
      <div class="lib-grid lib-grid--artists">${list.map(function (ar) { return artistTile(ar, artistMeta(ar)); })}</div>
    </section>`;
  }
  var SONG_SORTS = {
    album: ['Album order', function (a, b) { return a.album.localeCompare(b.album) || a.no - b.no; }],
    title: ['Title', byTitle],
    artist: ['Artist', function (a, b) { return a.artist.localeCompare(b.artist) || a.no - b.no; }],
    plays: ['Most played', function (a, b) { return b.plays - a.plays; }]
  };
  function songsSegment() {
    var list = M.tracks.filter(function (t) { return isSaved(t.id); }).sort(SONG_SORTS[prefs.songSort][1]);
    var ids = list.map(function (t) { return t.id; });
    return html`<section class="card lib-card" aria-labelledby="lib-songs-h">
      ${ruleWith('saved songs', 'lib-songs-h', sortButton('lib-song-sort', SONG_SORTS[prefs.songSort][0]))}
      <div class="lib-bar">
        <button class="btn btn--primary" data-act="play-ids" data-ids="${ids.join(',')}">${icon('play')}Play all</button>
        <button class="btn" data-act="lib-shuffle-ids" data-ids="${ids.join(',')}">${icon('shuffle')}Shuffle</button>
        <span class="lib-facts px">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</span>
      </div>
      ${ui.trackList(list, { label: 'Saved songs', unavailable: jellyfinDown() })}
    </section>`;
  }
  function genresSegment() {
    return html`<section class="card lib-card" aria-labelledby="lib-genres-h">
      ${ruleWith('genres & tags', 'lib-genres-h', html`<span class="t-micro fg-2">Tags from MusicBrainz and Last.fm</span>`)}
      <ul class="lib-tags">${M.tags.map(function (tag) {
        var tracks = M.tracks.filter(function (t) { return hasTag(t, tag.id); });
        var artists = M.artists.filter(function (a) { return hasTag(a, tag.id); });
        return html`<li class="lib-tagcard inset">
          <a class="lib-tagcard__art" href="#/tag/${encodeURIComponent(tag.id)}" tabindex="-1" aria-hidden="true">${ui.mosaic(tracks.map(function (t) { return t.id; }), 72)}</a>
          <div class="lib-tagcard__main">
            <h3><a href="#/tag/${encodeURIComponent(tag.id)}">${tag.name}</a></h3>
            <p class="lib-tagcard__meta">${plural(artists.length, 'artist')}, ${plural(tracks.filter(function (t) { return isSaved(t.id); }).length, 'saved song')}</p>
            <p class="lib-tagcard__desc">${tag.desc}</p>
          </div>
          <div class="lib-tagcard__tools"><button class="chip" data-act="follow-tag" data-id="${tag.id}" aria-pressed="${App.isFollowed(tag.id) ? 'true' : 'false'}" aria-label="Follow ${tag.name}">${icon(App.isFollowed(tag.id) ? 'check' : 'plus')}${App.isFollowed(tag.id) ? 'Following' : 'Follow'}</button>
            ${ui.iconButton('radio', 'Start ' + tag.name + ' radio', { 'data-act': 'start-radio', 'data-kind': 'tag', 'data-id': tag.id, cls: 'btn-icon--sm' })}</div>
        </li>`;
      })}</ul>
    </section>`;
  }

  function renderLibrary(ctx) {
    var seg = ctx.params.segment || 'albums';
    if (seg === 'liked') return libraryFrame('liked', likedBody(ctx), ctx);
    if (seg === 'history') return libraryFrame('history', historyBody(ctx), ctx);
    if (seg === 'following') return libraryFrame('following', followingBody(ctx), ctx);
    /* PROTO: demo state */ if (ctx.state === 'library-empty') return libraryFrame(seg, emptyLibraryCard(), ctx);
    var body = seg === 'artists' ? artistsSegment() : seg === 'songs' || seg === 'saved' ? songsSegment()
      : seg === 'genres' ? genresSegment() : albumsSegment();
    return libraryFrame(seg === 'saved' ? 'songs' : seg, body, ctx);
  }
  function mountLibrary(ctx) { liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']); }

  /* =====================================================================
     3. Album page: one cream card, art left, title block, disc headers
     ===================================================================== */
  function renderAlbum(ctx) {
    var a = M.album(ctx.params.id); if (!a) return notFound('album');
    var tracks = a.trackIds.map(M.track), down = jellyfinDown();
    var discs = []; tracks.forEach(function (t) { (discs[t.disc - 1] = discs[t.disc - 1] || []).push(t); });
    var related = M.albums.filter(function (x) { return x.id !== a.id && x.tags.some(function (g) { return hasTag(a, g); }); });
    return html`<div class="view__inner lib">
      ${jellyfinNotice()}
      <article class="card lib-page" aria-labelledby="lib-album-title" data-meta="${albumMeta(a)}" data-tracks="${tracks.length}" data-duration="${a.duration}" data-size="${a.sizeMB}">
        <div class="lib-hero">
          ${ui.cover(a, 'var(--lib-art)', { label: 'Cover of ' + a.title })}
          <div class="lib-hero__info">
            <p class="lib-kicker">${a.type.toLowerCase()}, ${a.year}</p>
            <h1 class="t-display lib-title" id="lib-album-title">${a.title}</h1>
            <p class="lib-hero__by"><a href="#/artist/${a.artistId}">${a.artist}</a></p>
            <div class="lib-hero__facts">
              <span data-lib-album-status="${a.id}">${albumStatus(a)}</span>${ui.qualityChip(a.format)}${a.isNew ? ui.newBadge() : ''}
              <span class="px">${plural(tracks.length, 'song')}, ${fmt.length(a.duration)}</span>
            </div>
            <p class="lib-hero__fmt" aria-hidden="true">${fmt.albumLine(a)}</p>
            <div class="lib-chips">${a.tags.map(function (g) { return ui.tagChip(g); })}</div>
            <div class="lib-actions">
              <button class="btn btn--primary" data-act="play-album" data-id="${a.id}">${icon('play', null, 'ic-play')}Play</button>
              <button class="btn" data-act="shuffle-album" data-id="${a.id}">${icon('shuffle')}Shuffle</button>
              ${groupSaveButton(a.trackIds, 'album')}
              <span class="lib-actions__icons">
                ${likeAllButton(a.trackIds)}
                ${ui.iconButton('playlists', 'Add album to playlist', { 'data-act': 'add-to-playlist', 'data-ids': a.trackIds.join(','), 'aria-haspopup': 'dialog' })}
                ${ui.iconButton('radio', 'Start album radio', { 'data-act': 'start-radio', 'data-kind': 'album', 'data-id': a.id })}
                ${ui.iconButton('more', 'More album actions', { 'data-act': 'lib-album-menu', 'data-id': a.id, 'aria-haspopup': 'menu' })}
              </span>
            </div>
          </div>
        </div>
        ${discs.map(function (list, i) {
          return html`${discs.length > 1 ? html`<div class="label-rule label-rule--sm lib-disc"><h2>disc ${i + 1}</h2><span class="label-rule__end t-micro fg-2">${plural(list.length, 'song')}, ${fmt.length(sumDuration(list.map(function (t) { return t.id; })))}</span></div>` : ''}
          <ol class="tlist" data-ids="${a.trackIds.join(',')}" aria-label="${discs.length > 1 ? 'Disc ' + (i + 1) + ' of ' + a.title : 'Songs on ' + a.title}">${list.map(function (t, j) {
            return ui.trackRow(t, { index: j + 1, unavailable: down && isSaved(t.id) });
          })}</ol>`;
        })}
        <p class="lib-foot">Released ${fmt.date(a.released, true)} on ${a.label}. ${a.sizeMB} MB as ${a.format}${savedCount(a) ? ', saved songs are in Jellyfin.' : ', streaming from Soulseek until you save it.'}</p>
      </article>
      ${related.length ? html`<section class="card lib-card" aria-labelledby="lib-related-h">${ruleWith('more like this', 'lib-related-h')}
        <div class="shelf">${related.slice(0, 8).map(albumTile)}</div></section>` : ''}
    </div>`;
  }
  function mountAlbum(ctx) { liveSaveButtons(ctx); liveOn(ctx, ['liked', 'services']); }

  /* =====================================================================
     4. Artist page: round art, follow, radio, top songs, discography, similar
     ===================================================================== */
  function renderArtist(ctx) {
    var ar = M.artist(ctx.params.id); if (!ar) return notFound('artist');
    var top = M.tracksBy(ar.id).slice().sort(function (a, b) { return b.plays - a.plays; }).slice(0, 5);
    var albums = M.albumsBy(ar.id);
    var similar = M.artists.filter(function (x) { return x.id !== ar.id && x.tags.some(function (g) { return hasTag(ar, g); }); });
    var topIds = top.map(function (t) { return t.id; });
    return html`<div class="view__inner lib">
      ${jellyfinNotice()}
      <section class="card lib-page" aria-labelledby="lib-artist-title">
        <div class="lib-hero lib-hero--round">
          ${ui.cover(ar, 'var(--lib-art)', { round: true, label: ar.name })}
          <div class="lib-hero__info">
            <p class="lib-kicker">artist, ${ar.listeners.toLocaleString('en-US')} listeners</p>
            <h1 class="t-display lib-title" id="lib-artist-title">${ar.name}</h1>
            <div class="lib-chips">${ar.tags.map(function (g) { return ui.tagChip(g); })}</div>
            <p class="lib-bio">${ar.bio}</p>
            <div class="lib-actions">
              <button class="btn btn--primary" data-act="start-radio" data-kind="artist" data-id="${ar.id}">${icon('radio')}Artist radio</button>
              ${followButton(ar.id)}
              <button class="btn" data-act="play-ids" data-ids="${topIds.join(',')}">${icon('play', null, 'ic-play')}Play top songs</button>
              <span class="lib-actions__icons">${ui.iconButton('more', 'More artist actions', { 'data-act': 'lib-artist-menu', 'data-id': ar.id, 'aria-haspopup': 'menu' })}</span>
            </div>
          </div>
        </div>
      </section>
      <div class="lib-cols">
        <section class="card lib-card" aria-labelledby="lib-top-h">
          ${ruleWith('top songs', 'lib-top-h', html`<span class="t-micro fg-2">by your plays</span>`)}
          ${ui.trackList(top, { label: 'Top songs by ' + ar.name, unavailable: jellyfinDown() })}
        </section>
        <section class="card lib-card" aria-labelledby="lib-disco-h">
          ${ruleWith('discography', 'lib-disco-h', html`<span class="t-micro fg-2">${plural(albums.length, 'release')}</span>`)}
          <div class="shelf">${albums.map(albumTile)}</div>
        </section>
      </div>
      ${similar.length ? html`<section class="card lib-card" aria-labelledby="lib-similar-h">${ruleWith('similar artists', 'lib-similar-h', html`<span class="t-micro fg-2">via Last.fm similar artists</span>`)}
        <div class="shelf">${similar.map(function (x) { return artistTile(x, artistMeta(x)); })}</div></section>` : ''}
    </div>`;
  }
  function mountArtist(ctx) { liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']); }

  /* =====================================================================
     5. Genre / tag page: follow, tag radio, top artists, releases, top songs
     ===================================================================== */
  function renderTag(ctx) {
    var id = ctx.params.id, tag = M.tag(id) || { id: id, name: id, desc: 'No description yet. Tags come from MusicBrainz and Last.fm.', related: [] };
    var albums = M.albums.filter(function (a) { return hasTag(a, id); }).sort(function (a, b) { return b.released.localeCompare(a.released); });
    var artists = M.artists.filter(function (a) { return hasTag(a, id); });
    var tracks = M.tracks.filter(function (t) { return hasTag(t, id); });
    var top = tracks.slice().sort(function (a, b) { return b.plays - a.plays; }).slice(0, 6);
    var on = App.isFollowed(id);
    return html`<div class="view__inner lib">
      <section class="card lib-page" aria-labelledby="lib-tag-title">
        <div class="lib-hero">
          ${ui.mosaic(tracks.map(function (t) { return t.id; }), 'var(--lib-art)', 'Covers tagged ' + tag.name)}
          <div class="lib-hero__info">
            <p class="lib-kicker">tag, ${plural(artists.length, 'artist')}, ${plural(albums.length, 'release')}</p>
            <h1 class="t-display lib-title" id="lib-tag-title">${tag.name}</h1>
            <p class="lib-bio">${tag.desc}</p>
            <div class="lib-actions">
              <button class="btn btn--primary" data-act="start-radio" data-kind="tag" data-id="${id}">${icon('radio')}Tag radio</button>
              <button class="btn lib-toggle" data-act="follow-tag" data-id="${id}" aria-pressed="${on ? 'true' : 'false'}">${icon(on ? 'check' : 'follow')}${on ? 'Following' : 'Follow tag'}</button>
              <button class="btn" data-act="play-ids" data-ids="${top.map(function (t) { return t.id; }).join(',')}">${icon('play', null, 'ic-play')}Play top songs</button>
            </div>
            ${tag.related.length ? html`<div class="lib-chips"><span class="t-micro fg-2">related</span>${tag.related.map(function (g) { return ui.tagChip(g); })}</div>` : ''}
          </div>
        </div>
      </section>
      ${artists.length ? html`<section class="card lib-card" aria-labelledby="lib-tagartists-h">${ruleWith('top artists', 'lib-tagartists-h')}
        <div class="shelf">${artists.map(function (x) { return artistTile(x, artistMeta(x)); })}</div></section>` : ''}
      <div class="lib-cols">
        <section class="card lib-card" aria-labelledby="lib-tagrel-h">
          ${ruleWith('releases', 'lib-tagrel-h', html`<span class="t-micro fg-2">newest first</span>`)}
          ${albums.length ? html`<div class="lib-grid lib-grid--tight">${albums.map(albumTile)}</div>`
            : ui.emptyState({ art: 'moon', title: 'No releases yet', text: 'Nothing in this tag so far. Tag radio still finds songs on Soulseek.' })}
        </section>
        <section class="card lib-card" aria-labelledby="lib-tagtop-h">
          ${ruleWith('top songs', 'lib-tagtop-h')}
          ${top.length ? ui.trackList(top, { label: 'Top songs tagged ' + tag.name, unavailable: jellyfinDown() })
            : ui.emptyState({ art: 'wave', title: 'No songs yet', text: 'Play tag radio to hear what this tag sounds like.' })}
        </section>
      </div>
    </div>`;
  }
  function mountTag(ctx) { liveSaveButtons(ctx); liveOn(ctx, ['followed', 'liked', 'services']); }

  /* =====================================================================
     6. Liked, History, Following (library segments with their own routes)
     ===================================================================== */
  function likedBody(ctx) {
    var ids = /* PROTO: demo state */ ctx.state === 'library-liked-empty' ? [] : Object.keys(App.store.get('liked')).reverse();
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
          </div>
        </div>
      </div>
      ${ui.trackList(ids, { label: 'Liked songs, newest first', unavailable: jellyfinDown() })}
    </section>`;
  }

  /* a pending scrobble counts as sent once Last.fm has nothing queued */
  function scrobbleOf(item) {
    var lf = App.store.get('services').lastfm;
    return item.scrobble === 'pending' && lf.state === 'ok' && !lf.queued ? 'done' : item.scrobble;
  }
  function historyBody(ctx) {
    /* PROTO: demo state */ if (ctx.state === 'library-history-empty') {
      return html`<section class="card lib-card" aria-label="History">${ui.emptyState({ art: 'wave', title: 'Nothing played yet',
        text: 'Songs you play show up here by day, with their Last.fm scrobble status.', action: { label: 'Go home', href: '#/home' } })}</section>`;
    }
    var waiting = M.history.reduce(function (n, d) { return n + d.items.filter(function (i) { return scrobbleOf(i) === 'pending'; }).length; }, 0);
    var status = waiting
      ? html`${ui.statusGlyph('warn', 'Waiting')}<p><b>${plural(waiting, 'scrobble')} waiting for Last.fm.</b> They stay queued and retry on their own.</p><button class="btn btn--sm" data-act="lastfm-retry">${icon('refresh')}Retry now</button>`
      : html`${ui.statusGlyph('ok', 'Up to date')}<p><b>Every eligible play is scrobbled.</b> Songs under 30 seconds are skipped, as Last.fm asks.</p>`;
    return html`<section class="card lib-card" aria-labelledby="lib-history-h">
      ${ruleWith('history', 'lib-history-h', html`<ul class="lib-legend" aria-label="Scrobble status key">
        <li>${icon('check', null, 'ic-16')}scrobbled</li><li>${icon('clock', null, 'ic-16')}pending</li><li>${icon('dash', null, 'ic-16')}too short</li></ul>`)}
      <div class="lib-status inset">${status}</div>
      ${M.history.map(function (day) {
        var ids = day.items.map(function (i) { return i.trackId; });
        return html`<div class="lib-day">${ui.labelRule(day.day, { level: 'h3', sm: true })}
          <ol class="tlist lib-history" data-ids="${ids.join(',')}" aria-label="Played ${day.day}">${day.items.map(function (i) {
            return ui.trackRow(i.trackId, { index: i.at, draggable: false, unavailable: jellyfinDown() && isSaved(i.trackId), badges: ui.scrobble(scrobbleOf(i)) });
          })}</ol></div>`;
      })}
    </section>`;
  }

  function followingBody(ctx) {
    var none = /* PROTO: demo state */ ctx.state === 'library-following-empty';
    var artists = none ? [] : M.artists.filter(function (a) { return App.isFollowed(a.id); });
    var tags = none ? [] : M.tags.filter(function (t) { return App.isFollowed(t.id); });
    var suggest = M.tags.filter(function (t) { return !App.isFollowed(t.id); });
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
          <div class="lib-chips">${suggest.map(function (t) { return ui.tagChip(t.id, { toggle: true, pressed: false }); })}</div></div>` : ''}
      </section>
    </div>`;
  }

  /* =====================================================================
     7. Playlists list
     ===================================================================== */
  function playlistTile(p) {
    var ids = plIds(p);
    return html`<div class="tile lib-tile lib-tile--pl" data-drop="playlist:${p.id}">
      <div class="tile__art"><a href="#/playlist/${p.id}" aria-label="${p.name}, ${plural(ids.length, 'song')}">${ui.mosaic(ids, '100%')}</a>
        <button class="tile__play" data-act="play-playlist" data-id="${p.id}" aria-label="Play ${p.name}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><a href="#/playlist/${p.id}">${p.name}</a></div>
      <div class="tile__by">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</div>
      <div class="lib-tile__meta t-micro">updated ${fmt.date(p.updated)}</div>
    </div>`;
  }
  function dropZone() {
    return html`<div class="lib-dropzone" data-lib-dropzone>
      ${icon('import', null, 'ic-24')}
      <p><b>Drop an .m3u or .m3u8 file</b> anywhere in the window to import it.</p>
      <button class="btn btn--sm" data-act="lib-import-pick">Choose file</button>
      <button class="btn btn--quiet btn--sm" data-act="lib-import-sample">Try a sample playlist</button>
    </div>`;
  }
  function renderPlaylists(ctx) {
    var list = /* PROTO: demo state */ ctx.state === 'library-playlists-empty' ? [] : playlists();
    var songs = list.reduce(function (n, p) { return n + p.items.length; }, 0);
    var tools = html`<button class="btn btn--primary" data-act="lib-pl-new">${icon('plus')}New playlist</button>
      <button class="btn" data-act="lib-import">${icon('import')}Import M3U</button>`;
    return html`<div class="view__inner lib">
      ${pageHead('playlists', list.length ? plural(list.length, 'playlist') + ', ' + plural(songs, 'song') + '. Drag songs onto a playlist to add them.' : 'Make one, or bring one over from another player.', tools)}
      <section class="card lib-card" aria-labelledby="lib-pls-h">
        ${ruleWith('your playlists', 'lib-pls-h', html`<span class="t-micro fg-2">recently updated first</span>`)}
        ${list.length ? html`<div class="lib-grid">
          <button class="lib-newtile" data-act="lib-pl-new"><span class="lib-newtile__box">${icon('plus', null, 'ic-24')}</span><span class="tile__title">New playlist</span><span class="tile__by">name it, then add songs</span></button>
          ${list.slice().sort(function (a, b) { return b.updated.localeCompare(a.updated); }).map(playlistTile)}
        </div>` : ui.emptyState({ art: 'slot', title: 'No playlists yet', text: 'Make a playlist and drag songs into it, or import an M3U from another player.',
          action: { label: 'New playlist', act: 'lib-pl-new' }, secondary: { label: 'Import M3U', act: 'lib-import' } })}
        ${dropZone()}
      </section>
    </div>`;
  }
  function mountPlaylists(ctx) {
    liveOn(ctx, ['playlists']);
    /* PROTO: demo state */ if (ctx.state === 'library-playlist-create') openEdit(null);
    /* PROTO: demo state */ if (ctx.state === 'library-m3u-start') openImportStart();
    /* PROTO: demo state */ if (ctx.state === 'library-m3u-preview') openImportPreview(SAMPLE_NAME, sampleM3U());
    /* PROTO: demo state */ if (ctx.state === 'library-m3u-error') openImportPreview('mixtape.m3u', '#EXTM3U\n#EXTINF:200,Nobody We Know - A Song That Isn\'t Here\n../old/mixtape/missing.mp3\n');
  }

  /* =====================================================================
     8. Playlist page (moodboard image 04): mosaic, big title, pixel-face count + duration, table
     ===================================================================== */
  var SORTS = {
    custom: { label: 'Custom order', fn: null },
    added: { label: 'Date added', fn: function (a, b) { return b.it.added.localeCompare(a.it.added); } },
    title: { label: 'Title', fn: function (a, b) { return a.t.title.localeCompare(b.t.title); } },
    album: { label: 'Album', fn: function (a, b) { return a.t.album.localeCompare(b.t.album); } },
    time: { label: 'Duration', fn: function (a, b) { return a.t.duration - b.t.duration; } }
  };
  var plSort = 'custom', plSortFor = null, pendingSort = null;   // sort is per visit: it resets when another playlist opens
  function sortedItems(p) {
    var rows = p.items.map(function (it, i) { return { it: it, i: i, t: M.track(it.trackId) }; });
    return SORTS[plSort].fn ? rows.slice().sort(SORTS[plSort].fn) : rows;
  }
  function headCell(key, label, cls) {
    var on = plSort === key;
    return html`<button class="lib-th ${cls}" data-act="lib-pl-sort" data-value="${key}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Sort by ${SORTS[key].label.toLowerCase()}">${label}${on && key !== 'custom' ? icon('chevron-down', null, 'ic-16') : ''}</button>`;
  }
  function renderPlaylist(ctx) {
    var p = playlist(ctx.params.id); if (!p) return notFound('playlist');
    if (plSortFor !== p.id) { plSort = pendingSort || 'custom'; plSortFor = p.id; pendingSort = null; }
    var ids = plIds(p), rows = sortedItems(p), custom = plSort === 'custom', down = jellyfinDown();
    var missing = ids.filter(function (id) { return !isSaved(id); }).length;
    var table = ids.length ? html`<div class="lib-table">
        <div class="lib-thead" role="group" aria-label="Sort songs">
          ${headCell('custom', '#', 'lib-th--num')}${headCell('title', 'title', 'lib-th--title')}${headCell('album', 'album', 'lib-th--album')}
          ${headCell('added', 'date added', 'lib-th--added')}<span class="lib-th lib-th--like" aria-hidden="true">${icon('heart', null, 'ic-16')}</span>
          ${headCell('time', icon('clock', 'Duration', 'ic-16'), 'lib-th--dur')}
        </div>
        <ol class="tlist tlist--dated lib-pl-list" data-ids="${rows.map(function (r) { return r.it.trackId; }).join(',')}" data-drop="lib-pl:${p.id}" data-pl="${p.id}"
          aria-label="Songs in ${p.name}${custom ? '. Alt plus arrow keys reorder, Delete removes' : ''}">${rows.map(function (r, n) {
          return ui.trackRow(r.t, { index: n + 1, added: r.it.added, draggable: custom, unavailable: down && isSaved(r.t.id) });
        })}</ol>
      </div>`
      : html`<div class="lib-dropzone lib-dropzone--songs ${/* PROTO: demo state */ ctx.state === 'library-playlist-drop' ? 'is-drop-target' : ''}" data-drop="playlist:${p.id}">
          ${ui.emptyState({ art: 'slot', title: /* PROTO: demo state */ ctx.state === 'library-playlist-drop' ? 'Drop to add 3 songs' : 'Drag songs here',
            text: 'Drop songs from search, an album or the queue, or use Add to playlist on any song.', action: { label: 'Find songs', href: '#/search' }, secondary: { label: 'Import M3U', act: 'lib-import' } })}
        </div>`;
    return html`<div class="view__inner lib">
      ${jellyfinNotice()}
      <article class="card card--flush lib-page lib-pl" aria-labelledby="lib-pl-title" data-meta="${playlistMeta(p)}">
        <header class="lib-pl__head">
          ${ui.mosaic(ids, 'var(--lib-art)', 'Playlist cover: ' + p.name)}
          <div class="lib-hero__info">
            <p class="lib-kicker">playlist</p>
            <h1 class="t-title-xl lib-pl__title" id="lib-pl-title">${p.name}</h1>
            ${p.desc ? html`<p class="lib-bio">${p.desc}</p>` : html`<button class="btn btn--quiet btn--sm lib-adddesc" data-act="lib-pl-edit" data-id="${p.id}">${icon('edit')}Add a description</button>`}
            <p class="lib-pl__facts"><span class="px">${plural(ids.length, 'song')}, ${fmt.length(sumDuration(ids))}</span><span class="lib-pl__upd">updated ${fmt.date(p.updated, true)}</span></p>
          </div>
        </header>
        <div class="lib-pl__bar">
          <button class="btn btn--primary" data-act="play-playlist" data-id="${p.id}" ${ids.length ? '' : raw('aria-disabled="true"')}>${icon('play', null, 'ic-play')}Play</button>
          ${ui.iconButton('shuffle', 'Shuffle play', { 'data-act': 'lib-shuffle-ids', 'data-ids': ids.join(',') })}
          ${ids.length && missing ? groupSaveButton(ids, 'playlist') : ''}
          ${ui.iconButton('edit', 'Edit name and description', { 'data-act': 'lib-pl-edit', 'data-id': p.id, 'aria-haspopup': 'dialog' })}
          ${ui.iconButton('export', 'Export as M3U', { 'data-act': 'lib-pl-export', 'data-id': p.id, 'aria-haspopup': 'dialog' })}
          ${ui.iconButton('more', 'More playlist actions', { 'data-act': 'lib-pl-menu', 'data-id': p.id, 'aria-haspopup': 'menu' })}
          <span class="spacer"></span>
          ${ids.length ? sortButton('lib-pl-sortmenu', SORTS[plSort].label) : ''}
        </div>
        ${!custom && ids.length ? html`<p class="lib-sortnote">${icon('info', null, 'ic-16')}Sorted by ${SORTS[plSort].label.toLowerCase()}. <button class="btn btn--quiet btn--sm" data-act="lib-pl-sort" data-value="custom">Back to custom order</button> to drag or reorder.</p>` : ''}
        ${table}
      </article>
    </div>`;
  }

  /* playlist mutations: every change goes through the store, so the rail and this page re-render */
  function changePlaylist(p, fn) { App.store.update('playlists', function () { fn(p); p.updated = TODAY; }); }
  function focusRow(i) { var rows = $$('.lib-pl-list .trow'); var r = rows[App.clamp(i, 0, rows.length - 1)]; if (r) { rows.forEach(function (x) { x.classList.remove('is-selected'); }); r.classList.add('is-selected'); r.focus(); } }
  /* move the items at `from` (indices, ascending) so they land before old index `to` */
  function moveItems(p, from, to) {
    var before = from.filter(function (i) { return i < to; }).length, at = to - before;
    changePlaylist(p, function () {
      var moving = from.map(function (i) { return p.items[i]; });
      var rest = p.items.filter(function (it, i) { return from.indexOf(i) < 0; });
      rest.splice.apply(rest, [at, 0].concat(moving));
      p.items = rest;
    });
    App.announce('Moved to position ' + (at + 1) + ' of ' + p.items.length);
    return at;
  }
  function insertItems(p, ids, at) {
    changePlaylist(p, function () { p.items.splice.apply(p.items, [at, 0].concat(ids.map(function (id) { return { trackId: id, added: TODAY }; }))); });
    App.toast('Added ' + plural(ids.length, 'song') + ' to ' + p.name, { icon: 'playlists', action: 'Undo', onAction: function () { changePlaylist(p, function () { p.items.splice(at, ids.length); }); } });
  }
  function removeItem(p, i) {
    var it = p.items[i], t = M.track(it.trackId);
    changePlaylist(p, function () { p.items.splice(i, 1); });
    App.toast('Removed ' + (t ? t.title : 'song') + ' from ' + p.name, { icon: 'trash', action: 'Undo', onAction: function () { changePlaylist(p, function () { p.items.splice(i, 0, it); }); focusRow(i); } });
    focusRow(i);
  }
  function deletePlaylist(p) {
    var i = playlists().indexOf(p);
    App.store.update('playlists', function (l) { l.splice(i, 1); });
    if (App.current.path === '/playlist/' + p.id) App.go('/playlists');
    App.toast('Deleted ' + p.name, { icon: 'trash', action: 'Undo', onAction: function () {
      App.store.update('playlists', function (l) { l.splice(Math.min(i, l.length), 0, p); });
      App.go('/playlist/' + p.id);
    } });
  }
  function createPlaylist(name, desc, ids) {
    var p = { id: 'pl-' + Date.now().toString(36), name: name, desc: desc || '', created: TODAY, updated: TODAY,
      items: (ids || []).map(function (id) { return { trackId: id, added: TODAY }; }) };
    App.store.update('playlists', function (l) { l.push(p); });
    return p;
  }

  /* drag reorder (custom order only): an insertion line follows the pointer; the drop itself arrives
     through the shell's 'drop' event because the list is a [data-drop] target */
  function mountPlaylist(ctx) {
    var p = playlist(ctx.params.id); if (!p) return;
    liveOn(ctx, ['playlists', 'liked', 'services']);
    liveSaveButtons(ctx);
    // row "more" buttons get a playlist-aware menu (adds move and remove)
    $$('.lib-pl-list .trow__more [data-act="track-menu"]', ctx.el).forEach(function (b) { b.setAttribute('data-act', 'lib-pl-row-menu'); });
    var list = $('.lib-pl-list', ctx.el);
    if (list && plSort === 'custom') {
      var line = document.createElement('li'); line.className = 'drop-line'; line.setAttribute('aria-hidden', 'true');
      var drag = { from: null, at: p.items.length };
      var clear = function () { if (line.parentNode) line.parentNode.removeChild(line); };
      list.addEventListener('dragstart', function (e) {
        var row = e.target.closest('.trow'); if (!row) return;
        var rows = $$('.trow', list), sel = rows.filter(function (r) { return r.classList.contains('is-selected'); });
        drag.from = (row.classList.contains('is-selected') && sel.length > 1 ? sel : [row]).map(function (r) { return rows.indexOf(r); });
      });
      list.addEventListener('dragover', function (e) {
        var rows = $$('.trow', list), at = rows.length;
        for (var i = 0; i < rows.length; i++) { var b = rows[i].getBoundingClientRect(); if (e.clientY < b.top + b.height / 2) { at = i; break; } }
        drag.at = at;
        if (at < rows.length) list.insertBefore(line, rows[at]); else list.appendChild(line);
      });
      list.addEventListener('dragleave', function (e) { if (!list.contains(e.relatedTarget)) clear(); });
      var onEnd = function () { clear(); drag.from = null; };
      document.addEventListener('dragend', onEnd);
      ctx.onCleanup(function () { document.removeEventListener('dragend', onEnd); });
      ctx.onCleanup(App.on('drop', function (d) {
        if (d.dest !== 'lib-pl:' + p.id) return;
        clear();
        if (drag.from) focusRow(moveItems(p, drag.from, drag.at)); else insertItems(p, d.ids, drag.at);
        drag.from = null;
      }));
      /* PROTO: demo state */ if (ctx.state === 'library-playlist-reorder') {         // frozen mid-drag picture for the state switcher
        var rows = $$('.trow', list);
        if (rows[1]) rows[1].classList.add('is-dragging');
        if (rows[5]) list.insertBefore(line, rows[5]);
      }
    }
    // keyboard reorder and remove (the shell emits these from a focused row)
    ctx.onCleanup(App.on('row:move', function (d) {
      if (d.list !== list) return;
      if (plSort !== 'custom') { sortNeeded(); return; }
      var to = d.index + d.dir; if (to < 0 || to >= p.items.length) return;
      moveItems(p, [d.index], d.dir > 0 ? d.index + 2 : d.index - 1); focusRow(to);
    }));
    ctx.onCleanup(App.on('row:delete', function (d) {
      if (d.list !== list) return;
      if (plSort !== 'custom') { sortNeeded(); return; }
      removeItem(p, d.index);
    }));
    /* PROTO: demo state */ if (ctx.state === 'library-playlist-edit') openEdit(p);
    /* PROTO: demo state */ if (ctx.state === 'library-playlist-export') openExport(p);
  }
  function sortNeeded() {
    App.toast('Switch to custom order to reorder or remove songs', { icon: 'sort', action: 'Switch', onAction: function () { plSort = 'custom'; App.refresh(); } });
  }

  /* create / rename / edit description */
  function openEdit(p) {
    var formId = App.uid('lib-edit');
    var d = App.openDialog({
      title: p ? 'edit playlist' : 'new playlist',
      body: html`<form class="lib-form" id="${formId}" novalidate>
        <div class="field"><label class="field__label" for="${formId}-name">Name</label>
          <input class="input" id="${formId}-name" name="name" maxlength="60" autocomplete="off" value="${p ? p.name : ''}" placeholder="late train home" aria-describedby="${formId}-err">
          <p class="field__error" id="${formId}-err" hidden>${icon('warning', null, 'ic-16')}<span></span></p></div>
        <div class="field"><label class="field__label" for="${formId}-desc">Description <span class="fg-2">(optional)</span></label>
          <textarea class="input lib-textarea" id="${formId}-desc" name="desc" rows="3" maxlength="300" placeholder="What it's for, when to play it">${p ? p.desc : ''}</textarea></div>
      </form>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button><button class="btn btn--primary" type="submit" form="${formId}">${p ? 'Save' : 'Create playlist'}</button>`,
      onMount: function (el, close) {
        var form = $('form', el), name = form.elements.name, err = $('.field__error', el);
        name.select();
        form.addEventListener('submit', function (e) {
          e.preventDefault();
          var v = name.value.trim(), clash = playlists().filter(function (x) { return x !== p && x.name.toLowerCase() === v.toLowerCase(); })[0];
          var msg = !v ? 'Give the playlist a name.' : clash ? 'You already have a playlist called ' + clash.name + '.' : '';
          err.hidden = !msg; $('span', err).textContent = msg; name.setAttribute('aria-invalid', msg ? 'true' : 'false');
          if (msg) { name.focus(); return; }
          var desc = form.elements.desc.value.trim();
          close();
          if (!p) { var made = createPlaylist(v, desc); App.go('/playlist/' + made.id); App.toast('Created ' + v + '. Drag songs in, or use Add to playlist.', { icon: 'playlists' }); return; }
          var old = { name: p.name, desc: p.desc };
          changePlaylist(p, function () { p.name = v; p.desc = desc; });
          App.toast(old.name !== v ? 'Renamed to ' + v : 'Playlist details saved', { icon: 'edit', action: 'Undo', onAction: function () { changePlaylist(p, function () { p.name = old.name; p.desc = old.desc; }); } });
        });
      }
    });
    return d;
  }

  /* export: .m3u8 with library paths or stream links, downloaded as a Blob (works from file://) */
  function m3uText(p, mode) {
    var lines = ['#EXTM3U', '#PLAYLIST:' + p.name];
    p.items.forEach(function (it) {
      var t = M.track(it.trackId); if (!t) return;
      if (mode === 'paths' && !isSaved(t.id)) return;
      var ext = (M.formats[t.format] || { codec: 'flac' }).codec.toLowerCase();
      lines.push('#EXTINF:' + t.duration + ',' + t.artist + ' - ' + t.title);
      lines.push(mode === 'paths' ? '/music/' + t.artist + '/' + t.year + ' - ' + t.album + '/' + (t.no < 10 ? '0' : '') + t.no + ' - ' + t.title + '.' + ext
        : 'https://lull.example/stream/' + t.id);
    });
    return lines.join('\n') + '\n';
  }
  function openExport(p) {
    var unsaved = p.items.filter(function (it) { return !isSaved(it.trackId); }).length, file = p.name.replace(/[\\/:*?"<>|]+/g, '') + '.m3u8';
    function preview(mode) {
      var lines = m3uText(p, mode).split('\n');
      return lines.slice(0, 7).join('\n') + (lines.length > 8 ? '\n...' : '');
    }
    App.openDialog({
      title: 'export playlist', wide: true,
      body: html`<div class="lib-form">
        <fieldset class="lib-choice"><legend class="field__label">Write each song as</legend>
          <label class="lib-choice__opt"><input type="radio" name="lib-exp" value="paths" checked><span><b>Library paths</b><span class="fg-2">For players on the same server, like Jellyfin or a desktop player.${unsaved ? ' ' + plural(unsaved, 'song isn\'t', 'songs aren\'t') + ' saved yet, so ' + (unsaved === 1 ? 'it is' : 'they are') + ' left out.' : ''}</span></span></label>
          <label class="lib-choice__opt"><input type="radio" name="lib-exp" value="urls"><span><b>Stream links</b><span class="fg-2">For any player that can reach lull. Includes every song; unsaved ones stream from Soulseek.</span></span></label>
        </fieldset>
        <div class="field"><span class="field__label">Preview of ${file}</span><pre class="lib-m3u" aria-live="polite">${preview('paths')}</pre></div>
      </div>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button><button class="btn btn--primary" type="button" data-lib-download>${icon('export')}Download ${file}</button>`,
      onMount: function (el, close) {
        var mode = function () { return $('input[name="lib-exp"]:checked', el).value; };
        el.addEventListener('change', function () { $('.lib-m3u', el).textContent = preview(mode()); });
        $('[data-lib-download]', el).addEventListener('click', function () {
          var url = URL.createObjectURL(new Blob([m3uText(p, mode())], { type: 'audio/x-mpegurl' }));
          var a = document.createElement('a'); a.href = url; a.download = file; document.body.appendChild(a); a.click(); a.remove();
          setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
          close(); App.toast('Exported ' + file + (mode() === 'paths' && unsaved ? ' without ' + plural(unsaved, 'unsaved song') : ''), { icon: 'export', kind: 'ok' });
        });
      }
    });
  }

  /* =====================================================================
     9. Add-to-playlist picker (App.hooks.addToPlaylist): search, new playlist, recent playlists
     ===================================================================== */
  function openPicker(ids) {
    ids = [].concat(ids).filter(Boolean); if (!ids.length) return;
    var first = M.track(ids[0]), what = ids.length === 1 && first ? first.title : plural(ids.length, 'song');
    var recent = playlists().slice().sort(function (a, b) { return b.updated.localeCompare(a.updated); });
    function rowMeta(p) {
      var have = ids.filter(function (id) { return plIds(p).indexOf(id) >= 0; }).length;
      if (have === ids.length) return html`<span class="lib-pick__have">${icon('check', null, 'ic-16')}already added</span>`;
      if (have) return html`<span class="lib-pick__have">${have} of ${ids.length} already in</span>`;
      return plural(p.items.length, 'song');
    }
    var inputId = App.uid('lib-pick');
    App.openSheet({
      title: 'add to playlist', className: 'lib-picker',
      body: html`<p class="lib-picker__what">Adding <b>${what}</b></p>
        <div class="search-field lib-picker__search">${icon('search')}<label class="vh" for="${inputId}">Find a playlist</label>
          <input class="input" id="${inputId}" type="search" placeholder="Find a playlist" autocomplete="off"></div>
        <button class="lib-pick lib-pick--new" type="button" data-pick-new><span class="lib-pick__plus">${icon('plus')}</span><span class="lib-pick__text"><b>New playlist</b><span>with ${what}</span></span></button>
        <form class="lib-pick-new" hidden><label class="vh" for="${inputId}-new">New playlist name</label>
          <input class="input" id="${inputId}-new" maxlength="60" placeholder="Name it" autocomplete="off"><button class="btn btn--primary" type="submit">Create</button></form>
        ${recent.length ? ui.labelRule('recent', { level: 'h3', sm: true }) : ''}
        <ul class="lib-pick-list">${recent.map(function (p) {
          return html`<li data-name="${p.name.toLowerCase()}"><button class="lib-pick" type="button" data-pick="${p.id}">${ui.mosaic(plIds(p), 44)}<span class="lib-pick__text"><b>${p.name}</b><span>${rowMeta(p)}</span></span></button></li>`;
        })}</ul>
        <p class="lib-pick-none" hidden>No playlist matches that. Press Enter to make a new one with that name.</p>`,
      onMount: function (el, close) {
        var input = $('#' + inputId, el), form = $('.lib-pick-new', el), none = $('.lib-pick-none', el);
        setTimeout(function () { input.focus(); }, 40);
        function pick(id) {
          var p = playlist(id), missing = ids.filter(function (x) { return plIds(p).indexOf(x) < 0; });
          close();
          if (!missing.length) { App.toast('Already in ' + p.name, { icon: 'check' }); return; }
          App.addToPlaylist(p.id, missing);       // shell toast with Undo
        }
        function create(name) {
          name = name.trim() || 'new playlist';
          close();
          var p = createPlaylist(name, '', ids);
          App.toast('Created ' + name + ' with ' + what, { icon: 'playlists', action: 'Open', onAction: function () { App.go('/playlist/' + p.id); } });
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
     10. M3U import (App.hooks.importFile + the Import buttons): parse, match, preview, import
     ===================================================================== */
  var SAMPLE_NAME = 'road-trip.m3u8';
  /* a sample file built from the mock catalogue plus two songs nobody has, so all three outcomes show */
  function sampleM3U() {
    var ids = ['t-lantern-season-2', 't-satellite-porch-3', 't-glasshouse-bloom-1', 't-snow-antenna-2', 't-marble-soda-club-3',
      't-night-bus-2', 't-quiet-weather-2', 't-satellite-picnic-1'];
    var out = ['#EXTM3U'];
    ids.forEach(function (id, i) {
      var t = M.track(id);
      out.push('#EXTINF:' + t.duration + ',' + t.artist + ' - ' + t.title);
      out.push('Music/' + t.artist + '/' + t.album + '/' + (t.no < 10 ? '0' : '') + t.no + ' - ' + t.title + '.flac');
      if (i === 3) { out.push('#EXTINF:212,Paper Kite Choir - Hollow Bell Morning'); out.push('Music/Paper Kite Choir/Bells/03 - Hollow Bell Morning.mp3'); }
    });
    out.push('Music/The Small Hours Club/Night Trams/07 - Tram Nine.mp3');
    return out.join('\n');
  }
  function norm(s) { return String(s || '').toLowerCase().replace(/[^a-z0-9]+/g, ' ').trim(); }
  function parseM3U(text) {
    var out = [], info = null;
    String(text).split(/\r?\n/).forEach(function (line) {
      line = line.trim();
      if (!line || line === '#EXTM3U') return;
      if (line.indexOf('#EXTINF') === 0) { var m = /^#EXTINF:(-?\d+)[^,]*,(.*)$/.exec(line); info = m ? { dur: +m[1], label: m[2] } : null; return; }
      if (line.charAt(0) === '#') return;
      var parts = line.split(/[\\/]/), file = parts[parts.length - 1].replace(/\.[^.]+$/, '').replace(/^\d+\s*[-.]\s*/, '');
      var artist = '', title = file, dash = info ? info.label.indexOf(' - ') : -1;
      if (dash > 0) { artist = info.label.slice(0, dash); title = info.label.slice(dash + 3); }
      else if (parts.length >= 3) artist = parts[parts.length - 3];
      out.push({ path: line, artist: artist.trim(), title: title.trim() });
      info = null;
    });
    return out;
  }
  function matchEntry(e) {
    var t = M.tracks.filter(function (x) { return norm(x.title) === norm(e.title) && (!e.artist || norm(x.artist) === norm(e.artist)); })[0];
    return { entry: e, track: t || null, status: !t ? 'none' : isSaved(t.id) ? 'library' : 'soulseek' };
  }

  function openImportStart() {
    App.openDialog({
      title: 'import a playlist',
      body: html`<div class="lib-form">${dropZone()}
        <p class="lib-help">Each line is matched to your library first, then searched on Soulseek. Nothing downloads until you play or save it.</p></div>`
    });
  }
  function pickFile() {
    var input = document.createElement('input');
    input.type = 'file'; input.accept = '.m3u,.m3u8,audio/x-mpegurl'; input.hidden = true;
    input.addEventListener('change', function () { if (input.files[0]) App.hooks.importFile(input.files[0]); input.remove(); });
    document.body.appendChild(input); input.click();
  }
  var IMPORT_LABEL = { library: 'In your library', soulseek: 'Found on Soulseek', none: 'Not found' };
  function importRow(r, i) {
    var t = r.track, status;
    if (r.status === 'library') status = ui.savedBadge();
    else if (r.status === 'soulseek') status = html`${ui.soulseekBadge()}<span class="lib-imp__src">${plural(t.sources.length, 'source')}</span>`;
    else status = html`<span class="lib-imp__none">${icon('no-source', null, 'ic-16')}not found</span><a class="btn btn--quiet btn--sm" href="#/search?q=${encodeURIComponent((r.entry.artist + ' ' + r.entry.title).trim())}" data-close>Search</a>`;
    return html`<li class="lib-imp__row" data-status="${r.status}">
      <span class="lib-imp__n">${i + 1}</span>
      ${t ? ui.cover(t, 40) : html`<span class="lib-imp__blank" aria-hidden="true">${icon('no-source')}</span>`}
      <span class="lib-imp__main"><b>${t ? t.title : r.entry.title}</b><span>${t ? t.artist : r.entry.artist || 'unknown artist'}</span><small>${r.entry.path}</small></span>
      <span class="lib-imp__status"><span class="vh">${IMPORT_LABEL[r.status]}: </span>${status}</span>
    </li>`;
  }
  function openImportPreview(fileName, text) {
    var rows = parseM3U(text).map(matchEntry);
    if (!rows.length) { App.toast(fileName + ' has no songs in it', { kind: 'error', icon: 'warning' }); return; }
    var count = { library: 0, soulseek: 0, none: 0 }; rows.forEach(function (r) { count[r.status]++; });
    var usable = rows.filter(function (r) { return r.track; });
    var slskIds = rows.filter(function (r) { return r.status === 'soulseek'; }).map(function (r) { return r.track.id; });
    var mb = Math.round(slskIds.reduce(function (s, id) { return s + M.track(id).sizeMB; }, 0));
    var nameId = App.uid('lib-imp');
    App.openDialog({
      title: 'import ' + fileName, wide: true, className: 'lib-import',
      body: html`<div class="lib-form">
        <div class="field"><label class="field__label" for="${nameId}">Playlist name</label>
          <input class="input" id="${nameId}" maxlength="60" value="${fileName.replace(/\.m3u8?$/i, '')}" autocomplete="off"></div>
        <div class="lib-imp__sum" role="group" aria-label="Show lines">
          <button class="pill-filter" data-imp-filter="all" aria-pressed="true">All<span class="lib-count">${rows.length}</span></button>
          <button class="pill-filter" data-imp-filter="library" aria-pressed="false">${icon('saved')}In library<span class="lib-count">${count.library}</span></button>
          <button class="pill-filter" data-imp-filter="soulseek" aria-pressed="false">${icon('soulseek')}On Soulseek<span class="lib-count">${count.soulseek}</span></button>
          <button class="pill-filter" data-imp-filter="none" aria-pressed="false">${icon('no-source')}Not found<span class="lib-count">${count.none}</span></button>
        </div>
        <ol class="lib-imp__list" aria-label="Lines in ${fileName}">${rows.map(importRow)}</ol>
        ${slskIds.length ? html`<label class="switch-row lib-imp__opt"><span><span class="switch-row__label">Save the Soulseek matches too</span>
          <span class="switch-row__help">${plural(slskIds.length, 'song')}, about ${mb} MB. Otherwise they stream when played.</span></span>
          <button class="switch" type="button" role="switch" aria-checked="false" data-imp-save></button></label>` : ''}
        ${!usable.length ? html`<p class="field__error">${icon('warning', null, 'ic-16')}Nothing in this file matched. Try searching for the songs instead.</p>` : ''}
      </div>`,
      foot: html`<button class="btn" type="button" data-close>Cancel</button>
        <button class="btn btn--primary" type="button" data-imp-go ${usable.length ? '' : raw('disabled')}>${icon('import')}Import ${plural(usable.length, 'song')}</button>`,
      onMount: function (el, close) {
        el.addEventListener('click', function (e) {
          var f = e.target.closest('[data-imp-filter]');
          if (f) {
            var v = f.getAttribute('data-imp-filter');
            $$('[data-imp-filter]', el).forEach(function (b) { b.setAttribute('aria-pressed', String(b === f)); });
            $$('.lib-imp__row', el).forEach(function (r) { r.hidden = v !== 'all' && r.getAttribute('data-status') !== v; });
            return;
          }
          var sw = e.target.closest('.lib-imp__opt');
          if (sw) { e.preventDefault(); var s = $('[data-imp-save]', el); s.setAttribute('aria-checked', String(s.getAttribute('aria-checked') !== 'true')); return; }
          if (!e.target.closest('[data-imp-go]')) return;
          var name = $('#' + nameId, el).value.trim() || fileName.replace(/\.m3u8?$/i, '');
          var saveToo = $('[data-imp-save]', el) && $('[data-imp-save]', el).getAttribute('aria-checked') === 'true';
          close();
          var p = createPlaylist(name, 'Imported from ' + fileName + '.', usable.map(function (r) { return r.track.id; }));
          App.store.update('activity', function (a) {
            a.unshift({ id: App.uid('job-import'), kind: 'import', name: fileName, matched: count.library, search: count.soulseek, notFound: count.none, state: 'done', started: '21:45' });
          });
          if (saveToo) App.saves.start(slskIds);
          App.go('/playlist/' + p.id);
          App.toast('Imported ' + plural(usable.length, 'song') + (count.none ? '. ' + count.none + ' weren\'t found.' : ''), { kind: 'ok', icon: 'import' });
        });
      }
    });
  }
  App.hooks.importFile = function (file) {
    if (!/\.m3u8?$/i.test(file.name)) { App.toast('Can\'t import ' + file.name + ': only .m3u and .m3u8 playlists', { kind: 'error', icon: 'warning' }); return; }
    var reader = new FileReader();
    reader.onload = function () { openImportPreview(file.name, String(reader.result)); };
    reader.onerror = function () { App.toast('Couldn\'t read ' + file.name, { kind: 'error', icon: 'warning' }); };
    reader.readAsText(file);
  };

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
  App.action('lib-like-all', function (el) {
    var ids = idsOf(el), on = !ids.every(isLiked), before = Object.assign({}, App.store.get('liked'));
    App.store.update('liked', function (l) { ids.forEach(function (id) { if (on) l[id] = true; else delete l[id]; }); });
    App.syncRows();
    App.toast(on ? 'Liked ' + plural(ids.length, 'song') : 'Removed ' + plural(ids.length, 'like'), { icon: on ? 'heart-fill' : 'heart', action: 'Undo',
      onAction: function () { App.store.set('liked', before); App.syncRows(); } });
  });
  App.action('lib-jellyfin-retry', function () {
    App.setService('jellyfin', { state: 'ok', detail: 'Library online, 1,204 songs' });
    App.toast('Jellyfin is back. Saved songs can play again.', { kind: 'ok', icon: 'status-ok' });
  });
  App.action('lib-album-filter', function (el) {
    prefs.albumFilter = el.getAttribute('data-value') || el.getAttribute('data-id') || 'all'; App.storage('libAlbumFilter', prefs.albumFilter); refreshKeepFocus();
  });
  function sortMenu(el, table, key, store) {
    App.menu(el, Object.keys(table).map(function (k) {
      return { label: table[k][0], icon: prefs[key] === k ? 'check' : 'dash', run: function () { prefs[key] = k; App.storage(store, k); App.refresh(); } };
    }), { label: 'Sort by' });
  }
  App.action('lib-album-sort', function (el) { sortMenu(el, ALBUM_SORTS, 'albumSort', 'libAlbumSort'); });
  App.action('lib-song-sort', function (el) { sortMenu(el, SONG_SORTS, 'songSort', 'libSongSort'); });
  App.action('lib-album-menu', function (el) {
    var a = M.album(el.getAttribute('data-id'));
    App.menu(el, [
      { label: 'Play next', icon: 'queue', run: function () { App.player.playNext(a.trackIds); App.toast(a.title + ' plays next', { icon: 'queue' }); } },
      { label: 'Add to queue', icon: 'plus', run: function () { App.player.addToQueue(a.trackIds); App.toast('Added ' + a.title + ' to the queue', { icon: 'queue' }); } },
      { label: 'Go to artist', icon: 'peer', run: function () { App.go('/artist/' + a.artistId); } },
      { label: 'Show sources for track 1', icon: 'soulseek', run: function () { App.hooks.sourcePicker(a.trackIds[0]); } },
      'sep',
      { label: 'Copy link', icon: 'link', run: function () { copyLink('/album/' + a.id); } }
    ], { label: 'Actions for ' + a.title });
  });
  App.action('lib-artist-menu', function (el) {
    var ar = M.artist(el.getAttribute('data-id'));
    App.menu(el, [
      { label: 'Add top songs to queue', icon: 'queue', run: function () { App.player.addToQueue(M.tracksBy(ar.id).map(function (t) { return t.id; })); App.toast('Added ' + ar.name + ' to the queue', { icon: 'queue' }); } },
      { label: 'Hide from my feed', icon: 'close', run: function () {
        App.store.update('hiddenArtists', function (h) { h[ar.id] = true; });
        App.toast('Hidden ' + ar.name + ' from your feed', { icon: 'info', action: 'Undo', onAction: function () { App.store.update('hiddenArtists', function (h) { delete h[ar.id]; }); } });
      } },
      'sep',
      { label: 'Copy link', icon: 'link', run: function () { copyLink('/artist/' + ar.id); } }
    ], { label: 'Actions for ' + ar.name });
  });
  App.action('lib-import', function () { openImportStart(); });
  App.action('lib-import-pick', function () { pickFile(); });
  App.action('lib-import-sample', function () { openImportPreview(SAMPLE_NAME, sampleM3U()); });
  App.action('lib-pl-new', function () { openEdit(null); });
  App.action('lib-pl-edit', function (el) { openEdit(playlist(el.getAttribute('data-id'))); });
  App.action('lib-pl-export', function (el) {
    var p = playlist(el.getAttribute('data-id'));
    if (!p.items.length) { App.toast('Add a few songs before exporting', { icon: 'info' }); return; }
    openExport(p);
  });
  App.action('lib-pl-sort', function (el) { plSort = el.getAttribute('data-value'); App.refresh(); App.announce('Sorted by ' + SORTS[plSort].label.toLowerCase()); });
  App.action('lib-pl-sortmenu', function (el) {
    App.menu(el, Object.keys(SORTS).map(function (k) {
      return { label: SORTS[k].label, icon: plSort === k ? 'check' : 'dash', run: function () { plSort = k; App.refresh(); } };
    }), { label: 'Sort songs' });
  });
  App.action('lib-pl-menu', function (el) {
    var p = playlist(el.getAttribute('data-id')), ids = plIds(p);
    var items = [
      { label: 'Rename', icon: 'edit', run: function () { openEdit(p); } },
      { label: 'Play next', icon: 'queue', run: function () { if (!ids.length) return App.toast('This playlist is empty', { icon: 'info' }); App.player.playNext(ids); App.toast(p.name + ' plays next', { icon: 'queue' }); } },
      { label: 'Add to queue', icon: 'plus', run: function () { if (!ids.length) return App.toast('This playlist is empty', { icon: 'info' }); App.player.addToQueue(ids); App.toast('Added ' + p.name + ' to the queue', { icon: 'queue' }); } },
      { label: 'Duplicate', icon: 'playlists', run: function () { var c = createPlaylist(p.name + ' (copy)', p.desc, ids); App.go('/playlist/' + c.id); App.toast('Made a copy of ' + p.name, { icon: 'playlists' }); } },
      { label: 'Export as M3U', icon: 'export', run: function () { if (ids.length) openExport(p); else App.toast('Add a few songs before exporting', { icon: 'info' }); } },
      { label: 'Copy link', icon: 'link', run: function () { copyLink('/playlist/' + p.id); } },
      'sep',
      { label: 'Delete playlist', icon: 'trash', danger: true, run: function () { deletePlaylist(p); } }
    ];
    App.menu(el, items, { label: 'Actions for ' + p.name });
  });
  App.action('lib-pl-row-menu', function (el) {
    var row = el.closest('.trow'), list = row.parentElement, p = playlist(list.getAttribute('data-pl'));
    var i = $$('.trow', list).indexOf(row), id = row.getAttribute('data-track-id'), t = M.track(id), custom = plSort === 'custom';
    App.menu(el, [
      { label: 'Play next', icon: 'queue', run: function () { App.player.playNext([id]); } },
      { label: 'Add to queue', icon: 'plus', run: function () { App.player.addToQueue([id]); } },
      { label: isSaved(id) ? 'In your library' : 'Save to library', icon: isSaved(id) ? 'saved' : 'save', run: function () { if (isSaved(id)) App.toast('Already saved to your library', { icon: 'saved' }); else App.saves.start([id]); } },
      { label: 'Add to another playlist', icon: 'playlists', run: function () { openPicker([id]); } },
      'sep',
      { label: 'Move up', icon: 'chevron-up', run: function () { if (!custom) return sortNeeded(); if (i > 0) { moveItems(p, [i], i - 1); focusRow(i - 1); } } },
      { label: 'Move down', icon: 'chevron-down', run: function () { if (!custom) return sortNeeded(); if (i < p.items.length - 1) { moveItems(p, [i], i + 2); focusRow(i + 1); } } },
      { label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } },
      { label: 'Remove from this playlist', icon: 'trash', danger: true, run: function () { if (!custom) return sortNeeded(); removeItem(p, i); } }
    ], { label: 'Actions for ' + t.title });
  });

  /* =====================================================================
     12. Routes and states
     ===================================================================== */
  /* title-bar meta lines (route.meta -> .main[data-meta]; the same text as data-meta on the page card) */
  function albumMeta(a) { return plural(a.trackIds.length, 'song') + ', ' + fmt.length(a.duration) + ', ' + fmt.size(a.sizeMB); }
  function playlistMeta(p) { var ids = plIds(p); return plural(ids.length, 'song') + ', ' + fmt.length(sumDuration(ids)); }
  function metaOf(fn, line) { return function (ctx) { var x = fn(ctx.params.id); return x ? line(x) : ''; }; }
  function titleOf(fn) { return function (ctx) { var x = fn(ctx.params.id); return x ? (x.title || x.name) : 'Not found'; }; }
  App.route('/library/:segment?', { render: renderLibrary, mount: mountLibrary, nav: 'library',
    title: function (ctx) { var s = SEGMENTS.filter(function (x) { return x.id === ctx.params.segment; })[0]; return 'Library' + (s ? ': ' + s.label : ''); } });
  App.route('/liked', { render: function (ctx) { return libraryFrame('liked', likedBody(ctx), ctx); }, mount: mountLibrary, nav: 'library', title: 'Liked songs' });
  App.route('/history', { render: function (ctx) { return libraryFrame('history', historyBody(ctx), ctx); }, mount: mountLibrary, nav: 'library', title: 'History' });
  App.route('/following', { render: function (ctx) { return libraryFrame('following', followingBody(ctx), ctx); }, mount: mountLibrary, nav: 'library', title: 'Following' });
  App.route('/album/:id', { render: renderAlbum, mount: mountAlbum, nav: 'library', title: titleOf(M.album), meta: metaOf(M.album, albumMeta) });
  App.route('/artist/:id', { render: renderArtist, mount: mountArtist, nav: 'library', title: titleOf(M.artist),
    meta: metaOf(M.artist, function (a) { return plural(M.albumsBy(a.id).length, 'release') + ', ' + plural(M.tracksBy(a.id).length, 'song'); }) });
  App.route('/tag/:id', { render: renderTag, mount: mountTag, nav: 'library', title: function (ctx) { return ctx.params.id; } });
  App.route('/playlists', { render: renderPlaylists, mount: mountPlaylists, nav: 'playlists', title: 'Playlists' });
  App.route('/playlist/:id', { render: renderPlaylist, mount: mountPlaylist, nav: 'playlists', title: titleOf(playlist), meta: metaOf(playlist, playlistMeta) });

  /* state helpers */
  function jellyfinOk() { if (jellyfinDown()) App.setService('jellyfin', { state: 'ok', detail: 'Library online, 1,204 songs' }); }
  function resetSaves(ids) { ids.forEach(function (id) { if (App.saves.get(id)) App.saves.set(id, null); }); }
  function demoPlaylist() {
    var p = playlist('pl-new-demo');
    if (!p) { p = createPlaylist('late train home', ''); p.id = 'pl-new-demo'; App.store.set('playlists', playlists()); }
    p.items = []; return p;
  }
  function custom() { plSortFor = null; pendingSort = null; }
  var snow = M.album('al-snow-antenna').trackIds;

  App.registerStates('library', [
    { id: 'library-albums', label: 'Library: albums grid', route: '/library/albums', apply: jellyfinOk },
    { id: 'library-artists', label: 'Library: artists', route: '/library/artists', apply: jellyfinOk },
    { id: 'library-songs', label: 'Library: saved songs', route: '/library/songs', apply: jellyfinOk },
    { id: 'library-genres', label: 'Library: genres and tags', route: '/library/genres', apply: jellyfinOk },
    { id: 'library-empty', label: 'Library: nothing saved yet', route: '/library/albums', apply: jellyfinOk },
    { id: 'library-jellyfin-down', label: 'Library: Jellyfin down (read-only cache)', route: '/library/songs',
      apply: function () { App.setService('jellyfin', { state: 'down', detail: 'Can\'t reach Jellyfin: connection refused' }); } },
    { id: 'library-album-saved', label: 'Album: saved, two discs', route: '/album/al-lantern-season', apply: jellyfinOk },
    { id: 'library-album-partial', label: 'Album: partly saved', route: '/album/al-harbour-lights', apply: jellyfinOk },
    { id: 'library-album-soulseek', label: 'Album: not saved (Soulseek)', route: '/album/al-snow-antenna', apply: function () { jellyfinOk(); resetSaves(snow); } },
    { id: 'library-album-saving', label: 'Album: saving', route: '/album/al-snow-antenna',
      apply: function () { snow.forEach(function (id, i) { App.saves.set(id, { step: 2 + (i % 3), status: 'saving', pct: 0, frozen: true }); }); } },
    { id: 'library-album-failed', label: 'Album: a save step failed', route: '/album/al-snow-antenna',
      apply: function () { resetSaves(snow); App.saves.set(snow[1], { step: 2, status: 'failed', error: 'No MusicBrainz match', frozen: true }); } },
    { id: 'library-artist', label: 'Artist page (followed)', route: '/artist/ar-glass-harbour' },
    { id: 'library-artist-unfollowed', label: 'Artist page (not followed)', route: '/artist/ar-moonpail' },
    { id: 'library-tag', label: 'Genre / tag page', route: '/tag/dream-gaze' },
    { id: 'library-liked', label: 'Liked songs', route: '/liked' },
    { id: 'library-liked-empty', label: 'Liked songs: empty', route: '/liked' },
    { id: 'library-history', label: 'History by day, scrobble status', route: '/history' },
    { id: 'library-history-empty', label: 'History: empty', route: '/history' },
    { id: 'library-following', label: 'Following: artists and tags', route: '/following' },
    { id: 'library-following-empty', label: 'Following: empty', route: '/following' },
    { id: 'library-playlists', label: 'Playlists: list', route: '/playlists' },
    { id: 'library-playlists-empty', label: 'Playlists: none yet', route: '/playlists' },
    { id: 'library-playlist', label: 'Playlist page', route: '/playlist/pl-sleepy-sunday', apply: custom },
    { id: 'library-playlist-sorted', label: 'Playlist: sorted by date added', route: '/playlist/pl-strawberry-static', apply: function () { plSortFor = null; pendingSort = 'added'; } },
    { id: 'library-playlist-reorder', label: 'Playlist: dragging a song', route: '/playlist/pl-3am-window', apply: custom },
    { id: 'library-playlist-empty', label: 'Playlist: empty', route: '/playlist/pl-new-demo', apply: demoPlaylist },
    { id: 'library-playlist-drop', label: 'Playlist: drop target', route: '/playlist/pl-new-demo', apply: demoPlaylist },
    { id: 'library-playlist-create', label: 'Playlist: create dialog', route: '/playlists' },
    { id: 'library-playlist-edit', label: 'Playlist: rename / edit', route: '/playlist/pl-pocket-folk', apply: custom },
    { id: 'library-playlist-export', label: 'Playlist: export M3U', route: '/playlist/pl-sleepy-sunday', apply: custom },
    { id: 'library-playlist-deleted', label: 'Playlist: deleted, with undo', route: '/playlists',
      apply: function () { var p = playlist('pl-pocket-folk'); if (p) deletePlaylist(p); } },
    { id: 'library-playlist-picker', label: 'Add to playlist picker', route: '/album/al-glasshouse-bloom',
      apply: function () { setTimeout(function () { openPicker(['t-glasshouse-bloom-2']); }, 60); } },
    { id: 'library-m3u-start', label: 'M3U import: choose a file', route: '/playlists' },
    { id: 'library-m3u-preview', label: 'M3U import: preview matches', route: '/playlists' },
    { id: 'library-m3u-error', label: 'M3U import: nothing matched', route: '/playlists' }
  ]);
})();
