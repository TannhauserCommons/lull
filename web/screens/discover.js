/* screens/discover.js: Home (discovery feed), Search, Soulseek folder sheet, source picker,
   source details and the save flow (duplicate dialog, step failures, release picker, aggregate saves).
   Owner: discover. Takes over the hooks beforeSave, sourcePicker, sourceDetails and reason.
   Server: GET /api/feed, GET /api/search + `search.results` events, GET /api/tracks/{id}/sources,
   POST /api/library/duplicates, /api/saves/{jobId}/releases|release (docs/context/api.md).
   Also exposes App.discover.{ openFolder, openPicker, openDetails, openSaveSheet, openAggregate,
   openReleasePicker, openDuplicate, startRadio } for other screens. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$;
  var P = App.player, size = App.fmt.size;
  var FEED = '/api/feed';
  var SOURCES_POLL_MS = 2000;       // how often an unfinished source list (`done` false) is asked again

  /* =====================================================================
     Small helpers
     ===================================================================== */
  /* format facts by label; a label no track has carried yet is read from the label itself ("FLAC 16/44.1", "MP3 320") */
  function fmt(label) {
    label = label || '';
    if (M.formats[label]) return M.formats[label];
    var parts = label.split(' '), spec = parts[1] || '', lossless = /^(FLAC|ALAC|WAV|AIFF)$/i.test(parts[0]), bk = spec.split('/');
    return { label: label, codec: parts[0], lossless: lossless, bits: lossless ? +bk[0] || null : null,
      khz: lossless ? +bk[1] || null : null, kbps: lossless ? null : parseInt(spec, 10) || null };
  }
  function detail(f) {
    if (f.lossless) return [f.bits ? f.bits + '-bit' : '', f.khz ? f.khz + ' kHz' : ''].filter(Boolean).join(', ');
    return f.kbps ? f.kbps + ' kbps' : '';
  }
  /* one comparable number per format (the picker's quality sort): any lossless file outranks any lossy one */
  function quality(label) { var f = fmt(label); return f.lossless ? 1e4 + (f.bits || 16) * (f.khz || 44.1) : f.kbps || 0; }
  function mbps(v) { return (Math.round((v || 0) * 10) / 10) + ' MB/s'; }
  function plural(n, one, many) { return n + ' ' + (n === 1 ? one : (many || one + 's')); }
  function isPhone() { return window.innerWidth < 640; }
  function thousands(n) { return n >= 1e6 ? (Math.round(n / 1e5) / 10) + 'M' : n >= 1000 ? (Math.round(n / 100) / 10) + 'k' : String(n); }
  function unique(list) { return list.filter(function (x, i) { return list.indexOf(x) === i; }); }
  /* unix seconds -> "just now", "6 min ago", "3 h ago", "Oct 29" */
  function ago(at) {
    var s = Date.now() / 1000 - at;
    if (s < 60) return 'just now';
    if (s < 3600) return Math.floor(s / 60) + ' min ago';
    if (s < 86400) return Math.floor(s / 3600) + ' h ago';
    return App.fmt.date(at);
  }
  /* followed tags live in the store next to followed artists; a tag's id is its name */
  function followedTagIds() { return Object.keys(App.store.get('followed') || {}).filter(function (id) { return !/^ar-/.test(id); }); }

  /* re-render one [data-region] in place; keeps focus on the same control when it still exists */
  function paint(name, markup, root) {
    var el = $('[data-region="' + name + '"]', root || document); if (!el) return;
    var a = document.activeElement, had = a && el.contains(a);
    var key = had ? (a.getAttribute('data-act') || '') + '|' + (a.getAttribute('data-id') || a.getAttribute('data-q') || '') : null;
    App.patch(el, markup);
    App.syncRows();
    if (had) {
      var back = $$('[data-act]', el).filter(function (b) { return (b.getAttribute('data-act') || '') + '|' + (b.getAttribute('data-id') || b.getAttribute('data-q') || '') === key; })[0];
      (back || el.querySelector('button, a[href]') || $('#view')).focus({ preventScroll: true });
    }
  }

  /* radio from anything: start it, then show where it lands (queue panel; Now Playing > Queue on phone) */
  function startRadio(seed) {
    return P.startRadio(seed).then(function (ok) {
      if (ok) { if (isPhone()) App.go('/now?tab=queue'); else App.openPanel('queue'); }
      return ok;
    });
  }
  App.action('disc-radio', function (el) {
    App.closePopover();
    startRadio({ kind: el.getAttribute('data-kind') || 'track', id: el.getAttribute('data-id') });
  });

  /* ---------- a track's Soulseek sources: ranked by the server, asked again until `done` ---------- */
  /* the copies in the order the streaming profile takes them, or with forSave the saving one (ADR-0008) */
  function sourcesPath(id, forSave) { return '/api/tracks/' + encodeURIComponent(id) + '/sources' + (forSave ? '?for=save' : ''); }
  /* onUpdate({ sources, done }) after each answer, or onUpdate({ error }); returns stop() */
  function watchSources(id, onUpdate, forSave) {
    var timer = null, stopped = false;
    (function ask() {
      M.api(sourcesPath(id, forSave)).then(function (res) {
        if (stopped) return;
        onUpdate({ sources: res.sources || [], done: !!res.done });
        if (!res.done) timer = setTimeout(ask, SOURCES_POLL_MS);
      }, function (err) { if (!stopped) onUpdate({ error: err }); });
    })();
    return function stop() { stopped = true; clearTimeout(timer); };
  }
  /* peers that failed this track before: the server's stalledBefore (unix time), plus the player's own fallbacks */
  function failedNotes(trackId, sources) {
    var notes = {};
    sources.forEach(function (s) { if (s.stalledBefore) notes[s.peer] = typeof s.stalledBefore === 'number' ? 'stalled ' + ago(s.stalledBefore) : 'stalled before'; });
    if (P.currentId() === trackId) P.state.failedPeers.forEach(function (f) { notes[f.peer] = f.at != null ? 'stalled at ' + f.at + '%' : 'stalled just now'; });
    return notes;
  }
  /* the part of a source row the player and the save pipeline take */
  function sourceFor(s) { return { peer: s.peer, path: s.path, format: s.format, sizeBytes: s.sizeBytes, speed: s.speed }; }

  /* =====================================================================
     1. Home / discovery feed (GET /api/feed)
     ===================================================================== */
  /* view state: picks dismissed this session, cold-start choices, "search again" on picks with no source,
     the cold-start follows in flight, and the shelves showing skeletons while the feed reloads */
  var H = { dismissed: {}, coldPicks: {}, finding: null, found: {}, building: false, reloading: null, expanded: {} };
  function list(feed, key) { return (feed && feed[key]) || []; }
  function greeting() {
    var h = new Date().getHours();
    return h >= 5 && h < 12 ? 'good morning' : h >= 12 && h < 18 ? 'good afternoon' : 'good evening';
  }
  function hello(sum) {
    return html`<header class="disc-hello">
        <h1 class="t-display">${greeting()}${ui.sparkle('--s:18px')}</h1>
        <p class="disc-hello__sum">${sum || ''}</p>
      </header>`;
  }

  function renderHome() {
    var r = M.get(FEED), feed = H.building ? null : r.data;
    /* a reload has answered (new data or a new error): its shelves stop showing skeletons */
    if (H.reloading && (r.data !== H.reloading.data || r.error !== H.reloading.error)) H.reloading = null;
    if (r.error && !r.data) {
      return html`<div class="disc-home">${hello('')}${App.errorCard(r.error, { path: FEED, title: 'Couldn\'t load your feed' })}</div>`;
    }
    if (feed && isCold(feed)) return renderCold(feed);
    return html`<div class="disc-home">
      ${hello(feed ? feed.summary : 'Gathering new releases and picks for you.')}
      ${feed && r.error ? App.errorCard(r.error, { path: FEED, title: 'Couldn\'t refresh your feed' }) : ''}
      ${feed ? homeLead() : ''}
      <div class="disc-modules">${MODULE_IDS.map(function (id) { return homeModule(id, feed); })}</div>
    </div>`;
  }

  /* ---------- Home modules (ADR-0007): a grid of one, two or three columns as the main area grows; each module is
     the `section` container, so its contents reveal more as it widens. ---------- */
  var MODULES = {
    new: { body: newReleasesBody, meta: function (feed) { return feed ? plural(list(feed, 'newReleases').length, 'release') : ''; } },
    radio: { body: radioBody },
    picks: { body: picksBody, meta: function (feed) { return feed && feed.updated ? 'updated ' + ago(feed.updated) : ''; } },
    jump: { body: jumpBody },
    tags: { body: tagsBody }
  };
  var MODULE_IDS = ['picks', 'new', 'radio', 'jump', 'tags'];        // reading order: "Picked for you" comes first
  function homeModule(id, feed) {
    var m = MODULES[id];
    var card = html`<section class="card disc-card" aria-labelledby="disc-h-${id}" data-region="${id}" data-meta="${m.meta ? m.meta(feed) : ''}">${m.body(feed)}</section>`;
    return html`<div class="disc-mod" data-module="${id}">${card}</div>`;
  }

  /* homeLead slot: a design's own first object on Home (a media window, a widget), between the greeting and the
     shelves. Default: nothing. ctx = { track, player, queue: next tracks (up to 8), parts: { cover(t, size),
     playButton(size), likeButton(t), trackList(tracks, opts), formatLine(t) } }. Rendered as .disc-lead[data-region="lead"]. */
  function homeLead() {
    var t = P.current();
    var out = String(App.slot('homeLead', { track: t, player: P.state, queue: P.upNext(8), parts: {
      cover: function (x, size) { return ui.cover(x, size || 64); },
      playButton: function (size) { return gloss(size); },
      likeButton: function (x) { return ui.likeButton((x || t || {}).id || ''); },
      trackList: function (tracks, opts) { return ui.trackList(tracks, Object.assign({ dense: true }, opts || {})); },
      formatLine: App.fmt.formatLine } }, function () { return ''; }));
    return out ? html`<div class="disc-lead" data-region="lead">${raw(out)}</div>` : '';
  }
  function gloss(size) {             // sizes with a .btn-gloss--N class: 40 48 52 56 64
    var on = P.state.playing;
    return html`<button class="btn-gloss btn-gloss--${size || 56}" data-act="play" data-bind="play" aria-label="${on ? 'Pause' : 'Play'}" data-tip="${on ? 'Pause (Space)' : 'Play (Space)'}"><i class="btn-gloss__spin" aria-hidden="true"></i>${icon(on ? 'pause' : 'play', null, on ? '' : 'ic-play')}</button>`;
  }

  /* shelves: key in the feed (and in its `errors`) -> region and body, so one shelf can be repainted alone */
  var SHELVES = {
    newReleases: ['new', newReleasesBody], radioStarters: ['radio', radioBody], picks: ['picks', picksBody],
    jumpBackIn: ['jump', jumpBody], followedTags: ['tags', tagsBody]
  };
  function paintShelf(key) {
    var s = SHELVES[key], feed = M.peek(FEED);
    if (s && feed) paint(s[0], s[1](feed));
  }
  /* a shelf still loading shows its skeleton. The shelves the server may fail to fill alone (newReleases, picks:
     the keys of `errors`) pass errorTitle and show the server's message with a retry. */
  function shelfIssue(feed, key, skeleton, errorTitle) {
    if (!feed || (H.reloading && H.reloading.shelves.indexOf(key) >= 0)) return skeleton;
    var msg = errorTitle && feed.errors && feed.errors[key];
    return msg ? ui.emptyState({ art: 'star', title: errorTitle, text: msg, action: { label: 'Retry', act: 'disc-retry-shelf', id: key } }) : null;
  }

  /* a section with nothing to show is one line: its name, why, and maybe one way to fill it */
  function emptyLine(region, label, note, action) {
    return html`<div class="disc-empty"><h2 id="disc-h-${region}">${label}</h2><p>${note}</p>${action || ''}</div>`;
  }

  function newReleasesBody(feed) {
    var issue = shelfIssue(feed, 'newReleases', html`<div class="disc-skel-shelf">${ui.skeleton('tile', 4)}</div>`, 'Couldn\'t load new releases');
    var more = !!H.expanded.new;
    /* See all opens the shelf in place as a grid: the feed's releases are not library albums */
    var head = ui.labelRule('new releases', { id: 'disc-h-new', action: issue ? null : { label: more ? 'Show less' : 'See all', act: 'disc-expand-new' } });
    if (issue) return html`${head}${issue}`;
    var albums = list(feed, 'newReleases').filter(function (id) { return M.album(id); });
    if (!albums.length) return emptyLine('new', 'new releases', 'Nothing new from artists you follow yet.');
    return html`${head}<div class="shelf disc-shelf${more ? ' disc-shelf--grid' : ''}">${albums.map(function (id) { return ui.releaseTile(id); })}</div>`;
  }

  function seedArt(seed, px) {
    var t = seed.kind === 'track' ? M.track(seed.id) : null;
    return ui.cover(t ? t.albumId : seed.id, px || 48, { round: true });
  }
  function radioBody(feed) {
    var issue = shelfIssue(feed, 'radioStarters', ui.skeleton('row', 4));
    var head = ui.labelRule('radio starters', { id: 'disc-h-radio' });
    if (issue) return html`${head}${issue}`;
    var starters = list(feed, 'radioStarters');
    if (!starters.length) return emptyLine('radio', 'radio starters', 'Play a few songs and starters show up here.');
    return html`${head}<ul class="disc-starters">${starters.map(function (r) {
      /* the whole row starts the radio on click; the round button is the keyboard control */
      return html`<li class="disc-starter" data-act="disc-radio" data-kind="${r.seed.kind}" data-id="${r.seed.id}">
        ${seedArt(r.seed)}
        <div class="disc-starter__main"><span class="disc-starter__title">${r.title}</span><span class="disc-starter__sub">${r.sub}</span></div>
        <button class="btn-icon disc-starter__play" data-act="disc-radio" data-kind="${r.seed.kind}" data-id="${r.seed.id}" aria-label="Start ${r.title}" data-tip="Start radio">${icon('play')}</button>
      </li>`;
    })}</ul>`;
  }

  function pickFor(trackId) { return list(M.peek(FEED), 'picks').filter(function (p) { return p.trackId === trackId; })[0] || null; }
  /* picks still shown: not dismissed, artist not hidden, track known; picks with a source first */
  function visiblePicks(feed, all) {
    var hidden = App.store.get('hiddenArtists') || {};
    var shown = list(feed, 'picks').filter(function (p) { var t = M.track(p.trackId); return t && !H.dismissed[p.trackId] && !hidden[t.artistId]; });
    /* up to 12: the module shows as many as its width allows (discover.css); See all lifts the cap */
    var ordered = shown.filter(function (p) { return !p.noSource; }).concat(shown.filter(function (p) { return p.noSource; }));
    return all ? ordered : ordered.slice(0, 12);
  }
  function picksBody(feed) {
    var issue = shelfIssue(feed, 'picks', html`<div class="disc-picks-skel">${ui.skeleton('row', 6)}</div>`, 'Couldn\'t load your picks');
    var more = !!H.expanded.picks, picks = visiblePicks(feed, more);
    var head = html`<div class="label-rule"><h2 id="disc-h-picks">picked for you</h2>${issue ? '' : html`${list(feed, 'picks').length > 6 || more ? html`<button class="btn btn--quiet btn--sm" data-act="disc-expand-picks" aria-expanded="${more ? 'true' : 'false'}">${more ? 'Show less' : 'See all'}</button>` : ''}<button class="btn btn--quiet btn--sm" data-act="disc-refresh-picks">Refresh picks</button>`}</div>`;
    if (issue) return html`${head}${issue}`;
    if (!picks.length) {
      return emptyLine('picks', 'picked for you', (list(feed, 'picks').length ? 'You hid everything we had. ' : '') + 'Like a few songs or follow a tag and picks show up here.',
        html`<a class="btn btn--quiet btn--sm" href="#/following">Browse tags</a>`);
    }
    var playable = picks.filter(function (p) { return !p.noSource || H.found[p.trackId]; }).map(function (p) { return p.trackId; });
    return html`${head}<ul class="disc-picks${more ? ' disc-picks--all' : ''}" data-ids="${playable.join(',')}">${picks.map(pickItem)}</ul>`;
  }
  function pickItem(p) {
    var t = M.track(p.trackId), reasons = p.reasons || [];
    var noSrc = p.noSource && !H.found[t.id], finding = H.finding === t.id;
    return html`<li class="disc-pick${noSrc ? ' is-nosource' : ''}">
      <div class="disc-pick__art">${ui.cover(t, 64)}${noSrc ? '' : html`<button class="disc-pick__play" data-act="play-track" data-id="${t.id}" tabindex="-1" aria-label="Play ${t.title}" data-tip="Play">${icon('play')}</button>`}</div>
      <div class="disc-pick__main">
        ${noSrc ? html`<span class="disc-pick__title">${t.title}</span>` : html`<button class="disc-pick__title" data-act="play-track" data-id="${t.id}">${t.title}</button>`}
        <a class="disc-pick__by" href="#/artist/${t.artistId}">${t.artist}</a>
      </div>
      <div class="disc-pick__end">${ui.likeButton(t.id)}<button class="btn-icon btn-icon--bare" data-act="disc-pick-menu" data-id="${t.id}" aria-haspopup="menu" aria-label="More actions for ${t.title}" data-tip="More">${icon('more')}</button></div>
      <div class="disc-pick__why">${reasons[0] ? ui.reasonChip(reasons[0], { more: reasons.length - 1, via: p.via, itemId: t.id }) : ''}
        ${noSrc ? html`<div class="disc-pick__nosrc">
          ${finding ? html`<span class="resolve" role="status"><span class="spin-ring" aria-hidden="true"></span>Searching Soulseek</span>`
            : html`<span class="resolve">${icon('no-source')}No source found yet</span>`}
          <button class="btn btn--sm" data-act="disc-find" data-id="${t.id}" ${finding ? raw('disabled') : ''}>${icon('refresh')}Search again</button></div>` : ''}
      </div>
    </li>`;
  }

  /* j: { kind: album | artist, id } */
  function jumpTile(j) {
    var art, href, title, by, byHref, play, round = false;
    if (j.kind === 'album') {
      var a = M.album(j.id); if (!a) return '';
      art = ui.cover(a, '100%'); href = '#/album/' + a.id; title = a.title; by = a.artist; byHref = '#/artist/' + a.artistId;
      play = html`<button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button>`;
    } else {
      var ar = M.artist(j.id); if (!ar) return '';
      art = ui.cover(ar.id, '100%', { round: true }); href = '#/artist/' + ar.id; title = ar.name; by = 'artist'; round = true;
      play = html`<button class="tile__play" data-act="disc-radio" data-kind="artist" data-id="${ar.id}" aria-label="Start ${ar.name} radio" data-tip="Start radio">${icon('radio')}</button>`;
    }
    return html`<div class="tile disc-jump${round ? ' disc-jump--round' : ''}">
      <div class="tile__art"><a href="${href}" tabindex="-1" aria-hidden="true">${art}</a>${play}</div>
      <div class="tile__title"><a href="${href}">${title}</a></div>
      <div class="tile__by">${byHref ? html`<a href="${byHref}">${by}</a>` : by}</div>
    </div>`;
  }
  function jumpBody(feed) {
    var issue = shelfIssue(feed, 'jumpBackIn', html`<div class="disc-skel-shelf">${ui.skeleton('tile', 4)}</div>`);
    var head = ui.labelRule('jump back in', { id: 'disc-h-jump', action: issue ? null : { label: 'History', href: '#/history' } });
    if (issue) return html`${head}${issue}`;
    var tiles = list(feed, 'jumpBackIn').map(jumpTile).filter(function (x) { return String(x); });
    if (!tiles.length) return emptyLine('jump', 'jump back in', 'What you play shows up here.');
    return html`${head}<div class="shelf disc-shelf disc-shelf--sm">${tiles}</div>`;
  }

  function tagsBody(feed) {
    var issue = shelfIssue(feed, 'followedTags', html`<div class="cluster">${ui.skeleton('chip', 5)}</div>`);
    var head = ui.labelRule('tags you follow', { id: 'disc-h-tags', action: issue ? null : { label: 'Manage', href: '#/following' } });
    if (issue) return html`${head}${issue}`;
    var followed = followedTagIds();
    var related = unique([].concat.apply([], followed.map(function (id) { return M.tag(id).related || []; })))
      .filter(function (id) { return !App.isFollowed(id); }).slice(0, 4);
    if (!followed.length && !related.length) {
      return emptyLine('tags', 'tags you follow', 'Follow a tag and its new releases show up here.', html`<a class="btn btn--quiet btn--sm" href="#/following">Browse tags</a>`);
    }
    return html`${head}
      ${followed.length ? html`<ul class="disc-tags">${followed.map(function (id) {
        var t = M.tag(id);
        return html`<li class="disc-tag"><a class="disc-tag__name" href="#/tag/${encodeURIComponent(t.id)}">${icon('tag')}<span>${t.name}</span></a>
          <span class="disc-tag__desc">${t.desc || ''}</span>
          <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="disc-radio" data-kind="tag" data-id="${t.id}" aria-label="Start ${t.name} radio" data-tip="Start radio">${icon('radio')}</button></li>`;
      })}</ul>` : html`<p class="disc-note">You don't follow any tags yet. Follow one and its new releases show up here.</p>`}
      ${related.length ? html`${ui.labelRule('you might like', { level: 'h3', sm: true })}<div class="cluster">${related.map(function (id) { return ui.tagChip(id, { toggle: true, pressed: false }); })}</div>` : ''}`;
  }

  /* cold start: the server had nothing to go on (no picks, no history, nothing followed) and offers tags to begin with */
  function isCold(feed) {
    return !list(feed, 'picks').length && !list(feed, 'jumpBackIn').length && !list(feed, 'followedTags').length &&
      !Object.keys(feed.errors || {}).length && list(feed, 'coldStartTags').length > 0;
  }
  function renderCold(feed) {
    var tags = list(feed, 'coldStartTags'), artists = list(feed, 'coldStartArtists').map(M.artist).filter(Boolean);
    return html`<div class="disc-home">
      ${hello('Nothing to go on yet. Pick a few tags or artists and your feed fills in from there.')}
      <section class="card disc-card disc-cold" aria-labelledby="disc-h-cold">
        ${ui.labelRule('pick a few to start', { id: 'disc-h-cold' })}
        <div class="disc-cloud" role="group" aria-label="Tags">${tags.map(function (id, i) { return coldTag(id, 3 - Math.floor(i * 3 / tags.length)); })}</div>
        ${artists.length ? html`${ui.labelRule('or a few artists', { level: 'h3', sm: true })}
        <ul class="disc-cold__artists">${artists.slice(0, 6).map(function (a) {
          return html`<li class="disc-cold__artist">${ui.cover(a.id, 56, { round: true })}<span class="disc-cold__name">${a.name}<small>${a.tags[0] || ''}</small></span>${coldArtistBtn(a)}</li>`;
        })}</ul>` : ''}
        <div class="disc-cold__foot">
          <span class="disc-cold__count" data-cold-count aria-live="polite">${coldCount()}</span>
          <span class="spacer"></span>
          <a class="btn btn--quiet" href="#/search">Try a search instead</a>
          <button class="btn btn--primary" data-act="disc-build-feed">${icon('sparkle')}Build my feed</button>
        </div>
      </section>
    </div>`;
  }
  /* w = cloud weight 1-3; the server lists the most common tags first, so they draw largest */
  function coldTag(id, w) {
    var on = !!H.coldPicks[id];
    return html`<button class="chip disc-cloud__tag disc-cloud__tag--w${w || 1}" data-act="disc-cold-pick" data-id="${id}" aria-pressed="${on ? 'true' : 'false'}">${on ? icon('check') : ''}${M.tag(id).name}</button>`;
  }
  function coldArtistBtn(a) {
    var on = !!H.coldPicks[a.id];
    return html`<button class="btn btn--sm disc-cold__follow" data-act="disc-cold-pick" data-id="${a.id}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Follow ${a.name}">${icon(on ? 'check' : 'plus')}${on ? 'Following' : 'Follow'}</button>`;
  }
  function coldCount() {
    var n = Object.keys(H.coldPicks).length;
    return n ? plural(n, 'pick') + ' so far' : 'Pick at least one';
  }

  /* home actions */
  /* refetch the feed in place: the named shelves show skeletons until it answers, the rest stays put */
  function reloadFeed(keys) {
    var r = M.get(FEED);
    H.reloading = { shelves: keys, data: r.data, error: r.error };
    keys.forEach(paintShelf);
    App.announce('Reloading your feed');
    M.reload(FEED);
  }
  App.action('disc-retry-shelf', function (el) { reloadFeed([el.getAttribute('data-id')]); });
  App.action('disc-refresh-picks', function () { reloadFeed(['picks']); });
  /* See all / Show less: a Home shelf grows in place to show everything the feed holds */
  App.action('disc-expand-new', function () { H.expanded.new = !H.expanded.new; paintShelf('newReleases'); });
  App.action('disc-expand-picks', function () { H.expanded.picks = !H.expanded.picks; paintShelf('picks'); });
  /* a pick with no source: ask Soulseek again; the first source that answers makes it playable */
  App.action('disc-find', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id);
    if (!t || H.finding) return;
    H.finding = id; paintShelf('picks'); App.announce('Searching Soulseek for ' + t.title);
    var stop = watchSources(id, function (res) {
      var found = !res.error && res.sources.length > 0;
      if (!res.error && !found && !res.done) return;
      stop(); H.finding = null;
      if (found) H.found[id] = true;
      paintShelf('picks');
      if (res.error) App.toast('Couldn\'t search Soulseek: ' + res.error.message, { kind: 'error' });
      else if (found) App.toast('Found ' + plural(res.sources.length, 'source') + ' for ' + t.title, { kind: 'ok', icon: 'soulseek', action: 'Play', onAction: function () { P.playTrack(id); } });
      else App.toast('Still no source for ' + t.title + '. Peers may share it later.', { icon: 'no-source' });
    });
  });
  /* "less like this" is feedback the server keeps; once it has, the pick leaves the list */
  function lessLike(id) {
    App.closePopover(true);
    M.api('/api/feedback', { method: 'POST', body: { kind: 'less-like', id: id } }).then(function () {
      var wasPick = !!pickFor(id);
      if (wasPick) { H.dismissed[id] = true; paintShelf('picks'); }
      App.toast('Got it: less like this' + (wasPick ? ', and removed from your picks' : ''), { icon: 'info' });
    }, function (err) { App.toast('Couldn\'t send that to your feed: ' + err.message, { kind: 'error' }); });
  }
  App.action('disc-less', function (el) { lessLike(el.getAttribute('data-id')); });
  App.action('disc-pick-menu', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id); if (!t) return;
    var p = pickFor(id), followed = App.isFollowed(t.artistId), noSrc = p && p.noSource && !H.found[id];
    var items = noSrc ? [] : [
      { label: 'Play next', icon: 'queue', run: function () { P.playNext([id]); } },
      { label: 'Add to queue', icon: 'plus', run: function () { P.addToQueue([id]); } },
      { label: App.isSaved(id) ? 'In your library' : 'Save to library', icon: App.isSaved(id) ? 'saved' : 'save', run: function () { App.saves.start([id]); } }
    ];
    items = items.concat([
      { label: followed ? 'Unfollow ' + t.artist : 'Follow ' + t.artist, icon: 'follow', run: function () { App.toggleFollow(t.artistId); } },
      { label: 'Start radio', icon: 'radio', run: function () { startRadio({ kind: 'track', id: id }); } }
    ]);
    if (!noSrc) items.push({ label: 'Show sources', icon: 'soulseek', run: function () { openPicker(id); } });
    items.push('sep', { label: 'Not interested', icon: 'close', run: function () { lessLike(id); } });
    App.menu(el, items, { label: 'Actions for ' + t.title });
  });
  App.action('disc-cold-pick', function (el) {
    var id = el.getAttribute('data-id');
    if (H.coldPicks[id]) delete H.coldPicks[id]; else H.coldPicks[id] = true;
    var a = M.artist(id), next = String(a ? coldArtistBtn(a) : coldTag(id));
    var tmp = document.createElement('div'); tmp.innerHTML = next;
    el.setAttribute('aria-pressed', tmp.firstChild.getAttribute('aria-pressed'));
    el.innerHTML = tmp.firstChild.innerHTML;
    var c = $('[data-cold-count]'); if (c) c.textContent = coldCount();
  });
  /* follow every cold-start pick (PUT /api/follows/...), then fetch the feed those follows build */
  App.action('disc-build-feed', function () {
    var ids = Object.keys(H.coldPicks);
    if (!ids.length) { App.toast('Pick at least one tag or artist first', { icon: 'info' }); return; }
    H.building = true; App.refresh();
    function refreshHome() { H.building = false; if (App.current.path === '/home') App.refresh(); }
    Promise.all(ids.map(function (id) {
      return M.api('/api/follows/' + (/^ar-/.test(id) ? 'artist' : 'tag') + '/' + encodeURIComponent(id), { method: 'PUT' }).then(function () {
        App.store.update('followed', function (f) { f[id] = true; });
        delete H.coldPicks[id];
      });
    })).then(function () {
      M.invalidate([FEED]); refreshHome();
      App.toast('Following ' + plural(ids.length, 'pick') + '. Your feed is building from them.', { kind: 'ok', icon: 'sparkle' });
    }, function (err) {
      refreshHome();
      App.toast('Couldn\'t follow your picks: ' + err.message, { kind: 'error' });
    });
  });

  function mountHome(ctx) {
    /* the hub rebuilt the feed, or a shelf filled in later (new releases once Soulseek has answered): fetch it again */
    ctx.onCleanup(App.on('server:feed.updated', function () { if (!H.building) M.reload(FEED); }));   // old shelves stay until it answers
    ctx.onCleanup(App.on('store', function (keys) {
      var feed = M.peek(FEED);
      if (!feed || H.building || isCold(feed)) return;
      if (keys.indexOf('hiddenArtists') >= 0) paintShelf('picks');
      if (keys.indexOf('followed') >= 0) paintShelf('followedTags');
    }));
  }

  /* reason popover: every reason for the item, the data source, and the three feedback actions */
  var REASON_ICON = { like: 'heart-fill', tag: 'sparkle', listeners: 'peer', fresh: 'bell' };
  function reasonText(r) {
    if (r.kind === 'like') return html`You play <b>${r.name}</b> a lot and liked several of their songs this month.`;
    if (r.kind === 'listeners') return html`People who play <b>${r.name}</b> also play this a lot.`;
    if (r.kind === 'fresh') return html`<b>${r.name}</b> released this recently, and you follow them.`;
    return html`It sits in <b>${r.name}</b>, a tag you follow and play often.`;
  }
  App.hooks.reason = function (anchor, r) {
    var p = r.item ? pickFor(r.item) : null, t = r.item ? M.track(r.item) : null;
    var reasons = p && p.reasons && p.reasons.length ? p.reasons : [{ kind: r.kind, name: r.name, ref: r.ref }];
    var artist = t ? M.artist(t.artistId) : (r.kind !== 'tag' && r.ref ? M.artist(r.ref) : null);
    /* feedback takes a track or an artist id; a tag reason, or "listeners like you" (no ref), has only the item */
    var target = r.item || (/^ar-/.test(r.ref || '') ? r.ref : ''), via = r.via || (p && p.via);
    App.popover(anchor, html`<div class="popover__head">${icon('sparkle')}<span>why this?</span></div>
      <div class="popover__body disc-why">
        ${t ? html`<p class="disc-why__item">${t.title}, ${t.artist}</p>` : ''}
        <ul class="disc-why__list">${reasons.map(function (x) { return html`<li>${icon(REASON_ICON[x.kind] || 'sparkle')}<span>${reasonText(x)}</span></li>`; })}</ul>
        ${via ? html`<p class="disc-why__via">via ${via}</p>` : ''}
      </div>
      <div class="popover__actions">
        ${target ? html`<button class="btn btn--sm" data-act="more-like" data-id="${target}">${icon('plus')}More like this</button>
        <button class="btn btn--sm" data-act="disc-less" data-id="${target}">${icon('minus')}Less like this</button>` : ''}
        ${artist ? html`<button class="btn btn--sm" data-act="hide-artist" data-id="${artist.id}" aria-label="Hide ${artist.name} from your feed">Hide this artist</button>` : ''}
      </div>`, { label: 'Why this was suggested', className: 'disc-why-pop' });
  };

  /* =====================================================================
     2. Search (ADR-0008): the catalogs answer first (GET /api/search: Deezer and saved songs at once, MusicBrainz and
        Last.fm as `search.more`), Soulseek behind them (GET /api/soulseek/search, rows as `search.results`): the
        songs no catalog knows are "Only on Soulseek"; raw folders and files show in a Files tab when
        Settings > Search > "Show file results" is on.
     ===================================================================== */
  /* one search session. cat = the catalog answer { top, artists, albums, tracks, pending, errors }, grown in place by
     search.more (appended, never reordered). Soulseek runs for 3+ characters once the query is submitted (Enter) or
     has rested a moment; while it runs only the files / peers counters move, and its last event carries the rows:
     peerSongs: trackId -> { trackId, sources, best, dup }, albums: albumId -> { albumId, trackIds, folders } (arrival
     order: peerOrder, albumOrder), onlySoulseek: the songs no catalog knew. */
  /* TYPE_PAUSE_MS: a search goes out once typing pauses this long (each one spends the catalogs' budgets: iTunes about
     18 a minute); the header field uses the same pause */
  var SLSK_REST_MS = 1500, SONGS_MAX = 20, PENDING_MAX_MS = 20000, TYPE_PAUSE_MS = 350;
  var CATALOG = { deezer: 'Deezer', itunes: 'iTunes', musicbrainz: 'MusicBrainz', 'musicbrainz-albums': 'MusicBrainz', lastfm: 'Last.fm', library: 'your library' };
  var S = { q: null, seq: 0, showAll: false, showAllOnly: false, tab: 'all', loading: false, error: null, cat: null,
    slsk: false, slskStarted: false, slskTimer: null, slskMissing: false, searchId: null, files: 0, peers: 0, done: true, slskError: null,
    peerSongs: {}, peerOrder: [], albums: {}, albumOrder: [], onlySoulseek: [] };

  function slskDown() { var s = App.store.get('services'); return !!(s && s.soulseek && s.soulseek.state === 'down'); }
  function streaming() { return S.slskStarted && !slskDown() && !S.done; }
  function search(q) {
    var mine = S.seq += 1;
    clearTimeout(S.slskTimer);
    /* while the next answer comes, the last one stays on screen, dimmed, instead of a skeleton per keystroke */
    var stale = q ? (S.cat && !S.loading ? S.cat : S.stale) : null;
    Object.assign(S, { stale: stale, q: q, showAll: false, showAllOnly: false, loading: !!q, error: null, cat: null, slsk: q.length >= 3, slskStarted: false, slskMissing: false,
      searchId: null, files: 0, peers: 0, done: true, slskError: null, peerSongs: {}, peerOrder: [], albums: {}, albumOrder: [], onlySoulseek: [] });
    if (!q) return;
    M.api('/api/search?q=' + encodeURIComponent(q)).then(function (res) {
      if (mine !== S.seq) return;
      S.loading = false;
      /* a hub from before ADR-0008 answers this path the old way: say so rather than show half of it */
      if (!Array.isArray(res.tracks) || !Array.isArray(res.artists) || !Array.isArray(res.albums)) {
        S.error = { status: 0, message: 'This hub still searches the old way. Update lull on the server for catalog search.' };
      } else {
        S.cat = { top: res.top || null, artists: res.artists.slice(), albums: res.albums.slice(), tracks: res.tracks.slice(), playlists: (res.playlists || []).slice(),
          pending: [], errors: {} };
        mergeCatalog(res); setPending(res.pending, mine);
      }
      paint('results', resultsBody());
    }, function (err) {
      if (mine !== S.seq) return;
      S.loading = false; S.error = err;
      paint('results', resultsBody());
    });
    if (S.slsk) S.slskTimer = setTimeout(function () { startSoulseek(mine); }, SLSK_REST_MS);
  }
  function startSoulseek(mine) {
    if (mine !== S.seq || !S.slsk || S.slskStarted) return;
    clearTimeout(S.slskTimer);
    if (slskDown()) { paint('results', resultsBody()); return; }
    S.slskStarted = true; S.done = false;
    /* submitted words also widen the catalog run (MusicBrainz release groups by title, which finds Bandcamp-only EPs);
       the answer merges like search.more */
    M.api('/api/search?q=' + encodeURIComponent(S.q) + '&submitted=1').then(function (res) {
      if (mine === S.seq && S.cat) { mergeCatalog(res); setPending(res.pending, mine); paint('results', resultsBody()); }
    }, function (err) { console.warn('Submitted catalog search failed:', err.message); });
    M.api('/api/soulseek/search?q=' + encodeURIComponent(S.q)).then(function (res) {
      if (mine !== S.seq) return;
      S.searchId = res.searchId;
      /* the same words within 10 minutes reuse the search: its latest event counts as if it had just arrived */
      if (res.soulseek) { M.absorb(res.soulseek.entities); applyResults(res.soulseek); }
      paint('results', resultsBody());
    }, function (err) {
      if (mine !== S.seq) return;
      S.done = true;
      if (err.status === 404) S.slskMissing = true; else S.slskError = err.message;
      paint('results', resultsBody());
    });
  }
  /* app.js has already absorbed e.entities. An event can beat the HTTP answer that names the search (a Soulseek that is
     down answers with its final event at once): the same words while one runs count as this search */
  App.on('server:search.results', function (e) {
    if (!e) return;
    if (!S.searchId && S.slskStarted && !S.done && sameQuery(e.query, S.q)) S.searchId = e.searchId;
    if (!S.searchId || e.searchId !== S.searchId) return;
    applyResults(e);
    paint('results', resultsBody());
  });
  /* catalog additions: appended to each list, so nothing the listener is reading moves */
  /* the hub's `query` is the words lowercased with single spaces */
  function words(q) { return String(q || '').trim().toLowerCase().replace(/\s+/g, ' '); }
  function sameQuery(a, b) { return words(a) === words(b); }
  function mergeCatalog(e) {
    ['artists', 'albums', 'tracks'].forEach(function (k) {
      (e[k] || []).forEach(function (id) { if (S.cat[k].indexOf(id) < 0) S.cat[k].push(id); });
    });
    /* playlists are rows, not entities: someone's playlist on YouTube or Deezer (discovery.md#playlists-in-search) */
    (e.playlists || []).forEach(function (p) { if (!S.cat.playlists.some(function (x) { return x.id === p.id; })) S.cat.playlists.push(p); });
    if (e.errors) S.cat.errors = Object.assign({}, S.cat.errors, e.errors);     // every source that failed so far
    S.cat.pending = S.cat.pending.filter(function (k) { return !S.cat.errors[k]; });   // a failed source isn't coming
  }
  /* the sources still searching; one that hasn't answered within PENDING_MAX_MS is noted as not answering */
  var pendingTimer = null;
  function setPending(list, mine) {
    S.cat.pending = (list || []).filter(function (k) { return !S.cat.errors[k]; });
    clearTimeout(pendingTimer);
    if (!S.cat.pending.length) return;
    pendingTimer = setTimeout(function () {
      if (mine !== S.seq || !S.cat || !S.cat.pending.length) return;
      S.cat.pending.forEach(function (k) { S.cat.errors[k] = 'no answer in time'; });
      S.cat.pending = [];
      paint('results', resultsBody());
    }, PENDING_MAX_MS);
  }
  App.on('server:search.more', function (e) {
    if (!e || !S.cat || !sameQuery(e.query, S.q)) return;
    mergeCatalog(e);
    if (e.done) { S.cat.pending = []; clearTimeout(pendingTimer); }
    paint('results', resultsBody());
  });
  function applyResults(e) {
    S.files = e.files || 0; S.peers = e.peers || 0;
    if (!e.done) return;
    S.slskError = e.error || null;
    S.peerSongs = {}; S.peerOrder = []; S.albums = {}; S.albumOrder = [];
    S.onlySoulseek = (e.onlySoulseek || []).slice();
    (e.results || []).forEach(function (row) {
      if (row.kind === 'track') { S.peerSongs[row.trackId] = row; S.peerOrder.push(row.trackId); }
      else if (row.kind === 'album') { S.albums[row.albumId] = row; S.albumOrder.push(row.albumId); }
      else console.error('Unknown search result', row);
    });
    finish();
  }
  function finish() {
    if (S.done) return;
    S.done = true;
    addRecent(S.q);
    App.announce(scanText());
  }

  /* an album's songs: the entity's, or the Soulseek folder row's when the entity doesn't list them */
  function trackIdsOf(a) { var row = S.albums[a.id]; return a.trackIds.length || !row ? a.trackIds : row.trackIds || []; }
  /* the best Soulseek copy's format: a song's best source, an album's best folder */
  function peerFormat(t) { var row = S.peerSongs[t.id]; return row && row.best && row.best.format; }
  function albumFormat(a) { var row = S.albums[a.id]; return row && row.folders && row.folders[0] ? row.folders[0].format : a.format; }
  function showFiles() { var r = M.peek('/api/settings'); return !!(r && r.search && r.search.showFiles); }

  function renderSearch(ctx) {
    var q = (ctx.query.q || '').trim();
    if (q !== S.q) search(q);
    M.get('/api/settings');               // the Files tab follows Settings > Search
    return html`<div class="disc-search">
      <header class="disc-hello disc-hello--search">
        <h1 class="t-display">search</h1>
        <p class="disc-hello__sum" data-region="sub">${subline()}</p>
      </header>
      <div class="disc-q search-field" role="search">
        ${icon('search')}
        <label class="vh" for="disc-q">Search artists, albums and songs</label>
        <input class="input" id="disc-q" type="search" maxlength="200" value="${q}" placeholder="Artists, albums, songs" autocomplete="off" spellcheck="false" enterkeyhint="search">
      </div>
      <div class="disc-filters" role="group" aria-label="Search view" data-filters ${q ? '' : raw('hidden')}>
        <span class="disc-split">${ui.filterPill('Side by side', { act: 'disc-split', pressed: App.layout.get().searchSplit, icon: 'split' })}</span>
      </div>
      <div class="disc-results" data-region="results">${q ? resultsBody() : emptySearchBody()}</div>
    </div>`;
  }
  function subline() {
    return S.q ? html`Results for <b>${S.q}</b>: artists, albums and songs, then what only Soulseek peers share.`
      : 'Artists, albums and songs from the catalogs and your library, then Soulseek peers.';
  }

  function emptySearchBody() {
    return html`<div class="disc-duo">
      <section class="card disc-card" aria-labelledby="disc-h-recent" data-region="recents">${recentsBody()}</section>
      <section class="card disc-card" aria-labelledby="disc-h-sugg">
        ${ui.labelRule('from tags you follow', { id: 'disc-h-sugg' })}
        <div class="cluster disc-sugg">${followedTagIds().map(function (id) { var t = M.tag(id); return ui.chip(t.name, { href: '#/search?q=' + encodeURIComponent(t.name), icon: 'search' }); })}</div>
        <p class="disc-note">Press ${ui.keycap('/')} anywhere to jump here. Saved songs show up at once; Soulseek takes about ten seconds to hear back from peers.</p>
      </section>
    </div>`;
  }
  function recentsBody() {
    var rec = App.store.get('recentSearches') || [];
    var head = ui.labelRule('recent searches', { id: 'disc-h-recent', action: rec.length ? { label: 'Clear all', act: 'disc-recent-clear' } : null });
    if (!rec.length) return html`${head}<p class="disc-note">No recent searches. What you search for shows up here so you can run it again.</p>`;
    return html`${head}<ul class="disc-recents">${rec.map(function (q) {
      return html`<li class="disc-recent"><a class="disc-recent__q" href="#/search?q=${encodeURIComponent(q)}">${icon('clock')}<span>${q}</span></a>
        <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="disc-recent-remove" data-q="${q}" aria-label="Remove ${q} from recent searches" data-tip="Remove">${icon('close', null, 'ic-16')}</button></li>`;
    })}</ul>`;
  }

  function resultsBody() {
    if (S.loading && S.stale) {
      var fresh = S.cat; S.cat = S.stale;
      var dim = html`<div class="disc-col is-stale" aria-busy="true">${catalogBody()}</div>`;
      S.cat = fresh; return dim;
    }
    if (S.loading) return html`<section class="card disc-card">${ui.labelRule('songs')}${ui.skeleton('row', 4)}</section>`;
    if (S.error) {
      var gone = S.error.status === 404 || S.error.status === 0;
      return html`<section class="card disc-card">${ui.emptyState({ art: 'star', title: gone ? 'Search needs a newer hub' : 'Couldn\'t search', text: S.error.message,
        action: gone ? null : { label: 'Try again', act: 'disc-search-again' } })}</section>`;
    }
    var two = splitView(), files = showFiles();
    if (two) return html`<div class="disc-splitview"><div class="disc-col">${catalogBody()}</div><div class="disc-col">${soulseekBody()}${files ? filesBody() : ''}</div></div>`;
    var tabs = files ? html`<div class="seg disc-tabs" role="tablist" aria-label="Results" style="--n:2;--i:${S.tab === 'files' ? 1 : 0}"><span class="seg__ind" aria-hidden="true"></span>
      <button class="seg__btn" role="tab" aria-selected="${String(S.tab !== 'files')}" tabindex="${S.tab !== 'files' ? 0 : -1}" data-act="disc-tab" data-id="all">All</button>
      <button class="seg__btn" role="tab" aria-selected="${String(S.tab === 'files')}" tabindex="${S.tab === 'files' ? 0 : -1}" data-act="disc-tab" data-id="files">Files</button></div>` : '';
    if (files && S.tab === 'files') return html`${tabs}${filesBody()}`;
    return html`${tabs}<div class="disc-col">${catalogBody()}${soulseekBody()}</div>`;
  }
  function entityOf(top) {
    var item = top && (top.kind === 'track' ? M.track(top.id) : top.kind === 'album' ? M.album(top.id) : top.kind === 'artist' ? M.artist(top.id)
      : top.kind === 'playlist' ? S.cat.playlists.filter(function (p) { return p.id === top.id; })[0] : null);
    return item ? { kind: top.kind, item: item } : null;
  }
  /* the catalogs' answer: top result and songs side by side when there is room, then artists, then releases */
  function catalogBody() {
    var C = S.cat, top = entityOf(C.top);
    var tracks = C.tracks.map(M.track).filter(Boolean), artists = C.artists.map(M.artist).filter(Boolean), albums = C.albums.map(M.album).filter(Boolean);
    var lists = C.playlists.filter(function (p) { return !top || top.kind !== 'playlist' || top.item.id !== p.id; });
    if (!top && !tracks.length && !artists.length && !albums.length && !lists.length) {
      return html`<section class="card disc-card">${pendingLine()}${C.pending.length ? ui.skeleton('row', 3)
        : ui.emptyState({ art: 'moon', title: 'Nothing matches ' + S.q, text: S.slsk ? 'Soulseek peers may still share it: see below.' : 'Type at least three characters to ask Soulseek peers too.' })}</section>`;
    }
    var shown = S.showAll ? tracks : tracks.slice(0, SONGS_MAX);
    /* catalogs still answering show in the songs heading, so the line going away moves nothing below it */
    var songsHead = ui.labelRule('songs', { id: 'disc-h-songs', end: pendingLine() });
    return html`<div class="disc-cat__lead">
        ${top ? html`<section class="card disc-card disc-top" aria-labelledby="disc-h-top">${ui.labelRule('top result', { id: 'disc-h-top' })}${topCard(top)}</section>` : ''}
        ${tracks.length ? html`<section class="card disc-card disc-songs${S.showAll ? ' is-all' : ''}" aria-labelledby="disc-h-songs">
          ${songsHead}
          <ol class="tlist tlist--dense" data-ids="${shown.map(function (t) { return t.id; }).join(',')}" aria-label="Songs">${shown.map(songRow)}</ol>
          ${!S.showAll && tracks.length > 5 ? html`<button class="btn btn--quiet disc-more" data-act="disc-show-all">Show all ${plural(tracks.length, 'song')}</button>` : ''}
        </section>` : C.pending.length ? html`<section class="card disc-card disc-songs" aria-labelledby="disc-h-songs" aria-busy="true">
          ${songsHead}<div class="disc-songs__wait">${ui.skeleton('row', 5)}</div>
        </section>` : ''}
      </div>
      ${artists.length ? html`<section class="card disc-card" aria-labelledby="disc-h-artists">${ui.labelRule('artists', { id: 'disc-h-artists' })}
        <div class="shelf disc-shelf disc-shelf--artists">${artists.map(artistTile)}</div></section>` : ''}
      ${albums.length ? html`<section class="card disc-card" aria-labelledby="disc-h-albums">${ui.labelRule('albums, EPs and singles', { id: 'disc-h-albums' })}
        <div class="shelf disc-shelf">${albums.map(function (a) { return ui.releaseTile(a, { showNew: false }); })}</div></section>` : ''}
      ${lists.length ? html`<section class="card disc-card" aria-labelledby="disc-h-playlists">${ui.labelRule('playlists', { id: 'disc-h-playlists', end: html`<span class="t-micro fg-2">from YouTube and Deezer</span>` })}
        <div class="shelf disc-shelf">${lists.map(playlistTile)}</div></section>` : ''}
      ${catalogErrors()}`;
  }
  /* catalogs still answering: a quiet line; their additions arrive in place */
  function pendingLine() {
    var names = unique((S.cat ? S.cat.pending : []).map(function (k) { return CATALOG[k]; }).filter(Boolean));
    if (!names.length) return '';
    return html`<span class="disc-pending" role="status"><span class="spin-ring" aria-hidden="true"></span>Still asking ${names.join(' and ')}</span>`;
  }
  /* a quiet line: which catalogs didn't answer (their own sentences as a tooltip) */
  function catalogErrors() {
    var e = S.cat.errors, keys = Object.keys(e), names = unique(keys.map(function (k) { return CATALOG[k] || k; }));
    if (!names.length) return '';
    return html`<p class="disc-note" title="${keys.map(function (k) { return (CATALOG[k] || k) + ': ' + e[k]; }).join('\n')}">${names.join(' and ')} didn't answer, so ${names.length > 1 ? 'their' : 'its'} results are missing.</p>`;
  }
  function songRow(t, i) {
    var row = S.peerSongs[t.id], saved = App.isSaved(t.id);
    var dup = row && row.dup === 'better' && !slskDown() ? 'better' : null;
    /* a Soulseek row carries its peer count ("N sources") and, for a song not saved, the best file's format */
    var shownTrack = row ? Object.assign({}, t, { sources: row.sources }, !saved && row.best ? { format: row.best.format } : {}) : t;
    var markup = String(ui.trackRow(shownTrack, { dense: true, showSources: true, index: i + 1 }));
    /* the "better than saved" marker goes on the artist line, so badges never squeeze the title */
    return raw(markup.replace(/(<div class="trow__artist">[\s\S]*?)(<\/div>)/, function (m, a, b) { return a + (dup ? String(ui.dupBetter()) : '') + b; }));
  }
  /* "Only on Soulseek": songs no catalog knew, built from file tags */
  function soulseekBody() {
    if (!S.slsk) return '';
    var only = S.onlySoulseek.map(M.track).filter(Boolean);
    /* a popular query leaves hundreds of songs only peers know (196 for "toro": 8,000 DOM nodes, most of the page's weight and
       its style work): the first SONGS_MAX show, the rest on request, as in Songs above */
    var shownOnly = S.showAllOnly ? only : only.slice(0, SONGS_MAX);
    return html`<section class="card disc-card disc-only" aria-labelledby="disc-h-only">
      ${ui.labelRule('only on soulseek', { id: 'disc-h-only' })}
      ${scanLine()}
      ${only.length ? html`<ol class="tlist tlist--dense" data-ids="${shownOnly.map(function (t) { return t.id; }).join(',')}" aria-label="Only on Soulseek">${shownOnly.map(songRow)}</ol>
        ${shownOnly.length < only.length ? html`<button class="btn btn--quiet disc-more" data-act="disc-only-all">Show all ${plural(only.length, 'song')}</button>` : ''}`
        : S.done && S.slskStarted && !S.slskError && !S.slskMissing ? html`<p class="disc-note">${S.peerOrder.length
          ? 'Every song peers share here is in the catalogs above.' : 'Soulseek found nothing for these words.'}</p>` : ''}
      ${streaming() ? ui.skeleton('row', 2) : ''}
    </section>`;
  }
  /* Files tab: what Soulseek peers share, as songs merged across peers and as folders */
  function filesBody() {
    var songs = S.peerOrder.map(M.track).filter(Boolean), folders = S.albumOrder.map(M.album).filter(Boolean);
    return html`<section class="card disc-card disc-files" aria-labelledby="disc-h-files">
      ${ui.labelRule('files', { id: 'disc-h-files' })}
      ${S.slsk ? scanLine() : html`<p class="disc-note">Type at least three characters to ask Soulseek peers.</p>`}
      ${songs.length ? html`<ol class="tlist tlist--dense" data-ids="${songs.map(function (t) { return t.id; }).join(',')}" aria-label="Songs peers share">${songs.map(songRow)}</ol>` : ''}
      ${folders.length ? html`${ui.labelRule('folders', { level: 'h3', sm: true })}<div class="shelf disc-shelf">${folders.map(folderTile)}</div>` : ''}
    </section>`;
  }

  /* side by side (ADR-0007, from wide): the catalogs on the left, Soulseek on the right */
  function splitView() { return App.layout.get().searchSplit && App.layout.at('wide'); }
  App.action('disc-split', function () {
    App.layout.set({ searchSplit: !App.layout.get().searchSplit });
    App.refresh();
    var b = $('[data-act="disc-split"]'); if (b) b.focus();
  });
  /* crossing the wide line turns side by side on or off */
  App.on('layout:size', function () { if (App.current.path === '/search' && App.layout.get().searchSplit) App.refresh(); });
  App.action('disc-tab', function (el) {
    S.tab = el.getAttribute('data-id');
    paint('results', resultsBody());
    var b = $('[data-act="disc-tab"][data-id="' + S.tab + '"]'); if (b) b.focus();
  });

  /* Soulseek line: before it asks (the query is resting), the live counters while peers answer, then the totals */
  function scanLine() {
    if (slskDown()) {
      return html`<div class="disc-scan is-down" role="status">${ui.statusGlyph('down', 'Soulseek down')}<span class="disc-scan__text">Soulseek is down, so only the catalogs were searched.</span>
        <button class="btn btn--sm" data-act="open-sources">Soulseek settings</button></div>`;
    }
    if (S.slskMissing) return html`<p class="disc-note">Soulseek search needs a newer hub.</p>`;
    if (!S.slskStarted) {
      return html`<div class="disc-scan disc-scan--wait">${icon('soulseek')}<span class="disc-scan__text">Soulseek peers are asked when you stop typing.</span>
        <button class="btn btn--quiet btn--sm" data-act="disc-slsk-now">Ask now</button></div>`;
    }
    if (S.slskError) {
      return html`<div class="disc-scan is-down" role="status">${ui.statusGlyph('down', 'Soulseek search failed')}<span class="disc-scan__text">Soulseek search failed: ${S.slskError}</span>
        <button class="btn btn--sm" data-act="disc-search-again">${icon('refresh')}Search again</button></div>`;
    }
    var done = S.done;
    return html`<div class="disc-scan${done ? ' is-done' : ''}" data-scan>
      ${icon('soulseek')}
      <span class="disc-scan__text" data-scan-text aria-live="polite">${scanText()}</span>
      ${done ? html`<button class="btn btn--quiet btn--sm" data-act="disc-search-again">${icon('refresh')}Search again</button>` : ''}
      <div class="progress${done ? '' : ' progress--xfer'} disc-scan__bar" role="progressbar" aria-label="Soulseek search" aria-valuetext="${done ? 'finished' : 'still asking peers'}" data-scan-bar style="--p:1"><i></i></div>
    </div>`;
  }
  function scanText() {
    if (!S.done) return 'Searching Soulseek: ' + plural(S.files, 'file') + ' from ' + plural(S.peers, 'peer');
    return 'Soulseek: ' + plural(S.peerOrder.length, 'song') + ' and ' + plural(S.albumOrder.length, 'folder') + ' from ' + plural(S.peers, 'peer');
  }

  function topCard(top) {
    if (top.kind === 'track') {
      var t = top.item, saved = App.isSaved(t.id), row = S.peerSongs[t.id], best = peerFormat(t);
      var better = !!(row && row.dup === 'better') && !slskDown();
      return html`<div class="disc-top__body">
          ${ui.cover(t, 112)}
          <div class="disc-top__info">
            <p class="disc-top__kind">song</p>
            <h3 class="disc-top__title">${t.title}</h3>
            <p class="disc-top__by"><a href="#/artist/${t.artistId}">${t.artist}</a>, <a href="#/album/${t.albumId}">${t.album}</a></p>
            <div class="cluster disc-top__badges">${ui.sourceBadge(t)}${ui.qualityChip(saved || !best ? t.format : best)}
              ${row && row.sources ? html`<button class="disc-top__n" data-act="show-sources" data-id="${t.id}">${plural(row.sources, 'source')}</button>` : ''}${better ? ui.dupBetter() : ''}</div>
          </div>
        </div>
        <div class="disc-top__actions">
          <button class="btn btn--primary" data-act="play-track" data-id="${t.id}">${icon('play')}${saved ? 'Play' : 'Play best source'}</button>
          ${saved ? '' : html`<button class="btn" data-act="show-sources" data-id="${t.id}">${icon('soulseek')}Pick a source</button>`}
          ${better ? html`<button class="btn" data-act="disc-save-version" data-id="${t.id}">${icon('arrow-up')}Save the better copy</button>` : ui.saveButton(t.id)}
          ${ui.iconButton('radio', 'Start radio from ' + t.title, { 'data-act': 'disc-radio', 'data-kind': 'track', 'data-id': t.id })}
        </div>
        <div data-issue="${t.id}">${saveIssue(t.id)}</div>`;
    }
    if (top.kind === 'album') {
      var a = top.item, ids = trackIdsOf(a), savedN = ids.filter(App.isSaved).length, fmtA = albumFormat(a);
      return html`<div class="disc-top__body">
          ${ui.cover(a, 112)}
          <div class="disc-top__info">
            <p class="disc-top__kind">${a.type.toLowerCase()}${S.albums[a.id] ? ', Soulseek folder' : ''}</p>
            <h3 class="disc-top__title">${S.albums[a.id] && !a.trackIds.length ? a.title : html`<a href="#/album/${a.id}">${a.title}</a>`}</h3>
            <p class="disc-top__by"><a href="#/artist/${a.artistId}">${a.artist}</a>${a.year ? ', ' + a.year : ''}</p>
            <div class="cluster disc-top__badges">${ids.length && savedN === ids.length ? ui.savedBadge() : savedN ? html`<span class="badge badge-saved">${icon('saved')}${savedN} of ${ids.length} saved</span>` : ui.soulseekBadge()}
              ${fmtA ? ui.qualityChip(fmtA) : ''}${ids.length ? html`<span class="px fg-2">${plural(ids.length, 'track')}${a.sizeMB ? ', ' + size(a.sizeMB) : ''}</span>` : ''}</div>
          </div>
        </div>
        <div class="disc-top__actions">
          <button class="btn btn--primary" data-act="play-album" data-id="${a.id}">${icon('play')}Play album</button>
          ${S.albums[a.id] && !slskDown() ? html`<button class="btn" data-act="disc-folder" data-id="${a.id}">${icon('soulseek')}Open folder</button>`: ''}
          ${!ids.length || savedN < ids.length ? html`<button class="btn" data-act="save-album" data-id="${a.id}">${icon('save')}Save whole album</button>` : ''}
          ${ui.iconButton('radio', 'Start radio from ' + a.title, { 'data-act': 'disc-radio', 'data-kind': 'album', 'data-id': a.id })}
        </div>`;
    }
    if (top.kind === 'playlist') {
      var pl = top.item, href = App.outside.href(pl.id);
      return html`<div class="disc-top__body">
          ${ui.mosaic([], 112, '', pl.cover)}
          <div class="disc-top__info">
            <p class="disc-top__kind">playlist</p>
            <h3 class="disc-top__title"><a href="${href}">${pl.title}</a></h3>
            <p class="disc-top__by">${pl.owner ? 'by ' + pl.owner + ', ' : ''}${playlistFacts(pl)}</p>
          </div>
        </div>
        <div class="disc-top__actions">
          <a class="btn btn--primary" href="${href}">${icon('play')}Open playlist</a>
          ${App.outside.button(pl.id, App.outside.saved(pl.id))}
        </div>`;
    }
    var ar = top.item, on = App.isFollowed(ar.id);
    return html`<div class="disc-top__body">
        ${ui.cover(ar.id, 112, { round: true })}
        <div class="disc-top__info">
          <p class="disc-top__kind">artist</p>
          <h3 class="disc-top__title"><a href="#/artist/${ar.id}">${ar.name}</a></h3>
          ${ar.listeners != null ? html`<p class="disc-top__by">${thousands(ar.listeners)} listeners</p>` : ''}
          <div class="cluster disc-top__badges">${ar.tags.slice(0, 2).map(function (g) { return ui.tagChip(g); })}</div>
        </div>
      </div>
      <div class="disc-top__actions">
        <a class="btn btn--primary" href="#/artist/${ar.id}">Go to artist</a>
        <button class="btn" data-act="disc-radio" data-kind="artist" data-id="${ar.id}">${icon('radio')}Start radio</button>
        <button class="btn" data-act="follow" data-id="${ar.id}" aria-pressed="${on ? 'true' : 'false'}">${icon(on ? 'check' : 'follow')}${on ? 'Following' : 'Follow'}</button>
      </div>`;
  }
  function saveIssue(id) {
    var s = App.saves.get(id);
    if (!s || (s.status !== 'failed' && s.status !== 'pending')) return '';
    return html`<div class="disc-issue" role="status">${icon(s.status === 'failed' ? 'warning' : 'clock')}
      <span>${s.status === 'failed' ? 'Save stopped at step ' + s.step + ': ' + (s.error || M.saveSteps[s.step - 1]) : 'Saved to disk, waiting for Jellyfin'}</span>
      <button class="btn btn--quiet btn--sm" data-act="disc-save-sheet" data-id="${id}">${s.status === 'failed' ? 'Fix it' : 'Details'}</button></div>`;
  }
  function folderTile(a) {
    var n = trackIdsOf(a).length;
    return html`<div class="tile disc-folder-tile">
      <div class="tile__art"><button class="disc-tile-open" data-act="disc-folder" data-id="${a.id}" tabindex="-1" aria-hidden="true">${ui.cover(a, '100%')}</button>
        <button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><button data-act="disc-folder" data-id="${a.id}" aria-label="Open ${a.title} folder">${a.title}</button></div>
      <div class="tile__by">${a.artist}</div>
      <div class="tile__meta">${icon(a.saved === 'all' ? 'saved' : 'soulseek', a.saved === 'all' ? 'Saved' : 'On Soulseek', 'ic-16')}<span>${n ? plural(n, 'track') : a.type}${S.albums[a.id].folders.length > 1 ? ', ' + plural(S.albums[a.id].folders.length, 'folder') : ''}</span></div>
    </div>`;
  }
  /* someone's playlist on YouTube or Deezer: opens its preview (library.js #/outside/{key}) */
  function playlistFacts(p) {
    return App.outside.site(p.site) + (p.tracks ? ', ' + plural(p.tracks, 'song') : '') + (p.fans ? ', ' + thousands(p.fans) + ' fans' : '');
  }
  function playlistTile(p) {
    var href = App.outside.href(p.id);
    return html`<div class="tile disc-jump disc-playlist">
      <div class="tile__art"><a href="${href}" tabindex="-1" aria-hidden="true">${ui.mosaic([], '100%', '', p.cover)}</a></div>
      <div class="tile__title"><a href="${href}">${p.title}</a></div>
      <div class="tile__by">${p.owner ? p.owner + ', ' : ''}${playlistFacts(p)}</div>
    </div>`;
  }
  function artistTile(a) {
    return html`<div class="tile disc-jump disc-jump--round">
      <div class="tile__art"><a href="#/artist/${a.id}" tabindex="-1" aria-hidden="true">${ui.cover(a.id, '100%', { round: true })}</a>
        <button class="tile__play" data-act="disc-radio" data-kind="artist" data-id="${a.id}" aria-label="Start ${a.name} radio" data-tip="Start radio">${icon('radio')}</button></div>
      <div class="tile__title"><a href="#/artist/${a.id}">${a.name}</a></div>
      <div class="tile__by">${a.listeners != null ? thousands(a.listeners) + ' listeners' : 'artist'}</div>
    </div>`;
  }

  function addRecent(q) {
    q = String(q || '').trim(); if (q.length < 2) return;
    App.store.update('recentSearches', function (r) { var i = r.indexOf(q); if (i >= 0) r.splice(i, 1); r.unshift(q); r.length = Math.min(r.length, 8); });
  }

  /* phone: the header field is hidden, so the page has its own; it updates the results in place */
  function setQuery(q) {
    q = q.trim(); if (q === S.q) return;
    history.replaceState(history.state, '', '#/search' + (q ? '?q=' + encodeURIComponent(q) : ''));
    search(q);
    var f = $('[data-filters]'); if (f) f.hidden = !q;
    paint('sub', subline());
    paint('results', q ? resultsBody() : emptySearchBody());
  }
  function mountSearch(ctx) {
    var input = $('#disc-q'), t = null;
    if (input) {
      ctx.on(input, 'input', function () { clearTimeout(t); t = setTimeout(function () { setQuery(input.value); }, TYPE_PAUSE_MS); });
      ctx.on(input, 'keydown', function (e) {
        if (e.key === 'Enter') { clearTimeout(t); setQuery(input.value); startSoulseek(S.seq); input.blur(); }
        if (e.key === 'Escape' && input.value) { e.stopPropagation(); input.value = ''; setQuery(''); }
      });
    }
    /* Enter in the header's field submits too: Soulseek is asked at once */
    function onHeaderEnter(e) { if (e.key === 'Enter' && e.target.id === 'header-search') setTimeout(function () { startSoulseek(S.seq); }, 0); }
    document.addEventListener('keydown', onHeaderEnter);
    var offStore = App.on('store', function (keys) {
      if (keys.indexOf('services') >= 0 && S.q) paint('results', resultsBody());
      if (keys.indexOf('recentSearches') >= 0 && !S.q) paint('recents', recentsBody());
    });
    var offSave = App.on('save', function (d) { var el = $('[data-issue="' + d.id + '"]'); if (el) el.innerHTML = String(saveIssue(d.id)); });
    var offSettings = App.on('server:settings', function () { if (S.q) { M.reload('/api/settings'); } });
    ctx.onCleanup(function () { clearTimeout(t); document.removeEventListener('keydown', onHeaderEnter); offStore(); offSave(); offSettings(); });
  }

  /* search actions */
  App.action('disc-slsk-now', function () { startSoulseek(S.seq); paint('results', resultsBody()); });
  App.action('disc-search-again', function () {
    var q = S.q; S.q = null; search(q); startSoulseek(S.seq);
    paint('results', resultsBody()); App.announce('Searching again');
  });
  App.action('disc-clear', function () {
    var h = $('#header-search'); if (h) h.value = '';
    var i = $('#disc-q'); if (i) i.value = '';
    App.go('/search');
  });
  App.action('disc-show-all', function () {
    S.showAll = true; paint('results', resultsBody());
    var rows = $$('.disc-songs .trow'); if (rows[5]) rows[5].focus();
  });
  App.action('disc-only-all', function () {
    S.showAllOnly = true; paint('results', resultsBody());
    var rows = $$('.disc-only .trow'); if (rows[SONGS_MAX]) rows[SONGS_MAX].focus();
  });
  App.action('disc-recent-remove', function (el) {
    var q = el.getAttribute('data-q');
    App.store.update('recentSearches', function (r) { var i = r.indexOf(q); if (i >= 0) r.splice(i, 1); });
    App.toast('Removed ' + q + ' from recent searches', { icon: 'trash', action: 'Undo', onAction: function () { addRecent(q); } });
  });
  App.action('disc-recent-clear', function () {
    var before = (App.store.get('recentSearches') || []).slice();
    App.store.set('recentSearches', []);
    App.toast('Cleared recent searches', { icon: 'trash', action: 'Undo', onAction: function () { App.store.set('recentSearches', before); } });
  });

  /* =====================================================================
     3. Sheets: Soulseek folder, source picker, source details
     ===================================================================== */
  function sheetHead(t, sub) {
    if (!t) return '';
    return html`<div class="disc-shead">${ui.cover(t, 56)}<div class="disc-shead__main"><p class="disc-shead__title">${t.title}</p>
      <p class="disc-shead__by">${t.artist}, ${t.album}</p></div>${sub || ''}</div>`;
  }

  /* --- Soulseek folder (album result): the search row's peer folders, best first ---
     The open folder (module state, so the sheet's buttons can read it after the sheet closes):
     { albumId, trackIds, folders, idx: the folder shown } */
  var folder = null;
  function folderBody(a, fs) {
    var cur = fs.folders[fs.idx], ids = fs.trackIds, n = ids.length, savedN = ids.filter(App.isSaved).length;
    var by = [a.year, a.label].filter(Boolean).join(', '), missing = cur && cur.tracks < n ? n - cur.tracks : 0;
    return html`<div class="disc-folder">
      <div class="disc-folder__head">
        <div class="disc-folder__art">${ui.cover(a, 128)}</div>
        <div class="disc-folder__info">
          <p class="disc-top__kind">${a.type} folder${cur ? html` from <b>${cur.peer}</b>` : ''}</p>
          <h3 class="disc-folder__title">${a.title}</h3>
          <p class="disc-top__by"><a href="#/artist/${a.artistId}" data-close>${a.artist}</a>${by ? ', ' + by : ''}</p>
          ${cur ? html`<div class="cluster disc-folder__facts">${ui.qualityChip(cur.format)}<span class="px">${plural(cur.tracks, 'track')}</span>${cur.sizeBytes ? html`<span class="px">${size(cur.sizeBytes / 1e6)}</span>` : ''}
            <span class="px">${mbps(cur.speed)}</span><span class="disc-folder__slot">${icon(cur.freeSlot ? 'check' : 'clock', null, 'ic-16')}${cur.freeSlot ? 'free slot' : 'queue of ' + cur.queue}</span></div>` : ''}
          <div class="disc-top__actions">
            <button class="btn btn--primary" data-act="disc-folder-play" data-close autofocus ${n ? '' : raw('disabled')}>${icon('play')}Play album</button>
            ${n && savedN === n ? ui.savedBadge() : html`<button class="btn" data-act="save-album" data-id="${a.id}">${icon('save')}${savedN ? 'Save the other ' + (n - savedN) : 'Save whole album'}</button>`}
            ${ui.iconButton('radio', 'Start radio from ' + a.title, { 'data-act': 'disc-radio', 'data-kind': 'album', 'data-id': a.id, 'data-close': '' })}
            <a class="btn btn--quiet" href="#/album/${a.id}" data-close>Album page</a>
          </div>
        </div>
      </div>
      <div data-region="agg">${aggInline(ids, a.title)}</div>
      ${missing ? html`<p class="disc-issue">${icon('info')}<span>This folder is missing ${plural(missing, 'track')}. Missing songs are fetched from other peers when you play or save them.</span></p>` : ''}
      ${ui.labelRule('tracks', { level: 'h3', sm: true })}
      ${ui.trackList(ids, { dense: true, label: a.title + ' tracks' })}
      ${ui.labelRule('other folders', { level: 'h3', sm: true })}
      ${fs.folders.length > 1 ? html`<ul class="disc-alts">${fs.folders.map(function (f, i) {
        if (i === fs.idx) return '';
        return html`<li class="disc-alt">
          <span class="disc-alt__peer">${icon('peer')}<b>${f.peer}</b></span>${ui.qualityChip(f.format)}
          <span class="px disc-alt__fact">${f.tracks}/${n}</span><span class="px disc-alt__fact">${f.sizeBytes ? size(f.sizeBytes / 1e6) : ''}</span><span class="px disc-alt__fact">${mbps(f.speed)}</span>
          <span class="score" style="--s:${f.score / 100}" role="img" aria-label="Score ${f.score} of 100"><i></i></span>
          <button class="btn btn--sm" data-act="disc-folder-show" data-i="${i}" aria-label="Show ${f.peer}'s folder">Show</button></li>`;
      })}</ul>` : html`<p class="disc-note">No other peer shares this album.</p>`}
    </div>`;
  }
  function openFolder(albumId) {
    var a = M.album(albumId), row = S.albums[albumId]; if (!a || !row) return null;
    var fs = folder = { albumId: albumId, trackIds: trackIdsOf(a), folders: row.folders || [], idx: 0 };
    var off = null;
    var sheet = App.openSheet({ title: 'soulseek folder', wide: true, className: 'disc-sheet', body: html`<div data-region="folder">${folderBody(a, fs)}</div>`,
      onMount: function (el) {
        off = App.on('save', function (d) { if (fs.trackIds.indexOf(d.id) >= 0) paint('agg', aggInline(fs.trackIds, a.title), el); });
      },
      onClose: function () { if (off) off(); } });
    App.syncRows();
    return sheet;
  }
  /* rows name a folder, not each song's file, so the player picks each song's peer itself */
  function playFolder(fs) { if (fs.trackIds.length) P.playList(fs.trackIds, 0); }
  App.action('disc-folder', function (el) { openFolder(el.getAttribute('data-id')); });
  /* shows another peer's folder facts; playing still lets the player pick each song's peer (see playFolder) */
  App.action('disc-folder-show', function (el) {
    var fs = folder, f = fs && fs.folders[+el.getAttribute('data-i')]; if (!f) return;
    fs.idx = fs.folders.indexOf(f);
    paint('folder', folderBody(M.album(fs.albumId), fs));
  });
  App.action('disc-folder-play', function () { if (folder) playFolder(folder); });

  /* --- source picker ---
     The open picker (module state, read by its buttons): { trackId, sources, done, error, sort, stop } */
  var picker = null;
  function sortedSources(t, pk) {
    var rows = pk.sources.map(function (s, i) { return { s: s, i: i }; });
    var by = {
      score: function (a, b) { return b.s.score - a.s.score; },
      quality: function (a, b) { return quality(b.s.format) - quality(a.s.format) || b.s.score - a.s.score; },
      speed: function (a, b) { return b.s.speed - a.s.speed; },
      size: function (a, b) { return (a.s.sizeBytes || 0) - (b.s.sizeBytes || 0); }
    }[pk.sort];
    return rows.sort(by);
  }
  function srcRow(t, s, i, failed, first) {
    var f = fmt(s.format), x = s.factors;
    /* what the profile says about the copy: below its minimum, or a peer queue longer than the limit (never auto-picked) */
    var note = [failed, s.meets === false ? 'below minimum' : '', s.queueTooLong ? 'long queue' : ''].filter(Boolean).join(', ');
    var tip = x ? 'format ' + x.format + ' + speed ' + x.speed + ' + free slot ' + x.slot + ' + queue ' + x.queue +
      (x.stalled ? ' - stalled ' + Math.abs(x.stalled) : '') + ' = ' + s.score : 'score ' + s.score;
    return html`<div class="src-row${failed ? ' is-failed' : ''}${s.meets === false ? ' is-below' : ''}${s.preferred ? ' is-preferred' : ''}" role="row">
      <span class="disc-src__peer" role="cell"><button class="src-row__peer" data-act="disc-use-source" data-id="${t.id}" data-i="${i}" ${first ? raw('autofocus') : ''}
        aria-label="Play from ${s.peer}, ${f.label}, score ${s.score}${note ? ', ' + note : ''}${s.preferred ? ', preferred format' : ''}">${icon('peer')}<span class="ellipsis">${s.peer}</span></button>
        ${note ? html`<span class="disc-src__note">${note}</span>` : ''}</span>
      <span role="cell">${ui.qualityChip(s.format)}</span>
      <span class="src-row__opt" role="cell">${s.sizeBytes ? size(s.sizeBytes / 1e6) : ''}</span>
      <span class="src-row__opt" role="cell">${mbps(s.speed)}</span>
      <span class="src-row__opt" role="cell">${icon(s.freeSlot ? 'check' : 'close', s.freeSlot ? 'Free slot' : 'No free slot')}</span>
      <span class="src-row__opt" role="cell">${s.queue}</span>
      <span class="disc-src__score" role="cell" tabindex="0" data-tip="${tip}" aria-label="Score ${s.score} of 100: ${tip}"><span class="score" style="--s:${(s.score / 100).toFixed(2)}"><i></i></span><span class="px">${s.score}</span></span>
    </div>`;
  }
  function pickerRows(t, pk) {
    if (!pk.sources) {
      return pk.error ? ui.emptyState({ art: 'star', title: 'Couldn\'t load sources', text: pk.error.message, action: { label: 'Try again', act: 'disc-src-retry' } })
        : ui.skeleton('row', 3);
    }
    if (!pk.sources.length && pk.done) {
      return ui.emptyState({ art: 'moon', title: 'No peer shares this right now', text: 'Soulseek peers come and go. Ask again in a while.',
        action: { label: 'Search again', act: 'disc-src-retry' } });
    }
    var notes = failedNotes(t.id, pk.sources);
    return html`<div class="src-table disc-src" role="table" aria-label="Sources for ${t.title}">
      <div class="src-row src-row--head" role="row"><span role="columnheader">peer</span><span role="columnheader">quality</span>
        <span class="src-row__opt" role="columnheader">size</span><span class="src-row__opt" role="columnheader">speed</span>
        <span class="src-row__opt" role="columnheader">slot</span><span class="src-row__opt" role="columnheader">queue</span><span role="columnheader">score</span></div>
      ${sortedSources(t, pk).map(function (r, n) { return srcRow(t, r.s, r.i, notes[r.s.peer], n === 0); })}
      ${pk.error ? html`<div class="src-row disc-src__more" role="row"><span role="cell">${icon('warning', null, 'ic-16')}Stopped asking peers: ${pk.error.message}
          <button class="btn btn--quiet btn--sm" data-act="disc-src-retry">Ask again</button></span></div>`
        : !pk.done ? html`<div class="src-row disc-src__more" role="row"><span role="cell"><span class="spin-ring" aria-hidden="true"></span>Still searching: more peers may answer</span></div>` : ''}
    </div>`;
  }
  function pickerMeta(pk) {
    if (!pk.sources) return html`<span class="px">asking peers</span>`;
    var lossless = pk.sources.filter(function (s) { return fmt(s.format).lossless; }).length;
    return html`<span class="px">${plural(pk.sources.length, 'source')}</span><span>${lossless} lossless</span>`;
  }
  function pickerBody(t, pk) {
    return html`${sheetHead(t, html`<span class="disc-shead__meta" data-region="srcmeta">${pickerMeta(pk)}</span>`)}
      ${pk.choose ? html`<p class="disc-issue disc-picker__choose" role="status">${icon('warning')}<span>No copy meets your streaming minimum. Pick one to play it anyway, or skip this song.
        <a href="#/settings/sources">Streaming rules</a></span></p>` : ''}
      <div class="disc-picker__bar">
        <span class="disc-picker__label" id="disc-sort-l">Sort by</span>
        <div class="cluster" role="group" aria-labelledby="disc-sort-l">${['score', 'quality', 'speed', 'size'].map(function (k) {
          return ui.filterPill(k.charAt(0).toUpperCase() + k.slice(1), { act: 'disc-src-sort', value: k, pressed: pk.sort === k });
        })}</div>
      </div>
      <div data-region="srcrows">${pickerRows(t, pk)}</div>
      <p class="disc-note disc-picker__hint">Score weighs format, peer speed, a free upload slot and queue length. Hover or focus a score for the breakdown.</p>`;
  }
  /* o.forSave: the pick answers a failed save's transfer (the job continues from that peer) instead of playing.
     o.choose: the player is waiting for this pick (streaming `below: ask`), so the sheet offers Skip too */
  function openPicker(trackId, o) {
    var t = M.track(trackId); if (!t) return null;
    var forSave = !!(o && o.forSave), choose = !!(o && o.choose);
    var pk = picker = { trackId: trackId, forSave: forSave, choose: choose, sources: null, done: false, error: null, sort: 'score', stop: null, closed: false };
    var sheet;
    function repaint() {
      if (pk.closed) return;
      paint('srcmeta', pickerMeta(pk), sheet.el);
      paint('srcrows', pickerRows(t, pk), sheet.el);
    }
    pk.watch = function () {
      if (pk.stop) pk.stop();
      pk.error = null; repaint();
      pk.stop = watchSources(trackId, function (res) {
        var wasDone = pk.done;
        if (res.error) pk.error = res.error; else { pk.sources = res.sources; pk.done = res.done; }
        repaint();
        if (pk.done && !wasDone) App.announce('Search finished: ' + plural(pk.sources.length, 'source'));
      }, forSave);
    };
    sheet = App.openSheet({ title: 'pick a source', wide: true, className: 'disc-sheet disc-picker', body: pickerBody(t, pk),
      foot: html`${choose ? html`<button class="btn" data-act="disc-choose-skip" data-close>${icon('next')}Skip this song</button>` : html`<button class="btn" data-close>Cancel</button>`}<button class="btn btn--primary" data-act="disc-play-best" data-id="${t.id}" data-close>${forSave ? html`${icon('refresh')}Retry with the best source` : html`${icon('play')}Play best source`}</button>`,
      onClose: function () { pk.closed = true; if (pk.stop) pk.stop(); } });
    pk.watch();
    return sheet;
  }
  App.hooks.sourcePicker = function (trackId, o) { openPicker(trackId, o); };
  App.action('disc-choose-skip', function () { P.next(); });
  App.action('disc-src-sort', function (el) {
    var pk = picker; if (!pk || pk.closed) return;
    pk.sort = el.getAttribute('data-value');
    $$('[data-act="disc-src-sort"]').forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-value') === pk.sort)); });
    paint('srcrows', pickerRows(M.track(pk.trackId), pk));
  });
  App.action('disc-src-retry', function () { if (picker && !picker.closed) { picker.done = false; picker.watch(); } });
  /* a picked source plays, or (picker opened for a failed save) restarts that save's transfer from it */
  function useSource(pk, s, note) {
    if (pk.forSave) {
      App.saves.retry(pk.trackId, { peer: s.peer, path: s.path, size: s.sizeBytes });
      App.toast('Retrying the save from ' + s.peer + ', ' + fmt(s.format).label, { icon: 'soulseek' });
      return;
    }
    P.useSource(pk.trackId, sourceFor(s));
    App.toast((note ? 'Trying ' + s.peer + ' again, ' : 'Playing from ' + s.peer + ', ') + fmt(s.format).label, { icon: 'soulseek' });
  }
  App.action('disc-use-source', function (el) {
    var pk = picker, s = pk && pk.sources && pk.sources[+el.getAttribute('data-i')]; if (!s) return;
    var note = failedNotes(pk.trackId, pk.sources)[s.peer];
    App.closeTop(); useSource(pk, s, note);
  });
  /* the best source that hasn't failed; before any source has answered, the player (or the save job) picks on its own */
  App.action('disc-play-best', function (el) {
    var id = el.getAttribute('data-id'), pk = picker && picker.trackId === id ? picker : null;
    if (!pk || !pk.sources || !pk.sources.length) { if (pk && pk.forSave) App.saves.retry(id); else P.playTrack(id); return; }
    var notes = failedNotes(id, pk.sources);
    useSource(pk, pk.sources.filter(function (s) { return !notes[s.peer]; })[0] || pk.sources[0]);
  });
  App.action('disc-swap-picker', function (el) { var id = el.getAttribute('data-id'); App.closeTop(); openPicker(id); });

  /* --- source details (the playing item) --- */
  function xferState() {
    var ps = P.state;
    if (ps.status === 'library') return ['ok', 'from your library'];
    if (ps.status === 'finding') return ['warn', 'looking for peers'];
    if (ps.status === 'queued') return ['warn', 'queued at peer, position ' + ps.queuePos];
    if (ps.status === 'stalled') return ['down', 'stalled'];
    if (ps.status === 'failed') return ['down', 'failed'];
    return ps.received >= 1 ? ['ok', 'complete'] : ['ok', 'transferring'];
  }
  function detailsBody() {
    var t = P.current(), ps = P.state;
    if (!t) return ui.emptyState({ art: 'slot', title: 'Nothing playing', text: 'Play something and its source shows up here.', action: { label: 'Go home', href: '#/home' } });
    var save = App.saves.get(t.id), st = xferState();
    if (ps.status === 'library' || !ps.peer) {
      var f0 = fmt(t.format);
      return html`${sheetHead(t)}<div class="source-card">
          <div class="source-card__head">${icon('saved')}<span>Playing from <b>your library</b> via Jellyfin</span>${ui.qualityChip(t.format, { solid: true })}</div></div>
        <dl class="disc-kv">
          <div><dt>Format</dt><dd>${f0.codec}${detail(f0) ? ', ' + detail(f0) : ''}</dd></div>
          ${t.kbps || f0.kbps ? html`<div><dt>Bitrate</dt><dd class="px">${t.kbps || f0.kbps} kbps</dd></div>` : ''}
          ${t.sizeMB ? html`<div><dt>Size</dt><dd class="px">${size(t.sizeMB)}</dd></div>` : ''}
          <div><dt>State</dt><dd>${ui.statusGlyph('ok')} saved, plays at once</dd></div>
        </dl>`;
    }
    var peer = ps.peer, f = fmt(peer.format || t.format), total = peer.sizeMB || t.sizeMB, kbps = f.kbps || t.kbps;
    var path = ps.source && ps.source.peer === peer.name ? ps.source.path : null;
    return html`${sheetHead(t)}
      <div class="source-card">
        <div class="source-card__head">${icon('soulseek')}<span>Streaming from <b>${peer.name}</b></span>${ui.qualityChip(f.label, { solid: true })}
          <span class="source-card__stat" data-d="speed">${mbps(peer.speed)}</span><span class="source-card__stat" data-d="recv">${Math.round(ps.received * 100)}%${total ? ' of ' + size(total) : ''}</span></div>
        ${save ? ui.pipeline(save.step, save.status === 'pending' ? 'saving' : save.status, save.error) : ''}
      </div>
      <dl class="disc-kv">
        <div><dt>Peer</dt><dd><b>${peer.name}</b>, source ${peer.rank}${peer.of ? ' of ' + peer.of : ''}</dd></div>
        <div><dt>Format</dt><dd>${f.codec}${detail(f) ? ', ' + detail(f) : ''}</dd></div>
        ${kbps ? html`<div><dt>Bitrate</dt><dd class="px">${kbps} kbps</dd></div>` : ''}
        ${total ? html`<div><dt>Size</dt><dd class="px">${size(total)}</dd></div>` : ''}
        <div><dt>Speed</dt><dd class="px" data-d="speed">${mbps(peer.speed)}</dd></div>
        ${total ? html`<div><dt>Received</dt><dd class="px" data-d="bytes">${size(total * ps.received)} of ${size(total)}</dd></div>` : ''}
        <div><dt>State</dt><dd data-d="state">${ui.statusGlyph(st[0])} ${st[1]}</dd></div>
        ${path ? html`<div class="disc-kv__wide"><dt>File</dt><dd class="disc-kv__path">${path}</dd></div>` : ''}
      </dl>
      ${ui.labelRule('fallback history', { level: 'h3', sm: true })}
      ${ps.failedPeers.length ? html`<ol class="disc-fallback">${ps.failedPeers.map(function (fp) {
          return html`<li>${ui.statusGlyph('down')}<span><b>${fp.peer}</b> stalled${fp.at != null ? ' at ' + fp.at + '%' : ''}, switched to the next source</span></li>`;
        })}<li>${ui.statusGlyph('ok')}<span>Now from <b>${peer.name}</b>, kept your position</span></li></ol>`
        : html`<p class="disc-note">No fallbacks. The first peer has held up.</p>`}`;
  }
  function openDetails() {
    var t = P.current(), offTick = null, offPlayer = null;
    var sh = App.openSheet({ title: 'source details', className: 'disc-sheet', body: html`<div data-region="details">${detailsBody()}</div>`,
      foot: t ? html`<button class="btn" data-act="disc-swap-picker" data-id="${t.id}">${icon('soulseek')}Pick another source</button>${ui.saveButton(t.id)}` : '',
      onMount: function (el) {
        offTick = App.on('tick', function () {
          var ps = P.state, peer = ps.peer, cur = P.current(); if (!peer || !cur) return;
          var total = peer.sizeMB || cur.sizeMB, stt = xferState();
          $$('[data-d="speed"]', el).forEach(function (n) { App.setText(n, mbps(peer.speed)); });
          $$('[data-d="recv"]', el).forEach(function (n) { App.setText(n, Math.round(ps.received * 100) + '%' + (total ? ' of ' + size(total) : '')); });
          if (total) $$('[data-d="bytes"]', el).forEach(function (n) { App.setText(n, size(total * ps.received) + ' of ' + size(total)); });
          $$('[data-d="state"]', el).forEach(function (n) { if (n.getAttribute('data-was') !== stt[1]) { n.setAttribute('data-was', stt[1]); n.innerHTML = String(html`${ui.statusGlyph(stt[0])} ${stt[1]}`); } });
        });
        offPlayer = App.on('player', function (k) { if (k === 'track' || k === 'source') paint('details', detailsBody(), el); });
      },
      onClose: function () { if (offTick) offTick(); if (offPlayer) offPlayer(); } });
    return sh;
  }
  App.hooks.sourceDetails = function () { openDetails(); };

  /* =====================================================================
     4. Save flow: duplicate dialog, save sheet (steps + recovery), release picker, aggregate saves
     ===================================================================== */
  /* one song: ask the library whether a copy is already saved (POST /api/library/duplicates). Album and queue
     saves skip the per-song check (their progress shows as an aggregate). */
  App.hooks.beforeSave = function (ids, proceed) {
    if (ids.length !== 1) { proceed(ids); return; }
    var id = ids[0];
    savedCopy(id, playingSource(id)).then(function (d) {
      if (!d) { proceed(ids); return; }
      openDuplicate(d.existing, candidate(id), d.better, function (choice) {
        if (choice === 'keep') proceed([]); else proceed(ids, { mode: choice });
      });
    }, function (err) {
      App.toast('Couldn\'t check your library for a saved copy (' + err.message + '). Saving anyway.', { kind: 'error' });
      proceed(ids);
    });
  };
  /* the saved copy of one song, or null; `source` ({ peer, path, size }) is the offered file `better` compares against */
  function savedCopy(id, source) {
    var body = { ids: [id] };
    if (source) { body.source = {}; body.source[id] = source; }
    return M.api('/api/library/duplicates', { method: 'POST', body: body }).then(function (res) {
      return (res.items || []).filter(function (x) { return x.trackId === id && x.existing; })[0] || null;
    });
  }
  /* the file a save of this song would take: the manually picked peer's, when it is the one playing */
  function playingSource(id) {
    var src = P.currentId() === id && P.state.source;
    return src ? { peer: src.peer, path: src.path, size: src.sizeBytes } : null;
  }
  /* the copy about to be saved: the playing peer's file when this song is playing, else the song's own facts */
  function candidate(id) {
    var t = M.track(id) || {}, ps = P.state, playing = P.currentId() === id && ps.peer;
    return { format: (playing && ps.peer.format) || t.format, sizeMB: (playing && ps.peer.sizeMB) || t.sizeMB, kbps: t.kbps,
      peer: playing ? ps.peer.name : null, speed: playing ? ps.peer.speed : null };
  }

  var dupDone = null;
  /* existing: the saved Track; cand: { format, sizeMB, kbps, peer, speed }; onChoice('keep' | 'separate' | 'replace') */
  function openDuplicate(existing, cand, better, onChoice) {
    function card(label, c, sub, isBest, note) {
      var f = fmt(c.format), kbps = c.kbps || f.kbps;
      return html`<div class="disc-dup__card${isBest ? ' is-better' : ''}">
        <p class="disc-dup__label">${label}</p>
        <div class="disc-dup__row">${ui.cover(existing, 56)}<div>${ui.qualityChip(c.format)}<p class="disc-dup__sub">${sub}</p></div></div>
        <dl class="disc-dup__facts"><div><dt>Detail</dt><dd>${detail(f)}</dd></div><div><dt>Bitrate</dt><dd class="px">${kbps ? kbps + ' kbps' : ''}</dd></div><div><dt>Size</dt><dd class="px">${c.sizeMB ? size(c.sizeMB) : ''}</dd></div></dl>
        ${isBest ? html`<p class="disc-dup__best">${icon('arrow-up')}better quality</p>` : note ? html`<p class="disc-dup__keep">${icon('saved')}${note}</p>` : ''}
      </div>`;
    }
    dupDone = { existing: existing, cand: cand, onChoice: onChoice };
    return App.openDialog({ title: 'already in your library', wide: true, className: 'disc-dup-dialog',
      body: html`<p class="disc-dup__lead"><b>${existing.artist}, ${existing.title}</b> is already saved as ${fmt(existing.format).label}. What should happen to this copy?</p>
        <div class="disc-dup">${card('in your library', existing, 'Saved, plays from Jellyfin', false, better ? '' : 'same or better')}
          ${card('from ' + (cand.peer || 'Soulseek'), cand, 'On Soulseek' + (cand.speed ? ', ' + mbps(cand.speed) : ''), better)}</div>
        <p class="disc-note">Set what happens by default in <a class="link" href="#/settings/library" data-close>Settings, Library</a>.</p>`,
      foot: html`<button class="btn" data-act="disc-dup" data-choice="separate" data-close>Save as a separate version</button>
        ${better ? html`<button class="btn" data-act="disc-dup" data-choice="replace" data-close>${icon('arrow-up')}Replace with better quality</button>` : ''}
        <button class="btn btn--primary" data-act="disc-dup" data-choice="keep" data-close autofocus>Keep existing</button>` });
  }
  App.action('disc-dup', function (el) {
    var d = dupDone; dupDone = null; if (!d) return;
    var choice = el.getAttribute('data-choice'), label = fmt(d.cand.format).label, title = d.existing.title;
    if (choice === 'keep') App.toast('Kept the saved copy of ' + title, { icon: 'saved' });
    else App.toast(choice === 'replace' ? 'Replacing ' + title + ' with ' + label : 'Saving a second copy of ' + title + ', ' + label, { icon: 'save' });
    d.onChoice(choice);
  });
  /* a saved song that peers share in better quality: offer the best source the server ranks, compared by the library */
  App.action('disc-save-version', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id); if (!t) return;
    var fail = function (err) { App.toast('Couldn\'t offer a better copy of ' + t.title + ': ' + err.message, { kind: 'error' }); };
    var row = S.peerSongs[id];
    (row && row.best ? Promise.resolve(row.best) : M.api(sourcesPath(id, true)).then(function (res) { return (res.sources || [])[0]; })).then(function (best) {
      if (!best) { App.toast('No peer shares ' + t.title + ' right now', { icon: 'no-source' }); return; }
      var offered = { peer: best.peer, path: best.path, size: best.sizeBytes };
      savedCopy(id, offered).then(function (d) {
        var cand = { format: best.format, sizeMB: best.sizeBytes ? best.sizeBytes / 1e6 : null, peer: best.peer, speed: best.speed };
        openDuplicate(d ? d.existing : t, cand, !!(d && d.better), function (choice) {
          if (choice === 'keep') return;
          var source = {}; source[id] = offered;
          App.saves.start([id], { mode: choice, source: source });   // with a mode, start() doesn't ask again
        });
      }, fail);
    }, fail);
  });

  /* --- save sheet: the 5 steps, the error, and what to do about it --- */
  var RECOVER = {
    transfer: function (id, s) {
      return { text: s.error || 'The transfer stopped before the file was complete.', actions: html`
        <button class="btn btn--primary" data-act="disc-save-retry" data-id="${id}">${icon('refresh')}Retry the transfer</button>
        <button class="btn" data-act="disc-save-pick" data-id="${id}">${icon('soulseek')}Pick a source</button>` };
    },
    match: function (id, s) {
      return { text: s.error || 'MusicBrainz found several releases and none is a sure fit.', actions: html`
        <button class="btn btn--primary" data-act="disc-release" data-id="${id}">Pick the right release</button>
        <button class="btn" data-act="disc-rel-skip" data-id="${id}">Save with existing tags</button>` };
    },
    write: function (id, s) {
      return { text: s.error || 'Couldn\'t write the file to your library.', actions: html`
        <button class="btn btn--primary" data-act="disc-save-retry" data-id="${id}">${icon('refresh')}Retry</button>
        <a class="btn" href="#/settings/library" data-close>Library settings</a>` };
    }
  };
  function failKind(s) { return RECOVER[s.kind] ? s.kind : s.step === 1 ? 'transfer' : s.step === 2 ? 'match' : 'write'; }
  function saveBody(id) {
    var t = M.track(id), s = App.saves.get(id), saved = App.isSaved(id);
    if (!s && !saved) {
      return html`${sheetHead(t)}<p class="disc-note">Not saved yet. Saving finishes the transfer, matches the song on MusicBrainz, fetches cover art, tags the file and asks Jellyfin to pick it up.</p>`;
    }
    var step = s ? s.step : 5, status = s ? s.status : 'saved';
    var steps = M.saveSteps.map(function (label, i) {
      var n = i + 1, cls = status === 'saved' || n < step ? 'is-done' : n === step ? (status === 'failed' ? 'is-failed' : status === 'pending' ? 'is-wait' : 'is-current') : '';
      var note = n === 1 && cls === 'is-current' && s ? (s.pct || 0) + '%' : n === 3 && s && s.nocover && n <= step ? 'none found, saving without one' : n === 5 && status === 'pending' ? (s.error || 'waiting for Jellyfin') : '';
      var ic = cls === 'is-done' ? (n === 3 && s && s.nocover ? 'dash' : 'check') : cls === 'is-failed' ? 'close' : cls === 'is-wait' ? 'clock' : cls === 'is-current' ? null : 'dash';
      return html`<li class="disc-step ${cls}${n === 3 && s && s.nocover && n < step ? ' is-skipped' : ''}">
        <span class="disc-step__mark" aria-hidden="true">${ic ? icon(ic, null, 'ic-16') : raw('<span class="spin-ring"></span>')}</span>
        <span class="disc-step__label">${label}</span>${note ? html`<span class="disc-step__note">${note}</span>` : ''}
        <span class="vh">${cls === 'is-done' ? 'done' : cls === 'is-failed' ? 'failed' : cls === 'is-current' ? 'in progress' : cls === 'is-wait' ? 'waiting' : 'not started'}</span></li>`;
    });
    var rec = status === 'failed' ? RECOVER[failKind(s)](id, s) : null;
    return html`${sheetHead(t, ui.saveButton(id))}
      <div class="source-card">${ui.pipeline(step, status === 'pending' ? 'saving' : status, s && s.error)}</div>
      <ol class="disc-steps">${steps}</ol>
      ${rec ? html`<div class="disc-recover" role="alert">${ui.statusGlyph('down', 'Failed')}<div><p class="disc-recover__text">${rec.text}</p><div class="disc-recover__actions">${rec.actions}</div></div></div>` : ''}
      ${status === 'pending' ? html`<div class="disc-recover is-wait">${icon('clock')}<div><p class="disc-recover__text">Saved to disk. It will appear in your library when Jellyfin is back.</p>
        <div class="disc-recover__actions"><button class="btn" data-act="disc-rescan">${icon('refresh')}Retry rescan now</button></div></div></div>` : ''}
      ${status === 'saving' ? html`<p class="disc-note">You can close this. Saving carries on, and Activity shows its progress.</p>` : ''}
      ${status === 'saved' ? html`<p class="disc-note">${t ? t.title : 'This song'} is in your library. Its badge now reads Saved everywhere.</p>` : ''}`;
  }
  function openSaveSheet(id) {
    var off = null;
    return App.openSheet({ title: 'saving to your library', className: 'disc-sheet', body: html`<div data-region="save">${saveBody(id)}</div>`,
      foot: html`<button class="btn" data-act="open-activity" data-close>${icon('activity')}Open Activity</button><button class="btn" data-close>Done</button>`,
      onMount: function (el) { off = App.on('save', function (d) { if (d.id === id) paint('save', saveBody(id), el); }); },
      onClose: function () { if (off) off(); } });
  }
  App.action('disc-save-sheet', function (el) { openSaveSheet(el.getAttribute('data-id')); });
  App.action('disc-save-retry', function (el) { App.saves.retry(el.getAttribute('data-id')); });
  App.action('disc-save-pick', function (el) { openPicker(el.getAttribute('data-id'), { forSave: true }); });
  /* the file is on disk; Jellyfin only needs to pick it up (progress arrives as `save` events) */
  App.action('disc-rescan', function () {
    M.api('/api/library/rescan', { method: 'POST' }).then(function () { App.toast('Asked Jellyfin to rescan', { icon: 'refresh' }); },
      function (err) { App.toast('Couldn\'t ask Jellyfin to rescan: ' + err.message, { kind: 'error' }); });
  });

  /* --- MusicBrainz release picker (step 2 had no confident match): GET/POST /api/saves/{jobId}/release(s) ---
     The open picker (module state, read by its buttons): { trackId, jobId, items, file, error, el }; `file` is the
     server's `track`: what lull knows of the file being saved */
  var rel = null;
  function jobOf(id) { var s = App.saves.get(id); return s && s.jobId; }
  function relBody(r) {
    var t = M.track(r.trackId);
    if (r.error) return ui.emptyState({ art: 'star', title: 'Couldn\'t load the releases', text: r.error.message, action: { label: 'Try again', act: 'disc-rel-reload' } });
    if (!r.items) return ui.skeleton('row', 3);
    if (!r.items.length) return html`<p class="disc-dup__lead">MusicBrainz has no release that holds <b>${t.title}</b>. Save it with the file's own tags.</p>`;
    return html`<p class="disc-dup__lead">MusicBrainz found ${r.items.length} releases that could hold <b>${t.title}</b> by ${t.artist}. Pick the one your file came from.</p>
      <fieldset class="disc-rel"><legend class="vh">Releases</legend>
        <div class="disc-rel__head" aria-hidden="true"><span></span><span>release</span><span>date</span><span>country</span><span>tracks</span></div>
        ${r.items.map(function (x, i) {
          return html`<label class="disc-rel__row"><input type="radio" name="disc-rel" value="${x.mbid}" ${i === 0 ? raw('checked') : ''}>
            <span class="disc-rel__title">${x.title}<small>${[x.kind, x.artist].filter(Boolean).join(', ')}</small></span>
            <span class="px disc-rel__date">${x.date || ''}</span><span class="disc-rel__country">${x.country || ''}</span><span class="px disc-rel__n">${x.tracks}<span class="vh"> tracks</span></span></label>`;
        })}
      </fieldset>
      ${fileNote(r.file || t)}`;
  }
  function fileNote(f) {
    var facts = [f.duration ? App.fmt.time(f.duration) : '', f.format ? fmt(f.format).label : '', f.no ? 'track ' + f.no : '', f.album ? 'tagged "' + f.album + '"' : ''];
    return html`<p class="disc-note">Your file: ${facts.filter(Boolean).join(', ')}.</p>`;
  }
  function loadReleases(r) {
    r.error = null; r.items = null; paint('rels', relBody(r), r.el);
    M.api('/api/saves/' + encodeURIComponent(r.jobId) + '/releases').then(function (res) {
      r.items = res.items || []; r.file = res.track || null; paint('rels', relBody(r), r.el);
    }, function (err) { r.error = err; paint('rels', relBody(r), r.el); });
  }
  function openReleasePicker(id) {
    var t = M.track(id), jobId = jobOf(id);
    if (!t || !jobId) { App.toast('This song has no save waiting for a release', { kind: 'error' }); return null; }
    var r = rel = { trackId: id, jobId: jobId, items: null, file: null, error: null, el: null };
    var dlg = App.openDialog({ title: 'pick the right release', wide: true, className: 'disc-rel-dialog',
      body: html`<div data-region="rels">${relBody(r)}</div>`,
      foot: html`<button class="btn" data-act="disc-rel-skip" data-id="${id}" data-close>Save with existing tags</button>
        <button class="btn btn--primary" data-act="disc-rel-use" data-id="${id}" data-close>Use this release</button>` });
    r.el = dlg.el;
    loadReleases(r);
    return dlg;
  }
  /* POST the choice; the save carries on and reports through `save` events */
  function chooseRelease(id, body, ok) {
    var jobId = jobOf(id);
    if (!jobId) { App.toast('This song has no save waiting for a release', { kind: 'error' }); return; }
    M.api('/api/saves/' + encodeURIComponent(jobId) + '/release', { method: 'POST', body: body }).then(function () { App.toast(ok, { icon: 'check', kind: 'ok' }); },
      function (err) { App.toast('Couldn\'t use that release: ' + err.message, { kind: 'error' }); });
  }
  App.action('disc-release', function (el) { openReleasePicker(el.getAttribute('data-id')); });
  App.action('disc-rel-reload', function () { if (rel) loadReleases(rel); });
  App.action('disc-rel-use', function (el) {
    var id = el.getAttribute('data-id'), pickEl = $('input[name="disc-rel"]:checked');
    var r = rel && rel.items && pickEl ? rel.items.filter(function (x) { return x.mbid === pickEl.value; })[0] : null;
    if (!r) { App.toast('Pick a release first', { icon: 'info' }); return; }
    chooseRelease(id, { mbid: r.mbid }, 'Matched to ' + r.title + (r.date ? ' (' + String(r.date).slice(0, 4) + ')' : '') + '. Saving carries on.');
  });
  App.action('disc-rel-skip', function (el) { chooseRelease(el.getAttribute('data-id'), { keepTags: true }, 'Saving with the file\'s own tags'); });

  /* --- aggregate saves: whole album, the queue --- */
  function aggCounts(ids) {
    var c = { saved: 0, failed: 0, saving: 0, pending: 0, total: ids.length };
    ids.forEach(function (id) {
      var s = App.saves.get(id);
      if (App.isSaved(id)) c.saved++; else if (s && s.status === 'failed') c.failed++; else if (s && s.status === 'pending') c.pending++; else if (s) c.saving++;
    });
    return c;
  }
  function aggText(c) {
    return c.saved + ' of ' + c.total + ' saved' + (c.failed ? ', ' + c.failed + ' need' + (c.failed === 1 ? 's' : '') + ' attention' : '') + (c.pending ? ', ' + c.pending + ' waiting for Jellyfin' : '');
  }
  function aggInline(ids, label) {
    var c = aggCounts(ids);
    if (!c.failed && !c.saving && !c.pending) return '';
    return html`<div class="disc-agg">
      <div class="disc-agg__top">${icon(c.failed ? 'warning' : 'save')}<span class="disc-agg__text" aria-live="polite">${aggText(c)}</span>
        <button class="btn btn--quiet btn--sm" data-act="disc-agg-open" data-ids="${ids.join(',')}" data-label="${label}">Details</button></div>
      <div class="progress${c.failed ? '' : ' progress--xfer'}" role="progressbar" aria-label="Saving ${label}" aria-valuemin="0" aria-valuemax="${c.total}" aria-valuenow="${c.saved}" style="--p:${(c.saved / c.total).toFixed(3)}"><i></i></div>
    </div>`;
  }
  function aggBody(ids, label) {
    var c = aggCounts(ids);
    return html`<div class="disc-agg disc-agg--sheet">
        <div class="disc-agg__top">${icon(c.failed ? 'warning' : 'save')}<span class="disc-agg__text">${aggText(c)}</span></div>
        <div class="progress" role="progressbar" aria-label="Saving ${label}" aria-valuemin="0" aria-valuemax="${c.total}" aria-valuenow="${c.saved}" style="--p:${(c.total ? c.saved / c.total : 0).toFixed(3)}"><i></i></div>
      </div>
      <ul class="disc-aggl">${ids.map(function (id) {
        var t = M.track(id), s = App.saves.get(id), status;
        if (!t) return '';
        if (App.isSaved(id)) status = ui.savedBadge();
        else if (s && s.status === 'failed') status = html`<span class="disc-aggl__fail">${ui.statusGlyph('down', 'Failed')}Needs attention</span><button class="btn btn--sm" data-act="disc-save-sheet" data-id="${id}">Fix</button>`;
        else if (s && s.status === 'pending') status = html`<span class="disc-aggl__st">${icon('clock', null, 'ic-16')}waiting for Jellyfin</span>`;
        else if (s) status = html`<span class="disc-aggl__st"><span class="px">step ${s.step} of 5</span><span class="disc-aggl__lbl">${M.saveSteps[s.step - 1]}</span></span>`;
        else status = html`<span class="disc-aggl__st">${icon('dash', null, 'ic-16')}not saving</span>`;
        return html`<li class="disc-aggl__row">${ui.cover(t, 40)}<span class="disc-aggl__main"><b>${t.title}</b><span>${t.artist}</span></span><span class="disc-aggl__status">${status}</span></li>`;
      })}</ul>`;
  }
  function openAggregate(label, ids) {
    var off = null;
    return App.openSheet({ title: 'saving ' + label, className: 'disc-sheet', body: html`<div data-region="aggs">${aggBody(ids, label)}</div>`,
      foot: html`<button class="btn" data-act="open-activity" data-close>${icon('activity')}Open Activity</button><button class="btn" data-close>Done</button>`,
      onMount: function (el) { off = App.on('save', function (d) { if (ids.indexOf(d.id) >= 0) paint('aggs', aggBody(ids, label), el); }); },
      onClose: function () { if (off) off(); } });
  }
  App.action('disc-agg-open', function (el) { openAggregate(el.getAttribute('data-label'), el.getAttribute('data-ids').split(',')); });
  /* save any list with aggregate progress (the queue panel can use: data-act="disc-save-ids" data-ids="..." data-label="your queue") */
  App.action('disc-save-ids', function (el) {
    var ids = (el.getAttribute('data-ids') || '').split(',').filter(Boolean);
    App.saves.start(ids);
    if (ids.filter(function (id) { return !App.isSaved(id); }).length > 1) openAggregate(el.getAttribute('data-label') || plural(ids.length, 'song'), ids);
  });

  /* =====================================================================
     5. Routes, public API
     ===================================================================== */
  App.route('/home', { render: renderHome, mount: mountHome, nav: 'home', title: 'Home',
    meta: function () { var f = M.peek(FEED); return f && f.updated ? 'updated ' + ago(f.updated) : ''; } });
  App.route('/search', { render: renderSearch, mount: mountSearch, nav: 'search', keepFocus: true, meta: function () { return 'catalogs + soulseek'; },
    title: function () { return S.q ? 'Search: ' + S.q : 'Search'; } });

  App.discover = { openFolder: openFolder, openPicker: openPicker, openDetails: openDetails, openSaveSheet: openSaveSheet,
    openAggregate: openAggregate, openReleasePicker: openReleasePicker, openDuplicate: openDuplicate, startRadio: startRadio, thousands: thousands };
})();
