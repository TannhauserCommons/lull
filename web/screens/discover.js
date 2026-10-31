/* screens/discover.js: Home (discovery feed), Search, Soulseek folder sheet, source picker,
   source details and the save flow (duplicate dialog, step failures, release picker, aggregate saves).
   Owner: discover. Takes over the hooks beforeSave, sourcePicker, sourceDetails and reason.
   Also exposes App.discover.{ openFolder, openPicker, openDetails, openSaveSheet, openAggregate,
   openReleasePicker, openDuplicate, startRadio } for other screens. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$;
  var P = App.player;

  /* =====================================================================
     Small helpers
     ===================================================================== */
  function fmt(name) { return M.formats[name] || { label: name, codec: name, detail: '', lossless: false, kbps: 0, mbPerMin: 2 }; }
  function isBetter(a, b) { return fmt(a).kbps > fmt(b).kbps; }
  /* worth flagging as "better than saved": lossy -> lossless, or a higher-resolution lossless file */
  function isUpgrade(a, b) { var fa = fmt(a), fb = fmt(b); return fa.lossless && (!fb.lossless || fa.kbps > fb.kbps); }
  function sizeFor(format, sec) { return fmt(format).mbPerMin * sec / 60; }
  function size(mb) { return mb >= 1000 ? (mb / 1000).toFixed(1) + ' GB' : (mb >= 100 ? Math.round(mb) : Math.round(mb * 10) / 10) + ' MB'; }
  function plural(n, one, many) { return n + ' ' + (n === 1 ? one : (many || one + 's')); }
  function isPhone() { return window.innerWidth < 640; }
  function hashStr(s) { var h = 7; for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 100003; return h; }
  function thousands(n) { return n >= 1000 ? (Math.round(n / 100) / 10) + 'k' : String(n); }
  function pickFor(trackId) { return M.feed.picks.filter(function (p) { return p.trackId === trackId; })[0] || null; }

  /* re-render one [data-region] in place; keeps focus on the same control when it still exists */
  function paint(name, markup, root) {
    var el = $('[data-region="' + name + '"]', root || document); if (!el) return;
    var a = document.activeElement, had = a && el.contains(a);
    var key = had ? (a.getAttribute('data-act') || '') + '|' + (a.getAttribute('data-id') || a.getAttribute('data-q') || '') : null;
    el.innerHTML = String(markup);
    App.syncRows();
    if (had) {
      var back = $$('[data-act]', el).filter(function (b) { return (b.getAttribute('data-act') || '') + '|' + (b.getAttribute('data-id') || b.getAttribute('data-q') || '') === key; })[0];
      (back || el.querySelector('button, a[href]') || $('#view')).focus({ preventScroll: true });
    }
  }

  /* radio from anything: start it, then show where it lands (queue panel; Now Playing > Queue on phone) */
  function startRadio(seed) {
    P.startRadio(seed);
    if (isPhone()) App.go('/now?tab=queue'); else App.openPanel('queue');
  }
  App.action('disc-radio', function (el) {
    App.closePopover();
    startRadio({ kind: el.getAttribute('data-kind') || 'track', id: el.getAttribute('data-id') });
  });

  /* prototype states: apply() runs on navigation only; `open` (an overlay) runs once after mount */
  var pendingOpen = null, STATE_Q = {};
  function st(id, label, route, o) {
    o = o || {};
    if (o.q != null) STATE_Q[id] = { q: o.q, p: o.p == null ? 1 : o.p };
    return { id: id, label: label, route: route, apply: function (ctx) {
      if (o.q != null && slskDown()) App.setService('soulseek', { state: 'ok', detail: 'Connected, 63 peers answered the last search' });
      if (o.apply) o.apply(ctx);
      pendingOpen = o.open || null;
    } };
  }
  function runPending() { var fn = pendingOpen; pendingOpen = null; if (fn) setTimeout(fn, 160); }

  /* =====================================================================
     1. Home / discovery feed
     ===================================================================== */
  var H = freshHome(null);
  function freshHome(key) {
    return { key: key, loading: key === 'discover-home-loading', shelfError: key === 'discover-home-shelf-error', shelfLoading: false,
      cold: key === 'discover-home-cold', noSourceFirst: key === 'discover-home-no-source',
      dismissed: {}, coldPicks: {}, finding: null, found: {}, rotate: 0 };
  }
  function greeting() {
    var c = /* PROTO: ?clock=HH (0-23) pins the hour for screenshots */ App.current && App.current.query.clock;
    var h = c != null && /^\d{1,2}$/.test(c) ? +c : new Date().getHours();
    return h >= 5 && h < 12 ? 'good morning' : h >= 12 && h < 18 ? 'good afternoon' : 'good evening';
  }

  function renderHome(ctx) {
    var key = /* PROTO: demo state */ ctx.state || '';
    if (H.key !== key) H = freshHome(key);
    if (H.cold) return renderCold();
    return html`<div class="disc-home">
      <header class="disc-hello">
        <h1 class="t-display">${greeting()}${ui.sparkle('--s:18px')}</h1>
        <p class="disc-hello__sum">${H.loading ? 'Gathering new releases and picks for you.' : M.feed.summary}</p>
      </header>
      ${homeLead()}
      <div class="disc-duo">
        <section class="card disc-card" aria-labelledby="disc-h-new" data-region="new" data-meta="${plural(M.feed.newReleases.length, 'release')}">${newReleasesBody()}</section>
        <section class="card disc-card" aria-labelledby="disc-h-radio" data-region="radio">${radioBody()}</section>
      </div>
      <section class="card disc-card" aria-labelledby="disc-h-picks" data-region="picks" data-meta="updated ${M.feed.updated}">${picksBody()}</section>
      <div class="disc-duo disc-duo--even">
        <section class="card disc-card" aria-labelledby="disc-h-jump" data-region="jump">${jumpBody()}</section>
        <section class="card disc-card" aria-labelledby="disc-h-tags" data-region="tags">${tagsBody()}</section>
      </div>
    </div>`;
  }

  /* homeLead slot: a design's own first object on Home (a media window, a widget), between the greeting and the
     shelves. Default: nothing. ctx = { track, player, queue: next tracks (up to 8), parts: { cover(t, size),
     playButton(size), likeButton(t), trackList(tracks, opts), formatLine(t) } }. Rendered as .disc-lead[data-region="lead"]. */
  function homeLead() {
    if (H.loading) return '';
    var t = P.current();
    var out = String(App.slot('homeLead', { track: t, player: P.state, queue: P.upNext(8), parts: {
      cover: function (x, size) { return ui.cover(x, size || 64); },
      playButton: function (size) { return gloss(size); },
      likeButton: function (x) { return ui.likeButton((x || t || {}).id || ''); },
      trackList: function (list, opts) { return ui.trackList(list, Object.assign({ dense: true }, opts || {})); },
      formatLine: App.fmt.formatLine } }, function () { return ''; }));
    return out ? html`<div class="disc-lead" data-region="lead">${raw(out)}</div>` : '';
  }
  function gloss(size) {             // sizes with a .btn-gloss--N class: 40 48 52 56 64
    var on = P.state.playing;
    return html`<button class="btn-gloss btn-gloss--${size || 56}" data-act="play" data-bind="play" aria-label="${on ? 'Pause' : 'Play'}" data-tip="${on ? 'Pause (Space)' : 'Play (Space)'}"><i class="btn-gloss__spin" aria-hidden="true"></i>${icon(on ? 'pause' : 'play', null, on ? '' : 'ic-play')}</button>`;
  }
  function newReleasesBody() {
    var head = ui.labelRule('new releases', { id: 'disc-h-new', action: H.loading || H.shelfError || H.shelfLoading ? null : { label: 'See all', href: '#/library/albums' } });
    if (H.loading || H.shelfLoading) return html`${head}<div class="disc-skel-shelf">${ui.skeleton('tile', 4)}</div>`;
    if (H.shelfError) {
      return html`${head}${ui.emptyState({ art: 'star', title: 'Couldn\'t load new releases',
        text: 'MusicBrainz didn\'t answer in time. The rest of your feed is fine.', action: { label: 'Retry', act: 'disc-retry-shelf' } })}`;
    }
    return html`${head}<div class="shelf disc-shelf">${M.feed.newReleases.map(function (id) { return ui.releaseTile(id); })}</div>`;
  }

  function seedArt(seed, px) {
    var key = seed.kind === 'track' ? M.track(seed.id).albumId : seed.id;
    return ui.cover(key, px || 48, { round: true });
  }
  function radioBody() {
    var head = ui.labelRule('radio starters', { id: 'disc-h-radio' });
    if (H.loading) return html`${head}${ui.skeleton('row', 4)}`;
    return html`${head}<ul class="disc-starters">${M.feed.radioStarters.map(function (r) {
      /* the whole row starts the radio on click; the round button is the keyboard control */
      return html`<li class="disc-starter" data-act="disc-radio" data-kind="${r.seed.kind}" data-id="${r.seed.id}">
        ${seedArt(r.seed)}
        <div class="disc-starter__main"><span class="disc-starter__title">${r.title}</span><span class="disc-starter__sub">${r.sub}</span></div>
        <button class="btn-icon disc-starter__play" data-act="disc-radio" data-kind="${r.seed.kind}" data-id="${r.seed.id}" aria-label="Start ${r.title}" data-tip="Start radio">${icon('play')}</button>
      </li>`;
    })}</ul>`;
  }

  function visiblePicks() {
    var hidden = App.store.get('hiddenArtists') || {};
    var list = M.feed.picks.filter(function (p) { return !H.dismissed[p.trackId] && !hidden[M.track(p.trackId).artistId]; });
    var normal = list.filter(function (p) { return !p.noSource; }), none = list.filter(function (p) { return p.noSource; });
    var r = normal.length ? H.rotate % normal.length : 0;
    normal = normal.slice(r).concat(normal.slice(0, r));
    return (H.noSourceFirst ? none.concat(normal) : normal).slice(0, 6);
  }
  function picksBody() {
    var head = ui.labelRule('picked for you', { id: 'disc-h-picks', action: H.loading ? null : { label: 'Refresh picks', act: 'disc-rotate' } });
    if (H.loading) return html`${head}<div class="disc-picks-skel">${ui.skeleton('row', 6)}</div>`;
    var picks = visiblePicks();
    if (!picks.length) {
      return html`${head}${ui.emptyState({ art: 'moon', title: 'No picks right now',
        text: 'You hid everything we had. Like a few songs or follow a tag and new picks show up here.', action: { label: 'Browse tags', href: '#/following' } })}`;
    }
    var playable = picks.filter(function (p) { return !p.noSource || H.found[p.trackId]; }).map(function (p) { return p.trackId; });
    return html`${head}<ul class="disc-picks" data-ids="${playable.join(',')}">${picks.map(pickItem)}</ul>`;
  }
  function pickItem(p) {
    var t = M.track(p.trackId), more = p.reasons.length - 1;
    var noSrc = p.noSource && !H.found[t.id], finding = H.finding === t.id;
    return html`<li class="disc-pick${noSrc ? ' is-nosource' : ''}">
      <div class="disc-pick__art">${ui.cover(t, 64)}${noSrc ? '' : html`<button class="disc-pick__play" data-act="play-track" data-id="${t.id}" tabindex="-1" aria-label="Play ${t.title}" data-tip="Play">${icon('play')}</button>`}</div>
      <div class="disc-pick__main">
        ${noSrc ? html`<span class="disc-pick__title">${t.title}</span>` : html`<button class="disc-pick__title" data-act="play-track" data-id="${t.id}">${t.title}</button>`}
        <a class="disc-pick__by" href="#/artist/${t.artistId}">${t.artist}</a>
      </div>
      <div class="disc-pick__end">${ui.likeButton(t.id)}<button class="btn-icon btn-icon--bare" data-act="disc-pick-menu" data-id="${t.id}" aria-haspopup="menu" aria-label="More actions for ${t.title}" data-tip="More">${icon('more')}</button></div>
      <div class="disc-pick__why">${ui.reasonChip(p.reasons[0], { more: more, via: p.via, itemId: t.id })}
        ${noSrc ? html`<div class="disc-pick__nosrc">
          ${finding ? html`<span class="resolve" role="status"><span class="spin-ring" aria-hidden="true"></span>Searching Soulseek</span>`
            : html`<span class="resolve">${icon('no-source')}No source found yet</span>`}
          <button class="btn btn--sm" data-act="disc-find" data-id="${t.id}" ${finding ? raw('disabled') : ''}>${icon('refresh')}Search again</button></div>` : ''}
      </div>
    </li>`;
  }

  function jumpTile(j) {
    var art, href, title, by, byHref, play, round = false;
    if (j.kind === 'album') {
      var a = M.album(j.id); art = ui.cover(a, '100%'); href = '#/album/' + a.id; title = a.title; by = a.artist; byHref = '#/artist/' + a.artistId;
      play = html`<button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button>`;
    } else if (j.kind === 'playlist') {
      var p = (App.store.get('playlists') || []).filter(function (x) { return x.id === j.id; })[0];
      if (!p) return '';
      art = ui.mosaic(p.items.map(function (i) { return i.trackId; }), '100%'); href = '#/playlist/' + p.id; title = p.name; by = 'playlist, ' + plural(p.items.length, 'song');
      play = html`<button class="tile__play" data-act="play-playlist" data-id="${p.id}" aria-label="Play ${p.name}" data-tip="Play">${icon('play')}</button>`;
    } else {
      var ar = M.artist(j.id); art = ui.cover(ar.id, '100%', { round: true }); href = '#/artist/' + ar.id; title = ar.name; by = 'artist'; round = true;
      play = html`<button class="tile__play" data-act="disc-radio" data-kind="artist" data-id="${ar.id}" aria-label="Start ${ar.name} radio" data-tip="Start radio">${icon('radio')}</button>`;
    }
    return html`<div class="tile disc-jump${round ? ' disc-jump--round' : ''}">
      <div class="tile__art"><a href="${href}" tabindex="-1" aria-hidden="true">${art}</a>${play}</div>
      <div class="tile__title"><a href="${href}">${title}</a></div>
      <div class="tile__by">${byHref ? html`<a href="${byHref}">${by}</a>` : by}</div>
    </div>`;
  }
  function jumpBody() {
    var head = ui.labelRule('jump back in', { id: 'disc-h-jump', action: H.loading ? null : { label: 'History', href: '#/history' } });
    if (H.loading) return html`${head}<div class="disc-skel-shelf">${ui.skeleton('tile', 4)}</div>`;
    return html`${head}<div class="shelf disc-shelf disc-shelf--sm">${M.feed.jumpBackIn.map(jumpTile)}</div>`;
  }

  function tagsBody() {
    var head = ui.labelRule('tags you follow', { id: 'disc-h-tags', action: H.loading ? null : { label: 'Manage', href: '#/following' } });
    if (H.loading) return html`${head}<div class="cluster">${ui.skeleton('chip', 5)}</div>`;
    var followed = M.tags.filter(function (t) { return App.isFollowed(t.id); });
    var related = M.tags.filter(function (t) { return !App.isFollowed(t.id); }).slice(0, 4);
    return html`${head}
      ${followed.length ? html`<ul class="disc-tags">${followed.map(function (t) {
        return html`<li class="disc-tag"><a class="disc-tag__name" href="#/tag/${encodeURIComponent(t.id)}">${icon('tag')}<span>${t.name}</span></a>
          <span class="disc-tag__desc">${t.desc}</span>
          <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="disc-radio" data-kind="tag" data-id="${t.id}" aria-label="Start ${t.name} radio" data-tip="Start radio">${icon('radio')}</button></li>`;
      })}</ul>` : html`<p class="disc-note">You don't follow any tags yet. Follow one below and its new releases show up here.</p>`}
      ${related.length ? html`${ui.labelRule('you might like', { level: 'h3', sm: true })}<div class="cluster">${related.map(function (t) { return ui.tagChip(t.id, { toggle: true, pressed: false }); })}</div>` : ''}`;
  }

  /* cold start: nothing played, nothing followed yet */
  var CLOUD_W = { 'dream-gaze': 3, 'city-pop revival': 3, 'chiptune-folk': 2, 'ambient jazz': 2, slowcore: 1, 'shibuya-kei': 2, 'bedroom pop': 3, shoegaze: 1 };
  function renderCold() {
    return html`<div class="disc-home">
      <header class="disc-hello">
        <h1 class="t-display">${greeting()}${ui.sparkle('--s:18px')}</h1>
        <p class="disc-hello__sum">Nothing to go on yet. Pick a few tags or artists and your feed fills in from there.</p>
      </header>
      <section class="card disc-card disc-cold" aria-labelledby="disc-h-cold">
        ${ui.labelRule('pick a few to start', { id: 'disc-h-cold' })}
        <div class="disc-cloud" role="group" aria-label="Tags">${M.feed.coldStartTags.map(function (id) { return coldTag(id); })}</div>
        ${ui.labelRule('or a few artists', { level: 'h3', sm: true })}
        <ul class="disc-cold__artists">${M.artists.slice(0, 6).map(function (a) {
          return html`<li class="disc-cold__artist">${ui.cover(a.id, 56, { round: true })}<span class="disc-cold__name">${a.name}<small>${a.tags[0]}</small></span>${coldArtistBtn(a)}</li>`;
        })}</ul>
        <div class="disc-cold__foot">
          <span class="disc-cold__count" data-cold-count aria-live="polite">${coldCount()}</span>
          <span class="spacer"></span>
          <a class="btn btn--quiet" href="#/search">Try a search instead</a>
          <button class="btn btn--primary" data-act="disc-build-feed">${icon('sparkle')}Build my feed</button>
        </div>
      </section>
    </div>`;
  }
  function coldTag(id) {
    var on = !!H.coldPicks[id];
    return html`<button class="chip disc-cloud__tag disc-cloud__tag--w${CLOUD_W[id] || 1}" data-act="disc-cold-pick" data-id="${id}" aria-pressed="${on ? 'true' : 'false'}">${on ? icon('check') : ''}${M.tag(id) ? M.tag(id).name : id}</button>`;
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
  App.action('disc-retry-shelf', function () {
    H.shelfError = false; H.shelfLoading = true; paint('new', newReleasesBody());
    App.announce('Loading new releases');
    /* PROTO: simulated delay */ setTimeout(function () {
      H.shelfLoading = false; paint('new', newReleasesBody());
      var first = $('[data-region="new"] .tile__title a'); if (first) first.focus({ preventScroll: true });
      App.announce('New releases loaded');
    }, 1000);
  });
  App.action('disc-rotate', function () {
    H.rotate += 2; paint('picks', picksBody());
    App.toast('Fresh picks from your recent listening', { icon: 'sparkle' });
  });
  App.action('disc-find', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id);
    H.finding = id; paint('picks', picksBody()); App.announce('Searching Soulseek for ' + t.title);
    /* PROTO: simulated delay */ setTimeout(function () {
      H.finding = null; H.found[id] = true;
      if ($('[data-region="picks"]')) paint('picks', picksBody());
      App.toast('Found ' + plural(t.sources.length, 'source') + ' for ' + t.title, { kind: 'ok', icon: 'soulseek', action: 'Play',
        onAction: function () { P.playTrack(id); } });
    }, 2400);
  });
  function dismissPick(id, msg) {
    H.dismissed[id] = true; paint('picks', picksBody());
    App.toast(msg || 'Removed from your picks', { icon: 'info', action: 'Undo', onAction: function () { delete H.dismissed[id]; paint('picks', picksBody()); } });
  }
  App.action('disc-pick-menu', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id), p = pickFor(id), followed = App.isFollowed(t.artistId);
    var noSrc = p && p.noSource && !H.found[id];
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
    items.push('sep', { label: 'Not interested', icon: 'close', run: function () { dismissPick(id); } });
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
  App.action('disc-build-feed', function () {
    var ids = Object.keys(H.coldPicks);
    if (!ids.length) { App.toast('Pick at least one tag or artist first', { icon: 'info' }); return; }
    App.store.update('followed', function (f) { ids.forEach(function (id) { f[id] = true; }); });
    H = freshHome(''); H.loading = true;
    history.replaceState(null, '', '#/home');
    App.refresh();
    /* PROTO: simulated delay */ setTimeout(function () {
      if (App.current.path !== '/home') return;
      H.loading = false; App.refresh();
      App.toast('Following ' + plural(ids.length, 'pick') + '. Your feed is ready.', { kind: 'ok', icon: 'sparkle' });
    }, 1400);
  });
  App.action('disc-less', function (el) {
    var id = el.getAttribute('data-id'); App.closePopover(true);
    if (id && pickFor(id) && $('[data-region="picks"]')) dismissPick(id, 'Got it: less like this, and removed from your picks');
    else App.toast('Got it: less like this', { icon: 'info' });
  });

  function mountHome(ctx) {
    var offStore = App.on('store', function (keys) {
      if (H.cold || H.loading) return;
      if (keys.indexOf('hiddenArtists') >= 0) paint('picks', picksBody());
      if (keys.indexOf('followed') >= 0) paint('tags', tagsBody());
      if (keys.indexOf('playlists') >= 0) paint('jump', jumpBody());
    });
    ctx.onCleanup(offStore);
    runPending();
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
    var reasons = p ? p.reasons : [{ kind: r.kind, name: r.name, ref: r.ref }];
    var artist = t ? M.artist(t.artistId) : (r.kind !== 'tag' && r.ref ? M.artist(r.ref) : null);
    App.popover(anchor, html`<div class="popover__head">${icon('sparkle')}<span>why this?</span></div>
      <div class="popover__body disc-why">
        ${t ? html`<p class="disc-why__item">${t.title}, ${t.artist}</p>` : ''}
        <ul class="disc-why__list">${reasons.map(function (x) { return html`<li>${icon(REASON_ICON[x.kind] || 'sparkle')}<span>${reasonText(x)}</span></li>`; })}</ul>
        <p class="disc-why__via">via ${r.via || (p && p.via) || 'ListenBrainz'}</p>
      </div>
      <div class="popover__actions">
        <button class="btn btn--sm" data-act="more-like" data-id="${r.item || ''}">${icon('plus')}More like this</button>
        <button class="btn btn--sm" data-act="disc-less" data-id="${r.item || ''}">${icon('minus')}Less like this</button>
        ${artist ? html`<button class="btn btn--sm" data-act="hide-artist" data-id="${artist.id}" aria-label="Hide ${artist.name} from your feed">Hide this artist</button>` : ''}
      </div>`, { label: 'Why this was suggested', className: 'disc-why-pop' });
  };

  /* =====================================================================
     2. Search: library first (instant), Soulseek streams in with a live counter
     ===================================================================== */
  var SEARCH_MS = 12000;
  var S = { q: null, t0: 0, frozen: null, src: 'all', ll: false, showAll: false, done: false, stateKey: null, timer: null };

  function progress() {
    if (S.frozen != null) return S.frozen;
    return Math.min(1, (Date.now() - S.t0) / SEARCH_MS);
  }
  function slskDown() { var s = App.store.get('services'); return !!(s && s.soulseek && s.soulseek.state === 'down'); }
  function streaming() { return !slskDown() && S.src !== 'library' && progress() < 1; }
  function beginSession(q) {
    if (q === S.q) return;
    S.q = q; S.t0 = Date.now(); S.frozen = null; S.showAll = false; S.done = !q;
  }

  /* match every word against title + artist + album; artists also come in through their matching albums */
  function runSearch(q) {
    var words = String(q || '').toLowerCase().split(/\s+/).filter(Boolean);
    function hit(s) { s = s.toLowerCase(); return words.length && words.every(function (w) { return s.indexOf(w) >= 0; }); }
    var tracks = M.tracks.filter(function (t) { return hit(t.title + ' ' + t.artist + ' ' + t.album); });
    var albums = M.albums.filter(function (a) { return hit(a.title + ' ' + a.artist); });
    var artists = M.artists.filter(function (a) { return hit(a.name) || albums.some(function (al) { return al.artistId === a.id; }); });
    var srcCount = tracks.reduce(function (n, t) { return n + t.sources.length; }, 0);
    var total = tracks.length ? srcCount * 4 + hashStr(q) % 17 : 0;
    return { tracks: tracks, albums: albums, artists: artists, total: total, peers: Math.round(total / 3.4) };
  }
  function bestFormat(t) { return App.isSaved(t.id) ? t.format : t.sources[0].format; }
  function filtered(R) {
    var down = slskDown(), src = S.src;
    function keepTrack(t) {
      var saved = App.isSaved(t.id);
      if ((src === 'library' || down) && !saved) return false;
      if (S.ll && !fmt(bestFormat(t)).lossless && !(saved && fmt(t.sources[0].format).lossless && src === 'soulseek')) return false;
      return true;
    }
    function keepAlbum(a) {
      if ((src === 'library' || down) && a.saved === 'none') return false;
      return !S.ll || fmt(a.format).lossless;
    }
    var songs = R.tracks.filter(keepTrack);
    songs.sort(function (a, b) { return (App.isSaved(b.id) ? 1 : 0) - (App.isSaved(a.id) ? 1 : 0); });
    var albums = R.albums.filter(keepAlbum);
    var artists = R.artists.filter(function (a) { return !(src === 'library' || down) || M.albumsBy(a.id).some(function (al) { return al.saved !== 'none'; }); });
    return { songs: songs, albums: albums, artists: artists };
  }
  function pickTop(F, q) {
    var n = String(q).toLowerCase().trim();
    var exact = F.songs.filter(function (t) { return t.title.toLowerCase() === n; })[0];
    if (exact) return { kind: 'track', item: exact };
    var ar = F.artists.filter(function (a) { return a.name.toLowerCase().indexOf(n) === 0; })[0];
    if (ar) return { kind: 'artist', item: ar };
    if (F.albums[0]) return { kind: 'album', item: F.albums[0] };
    if (F.songs[0]) return { kind: 'track', item: F.songs[0] };
    return null;
  }

  /* gating for streamed results: [data-after=p] shows once progress >= p, [data-before=p] hides then */
  function after(th) { return raw(' data-after="' + th + '"' + (streaming() && progress() < th ? ' hidden' : '')); }
  function before(th) { return raw(' data-before="' + th + '"' + (streaming() && progress() < th ? '' : ' hidden')); }
  function slskOnly(t) { return !App.isSaved(t.id); }

  function renderSearch(ctx) {
    var def = /* PROTO: demo state */ ctx.state && STATE_Q[ctx.state];
    if (def && ctx.query.q == null) {
      /* PROTO: demo state */ if (S.stateKey !== ctx.state) { S.stateKey = ctx.state; S.q = def.q; S.t0 = Date.now(); S.frozen = def.p; S.showAll = false; S.src = 'all'; S.ll = false; S.done = true; }
    } else {
      if (S.stateKey) { S.q = null; S.stateKey = null; }      // leaving a demo state: start a live session
      beginSession(ctx.query.q || '');
    }
    var q = S.q || '';
    return html`<div class="disc-search">
      <header class="disc-hello disc-hello--search">
        <h1 class="t-display">search</h1>
        <p class="disc-hello__sum" data-region="sub">${subline()}</p>
      </header>
      <div class="disc-q search-field" role="search">
        ${icon('search')}
        <label class="vh" for="disc-q">Search your library and Soulseek</label>
        <input class="input" id="disc-q" type="search" value="${q}" placeholder="Songs, albums, artists" autocomplete="off" spellcheck="false" enterkeyhint="search">
      </div>
      <div class="disc-filters" role="group" aria-label="Filter results" data-filters ${q ? '' : raw('hidden')}>
        <div class="disc-filters__group" role="group" aria-label="Where to search">
          ${ui.filterPill('All', { act: 'disc-src', value: 'all', pressed: S.src === 'all' })}
          ${ui.filterPill('Library', { act: 'disc-src', value: 'library', pressed: S.src === 'library', icon: 'saved' })}
          ${ui.filterPill('Soulseek', { act: 'disc-src', value: 'soulseek', pressed: S.src === 'soulseek', icon: 'soulseek' })}
        </div>
        <span class="disc-filters__sep" aria-hidden="true"></span>
        ${ui.filterPill('Lossless only', { act: 'disc-ll', pressed: S.ll, icon: 'quality-diamond' })}
      </div>
      <div class="disc-results" data-region="results">${q ? resultsBody() : emptySearchBody()}</div>
    </div>`;
  }
  function subline() {
    return S.q ? html`Results for <b>${S.q}</b>: your library first, then what Soulseek peers share.`
      : 'Your library answers first, then Soulseek peers.';
  }

  function emptySearchBody() {
    var tags = M.tags.filter(function (t) { return App.isFollowed(t.id); });
    return html`<div class="disc-duo">
      <section class="card disc-card" aria-labelledby="disc-h-recent" data-region="recents">${recentsBody()}</section>
      <section class="card disc-card" aria-labelledby="disc-h-sugg">
        ${ui.labelRule('from tags you follow', { id: 'disc-h-sugg' })}
        <div class="cluster disc-sugg">${tags.map(function (t) { return ui.chip(t.name, { href: '#/search?q=' + encodeURIComponent(t.name), icon: 'search' }); })}</div>
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
    var R = runSearch(S.q), F = filtered(R);
    var any = F.songs.length || F.albums.length || F.artists.length;
    if (!any) {
      if (streaming()) {
        return html`<section class="card disc-card">${ui.labelRule('songs')}${scanLine(R)}
          <p class="disc-note">Nothing in your library. Still asking peers.</p>${ui.skeleton('row', 3)}</section>`;
      }
      return html`<section class="card disc-card">${S.src !== 'library' ? scanLine(R, true) : ''}${ui.emptyState(noResults())}</section>`;
    }
    var top = pickTop(F, S.q);
    var songs = F.songs, limit = S.showAll ? songs.length : 6, shown = songs.slice(0, limit);
    if (!S.showAll) {          // library rows come first, but keep up to 2 Soulseek-only rows in view
      var libHits = songs.filter(function (t) { return !slskOnly(t); }), peerHits = songs.filter(slskOnly);
      var nLib = Math.min(libHits.length, limit - Math.min(2, peerHits.length));
      shown = libHits.slice(0, nLib).concat(peerHits.slice(0, limit - nLib));
    }
    var k = 0;   // index among Soulseek-only rows, for staggered arrival
    var rows = shown.map(function (t, i) {
      var saved = App.isSaved(t.id), best = t.sources[0].format;
      var dup = slskDown() ? null : saved && isUpgrade(best, t.format) ? 'better' : saved && S.src === 'soulseek' ? 'saved' : null;
      /* the duplicate marker goes on the artist line, so badges never squeeze the title */
      var row = String(ui.trackRow(t, { dense: true, showSources: true, index: i + 1 }))
        .replace(/(<div class="trow__artist">[\s\S]*?)(<\/div>)/, function (m, a, b) { return a + (dup ? String(dup === 'better' ? ui.dupBetter() : ui.dupNote()) : '') + b; });
      if (!saved) { row = row.replace('<li ', '<li' + String(after(Math.min(.9, .12 + k * .1))) + ' '); k++; }
      return raw(row);
    });
    var waiting = songs.length && shown.some(slskOnly) && streaming();
    var topAfter = top && top.kind === 'track' && slskOnly(top.item) ? .12 : top && top.kind === 'album' && top.item.saved === 'none' ? .45 : 0;
    return html`<div class="disc-duo disc-duo--results">
        ${top ? html`<section class="card disc-card disc-top" aria-labelledby="disc-h-top">${ui.labelRule('top result', { id: 'disc-h-top' })}
          ${topAfter ? html`<div${before(topAfter)}>${ui.skeleton('row', 2)}</div>` : ''}<div${after(topAfter)}>${topCard(top)}</div></section>` : ''}
        <section class="card disc-card disc-songs" aria-labelledby="disc-h-songs">
          ${ui.labelRule('songs', { id: 'disc-h-songs' })}
          ${scanLine(R)}
          ${songs.length ? html`<ol class="tlist tlist--dense" data-ids="${shown.map(function (t) { return t.id; }).join(',')}" aria-label="Songs">${rows}</ol>`
            : html`<p class="disc-note">No songs match${S.ll ? ' in lossless' : ''}.</p>`}
          ${waiting ? html`<div${before(.12 + (k - 1) * .1)}>${ui.skeleton('row', 2)}</div>` : ''}
          ${songs.length > limit ? html`<button class="btn btn--quiet disc-more" data-act="disc-show-all">Show all ${plural(songs.length, 'song')}</button>` : ''}
        </section>
      </div>
      ${F.albums.length || F.artists.length ? html`<div class="disc-duo disc-duo--results2">
        ${F.albums.length ? html`<section class="card disc-card" aria-labelledby="disc-h-albums">${ui.labelRule('albums', { id: 'disc-h-albums' })}
          <div class="shelf disc-shelf">${F.albums.map(function (a) { return folderTile(a, a.saved === 'none' ? .45 : 0); })}</div></section>` : ''}
        ${F.artists.length ? html`<section class="card disc-card" aria-labelledby="disc-h-artists">${ui.labelRule('artists', { id: 'disc-h-artists' })}
          <div class="shelf disc-shelf disc-shelf--artists">${F.artists.map(artistTile)}</div></section>` : ''}
      </div>` : ''}`;
  }
  function noResults() {
    if (S.src === 'library' || slskDown()) {
      return { art: 'moon', title: 'Nothing saved matches ' + S.q, text: slskDown() ? 'Soulseek is reconnecting, so only your library was searched. Try again once it is back.' : 'Your library has nothing by that name. Soulseek peers might.',
        action: slskDown() ? { label: 'Reconnect Soulseek', act: 'soulseek-reconnect' } : { label: 'Search Soulseek too', act: 'disc-src-all' } };
    }
    if (S.ll) return { art: 'moon', title: 'No lossless copies of ' + S.q, text: 'Peers only share lossy files for this. Turn off Lossless only to see them.', action: { label: 'Show all formats', act: 'disc-ll' } };
    return { art: 'moon', title: 'Nothing for ' + S.q, text: 'Not in your library, and no peer shares it right now. Check the spelling or try fewer words.',
      action: { label: 'Search Soulseek again', act: 'disc-search-again' }, secondary: { label: 'Clear search', act: 'disc-clear' } };
  }

  function scanLine(R, noAgain) {
    if (slskDown()) {
      return html`<div class="disc-scan is-down" role="status">${ui.statusGlyph('down', 'Soulseek down')}<span class="disc-scan__text">Soulseek is reconnecting, so only your library was searched.</span>
        <button class="btn btn--sm" data-act="soulseek-reconnect">Reconnect</button></div>`;
    }
    if (S.src === 'library') {
      return html`<div class="disc-scan disc-scan--lib">${icon('saved')}<span class="disc-scan__text">Showing your library only.</span>
        <button class="btn btn--quiet btn--sm" data-act="disc-src-all">Include Soulseek</button></div>`;
    }
    var p = progress(), done = p >= 1;
    return html`<div class="disc-scan${done ? ' is-done' : ''}" data-scan>
      ${icon('soulseek')}
      <span class="disc-scan__text" data-scan-text>${scanText(R, p)}</span>
      <span class="disc-scan__left px" data-scan-left aria-hidden="true">${done ? '' : '~' + Math.ceil((1 - p) * SEARCH_MS / 1000) + ' s'}</span>
      ${done && !noAgain ? html`<button class="btn btn--quiet btn--sm" data-act="disc-search-again">${icon('refresh')}Search again</button>` : ''}
      <div class="progress disc-scan__bar" role="progressbar" aria-label="Soulseek search" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round(p * 100)}" data-scan-bar><i style="--p:${p.toFixed(3)}"></i></div>
    </div>`;
  }
  function scanText(R, p) {
    var e = 1 - Math.pow(1 - p, 2), n = Math.round(R.total * e), peers = Math.max(n ? 1 : 0, Math.round(R.peers * e));
    return (p >= 1 ? 'Soulseek: ' : 'Searching Soulseek: ') + plural(n, 'result') + ' from ' + plural(peers, 'peer');
  }

  function topCard(top) {
    if (top.kind === 'track') {
      var t = top.item, saved = App.isSaved(t.id), best = t.sources[0], better = saved && !slskDown() && isUpgrade(best.format, t.format);
      return html`<div class="disc-top__body">
          ${ui.cover(t, 112)}
          <div class="disc-top__info">
            <p class="disc-top__kind">song</p>
            <h3 class="disc-top__title">${t.title}</h3>
            <p class="disc-top__by"><a href="#/artist/${t.artistId}">${t.artist}</a>, <a href="#/album/${t.albumId}">${t.album}</a></p>
            <div class="cluster disc-top__badges">${ui.sourceBadge(t)}${ui.qualityChip(saved ? t.format : best.format)}
              <button class="disc-top__n" data-act="show-sources" data-id="${t.id}">${plural(t.sources.length, 'source')}</button>${better ? ui.dupBetter() : ''}</div>
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
      var a = top.item, savedN = a.trackIds.filter(App.isSaved).length;
      return html`<div class="disc-top__body">
          ${ui.cover(a, 112)}
          <div class="disc-top__info">
            <p class="disc-top__kind">${a.type === 'album' ? 'album' : a.type}${a.saved === 'none' ? ', Soulseek folder' : ''}</p>
            <h3 class="disc-top__title">${a.title}</h3>
            <p class="disc-top__by"><a href="#/artist/${a.artistId}">${a.artist}</a>, ${a.year}</p>
            <div class="cluster disc-top__badges">${savedN === a.trackIds.length ? ui.savedBadge() : savedN ? html`<span class="badge badge-saved">${icon('saved')}${savedN} of ${a.trackIds.length} saved</span>` : ui.soulseekBadge()}
              ${ui.qualityChip(a.format)}<span class="px fg-2">${plural(a.trackIds.length, 'track')}, ${size(a.sizeMB)}</span></div>
          </div>
        </div>
        <div class="disc-top__actions">
          <button class="btn btn--primary" data-act="play-album" data-id="${a.id}">${icon('play')}Play album</button>
          ${slskDown() ? '' : html`<button class="btn" data-act="disc-folder" data-id="${a.id}">${icon('soulseek')}Open folder</button>`}
          ${savedN < a.trackIds.length ? html`<button class="btn" data-act="save-album" data-id="${a.id}">${icon('save')}Save whole album</button>` : ''}
          ${ui.iconButton('radio', 'Start radio from ' + a.title, { 'data-act': 'disc-radio', 'data-kind': 'album', 'data-id': a.id })}
        </div>`;
    }
    var ar = top.item, on = App.isFollowed(ar.id);
    return html`<div class="disc-top__body">
        ${ui.cover(ar.id, 112, { round: true })}
        <div class="disc-top__info">
          <p class="disc-top__kind">artist</p>
          <h3 class="disc-top__title">${ar.name}</h3>
          <p class="disc-top__by">${thousands(ar.listeners)} listeners</p>
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
  function folderTile(a, th) {
    return html`<div class="tile disc-folder-tile"${after(th)}>
      <div class="tile__art"><button class="disc-tile-open" data-act="disc-folder" data-id="${a.id}" tabindex="-1" aria-hidden="true">${ui.cover(a, '100%')}</button>
        <button class="tile__play" data-act="play-album" data-id="${a.id}" aria-label="Play ${a.title}" data-tip="Play">${icon('play')}</button></div>
      <div class="tile__title"><button data-act="disc-folder" data-id="${a.id}" aria-label="Open ${a.title} folder">${a.title}</button></div>
      <div class="tile__by">${a.artist}</div>
      <div class="tile__meta">${icon(a.saved === 'all' ? 'saved' : 'soulseek', a.saved === 'all' ? 'Saved' : 'On Soulseek', 'ic-16')}<span>${a.trackIds.length} tracks, ${size(a.sizeMB)}</span></div>
    </div>`;
  }
  function artistTile(a) {
    return html`<div class="tile disc-jump disc-jump--round">
      <div class="tile__art"><a href="#/artist/${a.id}" tabindex="-1" aria-hidden="true">${ui.cover(a.id, '100%', { round: true })}</a>
        <button class="tile__play" data-act="disc-radio" data-kind="artist" data-id="${a.id}" aria-label="Start ${a.name} radio" data-tip="Start radio">${icon('radio')}</button></div>
      <div class="tile__title"><a href="#/artist/${a.id}">${a.name}</a></div>
      <div class="tile__by">${thousands(a.listeners)} listeners</div>
    </div>`;
  }

  /* live update while Soulseek answers: counter, bar, staggered rows; no re-render (focus stays put) */
  function searchTick() {
    var p = progress(), R = runSearch(S.q);
    var txt = $('[data-scan-text]'); if (txt) txt.textContent = scanText(R, p);
    var left = $('[data-scan-left]'); if (left) left.textContent = p >= 1 ? '' : '~' + Math.ceil((1 - p) * SEARCH_MS / 1000) + ' s';
    var bar = $('[data-scan-bar]');
    if (bar) { bar.setAttribute('aria-valuenow', Math.round(p * 100)); $('i', bar).style.setProperty('--p', p.toFixed(3)); }
    $$('[data-after]').forEach(function (el) { el.hidden = p < +el.getAttribute('data-after'); });
    $$('[data-before]').forEach(function (el) { el.hidden = p >= +el.getAttribute('data-before'); });
    if (p >= 1) {
      stopTimer();
      if (!S.done) {
        S.done = true;
        if (S.frozen == null) addRecent(S.q);
        App.announce(scanText(R, 1));
      }
      paint('results', resultsBody());
    }
  }
  function stopTimer() { if (S.timer) { clearInterval(S.timer); S.timer = null; } }
  function ensureTimer() { stopTimer(); if (S.q && S.frozen == null && streaming()) /* PROTO: simulated streaming search */ S.timer = setInterval(searchTick, 250); }
  function addRecent(q) {
    q = String(q || '').trim(); if (q.length < 2) return;
    App.store.update('recentSearches', function (r) { var i = r.indexOf(q); if (i >= 0) r.splice(i, 1); r.unshift(q); r.length = Math.min(r.length, 8); });
  }

  /* phone: the header field is hidden, so the page has its own; it updates the results in place */
  function setQuery(q) {
    q = q.trim();
    history.replaceState(null, '', '#/search' + (q ? '?q=' + encodeURIComponent(q) : ''));
    S.stateKey = null; beginSession(q);
    var f = $('[data-filters]'); if (f) f.hidden = !q;
    paint('sub', subline());
    paint('results', q ? resultsBody() : emptySearchBody());
    ensureTimer();
  }
  function mountSearch(ctx) {
    var input = $('#disc-q'), t = null;
    if (input) {
      input.addEventListener('input', function () { clearTimeout(t); t = setTimeout(function () { setQuery(input.value); }, 220); });
      input.addEventListener('keydown', function (e) {
        if (e.key === 'Enter') { clearTimeout(t); setQuery(input.value); input.blur(); }
        if (e.key === 'Escape' && input.value) { e.stopPropagation(); input.value = ''; setQuery(''); }
      });
    }
    ensureTimer();
    var offStore = App.on('store', function (keys) {
      if (keys.indexOf('services') >= 0 && S.q) { paint('results', resultsBody()); ensureTimer(); }
      if (keys.indexOf('recentSearches') >= 0 && !S.q) paint('recents', recentsBody());
    });
    var offSave = App.on('save', function (d) { var el = $('[data-issue="' + d.id + '"]'); if (el) el.innerHTML = String(saveIssue(d.id)); });
    ctx.onCleanup(function () { stopTimer(); clearTimeout(t); offStore(); offSave(); });
    setTimeout(function () {                    // keep the header field in step (state links carry no ?q=)
      var h = $('#header-search'); if (h && document.activeElement !== h && h.value !== (S.q || '')) { h.value = S.q || ''; }
      var c = $('#header-search-clear'); if (c && h) c.hidden = !h.value;
    }, 0);
    runPending();
  }

  /* search actions */
  App.action('disc-src', function (el) {
    S.src = el.getAttribute('data-value') || 'all';
    $$('[data-act="disc-src"]').forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-value') === S.src)); });
    paint('results', resultsBody()); ensureTimer();
  });
  App.action('disc-src-all', function () { var b = $('[data-act="disc-src"][data-value="all"]'); if (b) { b.click(); b.focus(); } });
  App.action('disc-ll', function () {
    S.ll = !S.ll;
    $$('.disc-filters [data-act="disc-ll"]').forEach(function (b) { b.setAttribute('aria-pressed', String(S.ll)); });
    paint('results', resultsBody()); ensureTimer();
  });
  App.action('disc-search-again', function () {
    S.t0 = Date.now(); S.frozen = null; S.done = false; S.showAll = false;
    paint('results', resultsBody()); ensureTimer(); App.announce('Searching Soulseek again');
  });
  App.action('disc-clear', function () {
    var h = $('#header-search'); if (h) h.value = '';
    var i = $('#disc-q'); if (i) i.value = '';
    App.go('/search');
  });
  App.action('disc-show-all', function () {
    S.showAll = true; paint('results', resultsBody());
    var rows = $$('.disc-songs .trow'); if (rows[6]) rows[6].focus();
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
    return html`<div class="disc-shead">${ui.cover(t, 56)}<div class="disc-shead__main"><p class="disc-shead__title">${t.title}</p>
      <p class="disc-shead__by">${t.artist}, ${t.album}</p></div>${sub || ''}</div>`;
  }

  /* --- Soulseek folder (album result) --- */
  function folderAlts(a) {
    var first = M.track(a.trackIds[0]);
    return first.sources.map(function (s, i) {
      return { peer: s.peer, format: s.format, speed: s.speed, freeSlot: s.freeSlot, queue: s.queue, score: s.score,
        tracks: a.trackIds.length - (i === 2 ? 1 : 0), sizeMB: sizeFor(s.format, a.duration), cover: i !== 1 };
    });
  }
  function folderBody(a, idx) {
    var alts = folderAlts(a), cur = alts[idx], savedN = a.trackIds.filter(App.isSaved).length, all = savedN === a.trackIds.length;
    var missing = a.trackIds.length - cur.tracks;
    return html`<div class="disc-folder">
      <div class="disc-folder__head">
        <div class="disc-folder__art">${ui.cover(a, 128)}${cur.cover ? '' : html`<span class="disc-folder__nocover">no cover in folder</span>`}</div>
        <div class="disc-folder__info">
          <p class="disc-top__kind">${a.type} folder from <b>${cur.peer}</b></p>
          <h3 class="disc-folder__title">${a.title}</h3>
          <p class="disc-top__by"><a href="#/artist/${a.artistId}" data-close>${a.artist}</a>, ${a.year}, ${a.label}</p>
          <div class="cluster disc-folder__facts">${ui.qualityChip(cur.format)}<span class="px">${cur.tracks} tracks</span><span class="px">${size(cur.sizeMB)}</span>
            <span class="px">${cur.speed} MB/s</span><span class="disc-folder__slot">${icon(cur.freeSlot ? 'check' : 'clock', null, 'ic-16')}${cur.freeSlot ? 'free slot' : 'queue of ' + cur.queue}</span></div>
          <div class="disc-top__actions">
            <button class="btn btn--primary" data-act="play-album" data-id="${a.id}" data-close autofocus>${icon('play')}Play album</button>
            ${all ? ui.savedBadge() : html`<button class="btn" data-act="save-album" data-id="${a.id}">${icon('save')}${savedN ? 'Save the other ' + (a.trackIds.length - savedN) : 'Save whole album'}</button>`}
            ${ui.iconButton('radio', 'Start radio from ' + a.title, { 'data-act': 'disc-radio', 'data-kind': 'album', 'data-id': a.id, 'data-close': '' })}
            <a class="btn btn--quiet" href="#/album/${a.id}" data-close>Album page</a>
          </div>
        </div>
      </div>
      <div data-region="agg">${aggInline(a.trackIds, a.title)}</div>
      ${missing ? html`<p class="disc-issue">${icon('info')}<span>This folder is missing ${plural(missing, 'track')}. Missing songs are fetched from other peers when you play or save them.</span></p>` : ''}
      ${ui.labelRule('tracks', { level: 'h3', sm: true })}
      ${ui.trackList(a.trackIds, { dense: true, label: a.title + ' tracks' })}
      ${ui.labelRule('other folders', { level: 'h3', sm: true })}
      <ul class="disc-alts">${alts.map(function (f, i) {
        if (i === idx) return '';
        return html`<li class="disc-alt">
          <span class="disc-alt__peer">${icon('peer')}<b>${f.peer}</b></span>${ui.qualityChip(f.format)}
          <span class="px disc-alt__fact">${f.tracks}/${a.trackIds.length}</span><span class="px disc-alt__fact">${size(f.sizeMB)}</span><span class="px disc-alt__fact">${f.speed} MB/s</span>
          <span class="score" style="--s:${f.score / 100}" role="img" aria-label="Score ${f.score} of 100"><i></i></span>
          <button class="btn btn--sm" data-act="disc-folder-use" data-id="${a.id}" data-i="${i}">Use this folder</button></li>`;
      })}</ul>
    </div>`;
  }
  var folderSheet = null;
  function openFolder(albumId, idx) {
    var a = M.album(albumId); idx = idx || 0;
    var off = null;
    folderSheet = App.openSheet({ title: 'soulseek folder', wide: true, className: 'disc-sheet', body: folderBody(a, idx),
      onMount: function (el) {
        off = App.on('save', function (d) { if (a.trackIds.indexOf(d.id) >= 0) paint('agg', aggInline(a.trackIds, a.title), el); });
      },
      onClose: function () { if (off) off(); folderSheet = null; } });
    folderSheet.albumId = albumId;
    App.syncRows();
    return folderSheet;
  }
  App.action('disc-folder', function (el) { openFolder(el.getAttribute('data-id')); });
  App.action('disc-folder-use', function (el) {
    if (!folderSheet) return;
    var a = M.album(el.getAttribute('data-id')), i = +el.getAttribute('data-i'), alt = folderAlts(a)[i];
    $('.sheet__body', folderSheet.el).innerHTML = String(folderBody(a, i));
    App.syncRows();
    var first = $('.sheet__body button', folderSheet.el); if (first) first.focus();
    App.toast('Using the folder from ' + alt.peer + ', ' + fmt(alt.format).label, { icon: 'soulseek' });
  });

  /* --- source picker --- */
  var FAILED = {};          // trackId -> { peer: 'stalled 2 min ago' } (demo history, plus the player's own failures)
  var pickerSort = 'score', picker = null;
  function failedNotes(t) {
    var notes = Object.assign({}, FAILED[t.id] || {});
    if (P.currentId() === t.id) P.state.failedPeers.forEach(function (f) { notes[f.peer] = 'stalled at ' + f.at + '%'; });
    return notes;
  }
  function sortedSources(t) {
    var list = t.sources.map(function (s, i) { return Object.assign({ i: i }, s); });
    var by = {
      score: function (a, b) { return b.score - a.score; },
      quality: function (a, b) { return fmt(b.format).kbps - fmt(a.format).kbps || b.score - a.score; },
      speed: function (a, b) { return b.speed - a.speed; },
      size: function (a, b) { return sizeFor(a.format, t.duration) - sizeFor(b.format, t.duration); }
    }[pickerSort];
    return list.sort(by);
  }
  function srcRow(t, s, note, first) {
    var f = fmt(s.format), x = s.factors;
    var tip = 'format ' + x.format + ' + speed ' + x.speed + ' + free slot ' + x.slot + ' + queue ' + x.queue + ' = ' + s.score;
    return html`<div class="src-row${note ? ' is-failed' : ''}${f.lossless ? ' is-preferred' : ''}" role="row">
      <span class="disc-src__peer" role="cell"><button class="src-row__peer" data-act="disc-use-source" data-id="${t.id}" data-i="${s.i}" ${first ? raw('autofocus') : ''}
        aria-label="Play from ${s.peer}, ${f.label}, score ${s.score}${note ? ', ' + note : ''}${f.lossless ? ', preferred format' : ''}">${icon('peer')}<span class="ellipsis">${s.peer}</span></button>
        ${note ? html`<span class="disc-src__note">${note}</span>` : ''}</span>
      <span role="cell">${ui.qualityChip(s.format)}</span>
      <span class="src-row__opt" role="cell">${size(sizeFor(s.format, t.duration))}</span>
      <span class="src-row__opt" role="cell">${s.speed} MB/s</span>
      <span class="src-row__opt" role="cell">${icon(s.freeSlot ? 'check' : 'close', s.freeSlot ? 'Free slot' : 'No free slot')}</span>
      <span class="src-row__opt" role="cell">${s.queue}</span>
      <span class="disc-src__score" role="cell" tabindex="0" data-tip="${tip}" aria-label="Score ${s.score} of 100: ${tip}"><span class="score" style="--s:${(s.score / 100).toFixed(2)}"><i></i></span><span class="px">${s.score}</span></span>
    </div>`;
  }
  function pickerRows(t, loading) {
    var notes = failedNotes(t);
    return html`<div class="src-row src-row--head" role="row"><span role="columnheader">peer</span><span role="columnheader">quality</span>
        <span class="src-row__opt" role="columnheader">size</span><span class="src-row__opt" role="columnheader">speed</span>
        <span class="src-row__opt" role="columnheader">slot</span><span class="src-row__opt" role="columnheader">queue</span><span role="columnheader">score</span></div>
      ${sortedSources(t).map(function (s, i) { return srcRow(t, s, notes[s.peer], i === 0); })}
      ${loading ? html`<div class="src-row disc-src__more" role="row"><span role="cell"><span class="spin-ring" aria-hidden="true"></span>Still searching: more peers may answer</span></div>` : ''}`;
  }
  function pickerBody(t, loading) {
    var lossless = t.sources.filter(function (s) { return fmt(s.format).lossless; }).length;
    return html`${sheetHead(t, html`<span class="disc-shead__meta"><span class="px">${plural(t.sources.length, 'source')}</span><span>${lossless} lossless</span></span>`)}
      <div class="disc-picker__bar">
        <span class="disc-picker__label" id="disc-sort-l">Sort by</span>
        <div class="cluster" role="group" aria-labelledby="disc-sort-l">${['score', 'quality', 'speed', 'size'].map(function (k) {
          return ui.filterPill(k.charAt(0).toUpperCase() + k.slice(1), { act: 'disc-src-sort', value: k, pressed: pickerSort === k });
        })}</div>
      </div>
      <div class="src-table disc-src" role="table" aria-label="Sources for ${t.title}" data-region="srcrows">${pickerRows(t, loading)}</div>
      <p class="disc-note disc-picker__hint">Score weighs format, peer speed, a free upload slot and queue length. Hover or focus a score for the breakdown.</p>`;
  }
  function openPicker(trackId, o) {
    o = o || {};
    var t = M.track(trackId); if (!t) return;
    var loading = o.loading != null ? o.loading : (App.current.path === '/search' && streaming());
    pickerSort = 'score';
    var timer = null;
    picker = App.openSheet({ title: 'pick a source', wide: true, className: 'disc-sheet disc-picker', body: pickerBody(t, loading),
      foot: html`<button class="btn" data-close>Cancel</button><button class="btn btn--primary" data-act="disc-play-best" data-id="${t.id}" data-close>${icon('play')}Play best source</button>`,
      onMount: function (el) {
        if (!loading) return;
        /* PROTO: simulated delay */ timer = setTimeout(function () { paint('srcrows', pickerRows(t, false), el); App.announce('Search finished: ' + plural(t.sources.length, 'source')); }, 3500);
      },
      onClose: function () { clearTimeout(timer); picker = null; } });
    picker.trackId = trackId;
    return picker;
  }
  App.hooks.sourcePicker = function (trackId) { openPicker(trackId); };
  App.action('disc-src-sort', function (el) {
    if (!picker) return;
    pickerSort = el.getAttribute('data-value');
    $$('[data-act="disc-src-sort"]', picker.el).forEach(function (b) { b.setAttribute('aria-pressed', String(b.getAttribute('data-value') === pickerSort)); });
    paint('srcrows', pickerRows(M.track(picker.trackId), !!$('.disc-src__more', picker.el)), picker.el);
  });
  function useSource(id, i) {
    var t = M.track(id), s = t.sources[i], again = failedNotes(t)[s.peer];
    P.useSource(id, i);
    App.toast((again ? 'Trying ' + s.peer + ' again, ' : 'Playing from ' + s.peer + ', ') + fmt(s.format).label, { icon: 'soulseek' });
  }
  App.action('disc-use-source', function (el) { var id = el.getAttribute('data-id'); App.closeTop(); useSource(id, +el.getAttribute('data-i')); });
  App.action('disc-play-best', function (el) {
    var t = M.track(el.getAttribute('data-id')), notes = failedNotes(t);
    var best = t.sources.map(function (s, i) { return Object.assign({ i: i }, s); }).filter(function (s) { return !notes[s.peer]; })[0] || { i: 0 };
    useSource(t.id, best.i);
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
          <div><dt>Format</dt><dd>${f0.codec}, ${f0.detail}</dd></div>
          <div><dt>Bitrate</dt><dd class="px">${f0.kbps} kbps</dd></div>
          <div><dt>Size</dt><dd class="px">${size(sizeFor(t.format, t.duration))}</dd></div>
          <div><dt>State</dt><dd>${ui.statusGlyph('ok')} saved, plays at once</dd></div>
        </dl>`;
    }
    var peer = ps.peer, f = fmt(peer.format || t.format), total = sizeFor(peer.format || t.format, t.duration);
    return html`${sheetHead(t)}
      <div class="source-card">
        <div class="source-card__head">${icon('soulseek')}<span>Streaming from <b>${peer.name}</b></span>${ui.qualityChip(f.label, { solid: true })}
          <span class="source-card__stat" data-d="speed">${peer.speed} MB/s</span><span class="source-card__stat" data-d="recv">${Math.round(ps.received * 100)}% of ${size(total)}</span></div>
        ${save ? ui.pipeline(save.step, save.status === 'pending' ? 'saving' : save.status, save.error) : ''}
      </div>
      <dl class="disc-kv">
        <div><dt>Peer</dt><dd><b>${peer.name}</b>, source ${peer.rank} of ${peer.of}</dd></div>
        <div><dt>Format</dt><dd>${f.codec}, ${f.detail}</dd></div>
        <div><dt>Bitrate</dt><dd class="px">${f.kbps} kbps</dd></div>
        <div><dt>Size</dt><dd class="px">${size(total)}</dd></div>
        <div><dt>Speed</dt><dd class="px" data-d="speed">${peer.speed} MB/s</dd></div>
        <div><dt>Received</dt><dd class="px" data-d="bytes">${size(total * ps.received)} of ${size(total)}</dd></div>
        <div><dt>State</dt><dd data-d="state">${ui.statusGlyph(st[0])} ${st[1]}</dd></div>
        <div class="disc-kv__wide"><dt>File</dt><dd class="disc-kv__path">${(t.sources.filter(function (s) { return s.peer === peer.name; })[0] || t.sources[0]).path}</dd></div>
      </dl>
      ${ui.labelRule('fallback history', { level: 'h3', sm: true })}
      ${ps.failedPeers.length ? html`<ol class="disc-fallback">${ps.failedPeers.map(function (fp) {
          return html`<li>${ui.statusGlyph('down')}<span><b>${fp.peer}</b> stalled at ${fp.at}%, switched to the next source</span></li>`;
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
          var total = sizeFor(peer.format || cur.format, cur.duration), stt = xferState();
          $$('[data-d="speed"]', el).forEach(function (n) { n.textContent = peer.speed + ' MB/s'; });
          $$('[data-d="recv"]', el).forEach(function (n) { n.textContent = Math.round(ps.received * 100) + '% of ' + size(total); });
          $$('[data-d="bytes"]', el).forEach(function (n) { n.textContent = size(total * ps.received) + ' of ' + size(total); });
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
  /* a different copy of an already-saved song (same title and artist) */
  function dupOf(id) {
    var t = M.track(id);
    return M.tracks.filter(function (x) { return x.id !== id && App.isSaved(x.id) && x.title === t.title && x.artistId === t.artistId; })[0] || null;
  }
  /* album and queue saves skip the per-song check (their progress shows as an aggregate) */
  App.hooks.beforeSave = function (ids, proceed) {
    var have = ids.length === 1 ? dupOf(ids[0]) : null;
    if (!have) { proceed(ids); return; }
    var t = M.track(ids[0]);
    openDuplicate(have.id, { trackId: t.id, peer: t.sources[0].peer, format: t.sources[0].format, speed: t.sources[0].speed }, function (choice) {
      proceed(choice === 'keep' ? [] : ids);
    });
  };

  var dupDone = null;
  function openDuplicate(savedId, cand, onChoice) {
    var t = M.track(savedId), better = isBetter(cand.format, t.format);
    function card(label, format, sub, isBest, note) {
      var f = fmt(format);
      return html`<div class="disc-dup__card${isBest ? ' is-better' : ''}">
        <p class="disc-dup__label">${label}</p>
        <div class="disc-dup__row">${ui.cover(t, 56)}<div>${ui.qualityChip(format)}<p class="disc-dup__sub">${sub}</p></div></div>
        <dl class="disc-dup__facts"><div><dt>Detail</dt><dd>${f.detail}</dd></div><div><dt>Bitrate</dt><dd class="px">${f.kbps} kbps</dd></div><div><dt>Size</dt><dd class="px">${size(sizeFor(format, t.duration))}</dd></div></dl>
        ${isBest ? html`<p class="disc-dup__best">${icon('arrow-up')}better quality</p>` : note ? html`<p class="disc-dup__keep">${icon('saved')}${note}</p>` : ''}
      </div>`;
    }
    dupDone = { savedId: savedId, cand: cand, onChoice: onChoice || null };
    return App.openDialog({ title: 'already in your library', wide: true, className: 'disc-dup-dialog',
      body: html`<p class="disc-dup__lead"><b>${t.artist}, ${t.title}</b> is already saved as ${fmt(t.format).label}. What should happen to this copy?</p>
        <div class="disc-dup">${card('in your library', t.format, 'Saved, plays from Jellyfin', false, better ? '' : 'same or better')}
          ${card('from ' + cand.peer, cand.format, 'On Soulseek, ' + cand.speed + ' MB/s', better)}</div>
        <p class="disc-note">Set what happens by default in <a class="link" href="#/settings/library" data-close>Settings, Library</a>.</p>`,
      foot: html`<button class="btn" data-act="disc-dup" data-choice="separate" data-close>Save as a separate version</button>
        ${better ? html`<button class="btn" data-act="disc-dup" data-choice="replace" data-close>${icon('arrow-up')}Replace with better quality</button>` : ''}
        <button class="btn btn--primary" data-act="disc-dup" data-choice="keep" data-close autofocus>Keep existing</button>` });
  }
  App.action('disc-dup', function (el) {
    var d = dupDone; dupDone = null; if (!d) return;
    var choice = el.getAttribute('data-choice'), t = M.track(d.savedId), label = fmt(d.cand.format).label;
    if (d.onChoice) { d.onChoice(choice); return; }
    if (choice === 'keep') { App.toast('Kept the saved copy of ' + t.title, { icon: 'saved' }); return; }
    App.toast(choice === 'replace' ? 'Replacing ' + t.title + ' with ' + label + '. The old file is kept until this finishes.' : 'Saving a second copy of ' + t.title + ', ' + label, { icon: 'save' });
    /* PROTO: simulated delay */ setTimeout(function () {
      App.toast(choice === 'replace' ? t.title + ' is now ' + label : 'Saved: ' + t.title + ' (' + label + ' version)', { kind: 'ok', icon: 'saved' });
    }, 4500);
  });
  App.action('disc-save-version', function (el) {
    var t = M.track(el.getAttribute('data-id')), s = t.sources[0];
    openDuplicate(t.id, { trackId: t.id, peer: s.peer, format: s.format, speed: s.speed });
  });

  /* --- save sheet: the 5 steps, the error, and what to do about it --- */
  var RECOVER = {
    transfer: function (id, s) {
      var t = M.track(id), next = t.sources[1] || t.sources[0];
      return { text: s.error || 'The transfer stopped before the file was complete.', actions: html`
        <button class="btn btn--primary" data-act="disc-save-next-source" data-id="${id}">${icon('refresh')}Retry with ${next.peer}</button>
        <button class="btn" data-act="disc-swap-picker" data-id="${id}">${icon('soulseek')}Pick a source</button>` };
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
  function failKind(s) { return s.kind || (s.step === 1 ? 'transfer' : s.step === 2 ? 'match' : 'write'); }
  function saveBody(id) {
    var t = M.track(id), s = App.saves.get(id), saved = App.isSaved(id);
    if (!s && !saved) {
      return html`${sheetHead(t)}<p class="disc-note">Not saved yet. Saving finishes the transfer, matches the song on MusicBrainz, fetches cover art, tags the file and asks Jellyfin to pick it up.</p>`;
    }
    var step = s ? s.step : 5, status = s ? s.status : 'saved';
    var steps = M.saveSteps.map(function (label, i) {
      var n = i + 1, cls = status === 'saved' || n < step ? 'is-done' : n === step ? (status === 'failed' ? 'is-failed' : status === 'pending' ? 'is-wait' : 'is-current') : '';
      var note = n === 1 && cls === 'is-current' && s ? (s.pct || 0) + '%' : n === 3 && s && s.nocover && n <= step ? 'none found, saving without one' : n === 5 && status === 'pending' ? 'Jellyfin is down, retrying every minute' : '';
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
        <div class="disc-recover__actions"><button class="btn" data-act="disc-rescan" data-id="${id}">${icon('refresh')}Retry rescan now</button></div></div></div>` : ''}
      ${status === 'saving' ? html`<p class="disc-note">You can close this. Saving carries on, and Activity shows its progress.</p>` : ''}
      ${status === 'saved' ? html`<p class="disc-note">${t.title} is in your library. Its badge now reads Saved everywhere.</p>` : ''}`;
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
  App.action('disc-save-next-source', function (el) {
    var id = el.getAttribute('data-id'), t = M.track(id), next = t.sources[1] || t.sources[0];
    App.saves.set(id, { step: 1, status: 'saving', pct: 0 });
    App.toast('Retrying the transfer from ' + next.peer, { icon: 'soulseek' });
  });
  App.action('disc-rescan', function (el) {
    App.saves.set(el.getAttribute('data-id'), { step: 5, status: 'saving', pct: 100 });
    App.toast('Asking Jellyfin to rescan', { icon: 'refresh' });
  });

  /* --- MusicBrainz release picker (step 2 had no confident match) --- */
  function releases(t) {
    var a = M.album(t.albumId);
    return [
      { id: 'r1', title: a.title, kind: a.type + ', digital', label: a.label, date: a.released, country: 'Worldwide', tracks: a.trackIds.length },
      { id: 'r2', title: a.title + ' (deluxe)', kind: a.type + ', CD', label: a.label, date: (a.year + 1) + '-02-14', country: 'United Kingdom', tracks: a.trackIds.length + 3 },
      { id: 'r3', title: t.title, kind: 'single, 7 inch vinyl', label: 'Porchlight Press', date: a.year + '-04-30', country: 'Japan', tracks: 2 }
    ];
  }
  function openReleasePicker(id) {
    var t = M.track(id), list = releases(t);
    return App.openDialog({ title: 'pick the right release', wide: true, className: 'disc-rel-dialog',
      body: html`<p class="disc-dup__lead">MusicBrainz found ${list.length} releases that could hold <b>${t.title}</b> by ${t.artist}. Pick the one your file came from.</p>
        <fieldset class="disc-rel"><legend class="vh">Releases</legend>
          <div class="disc-rel__head" aria-hidden="true"><span></span><span>release</span><span>date</span><span>country</span><span>tracks</span></div>
          ${list.map(function (r, i) {
            return html`<label class="disc-rel__row"><input type="radio" name="disc-rel" value="${r.id}" ${i === 0 ? raw('checked') : ''}>
              <span class="disc-rel__title">${r.title}<small>${r.kind}, ${r.label}</small></span>
              <span class="px disc-rel__date">${r.date}</span><span class="disc-rel__country">${r.country}</span><span class="px disc-rel__n">${r.tracks}<span class="vh"> tracks</span></span></label>`;
          })}
        </fieldset>
        <p class="disc-note">Your file: ${App.fmt.time(t.duration)}, ${fmt(t.sources[0].format).label}, track ${t.no}, tagged "${t.album}".</p>`,
      foot: html`<button class="btn" data-act="disc-rel-skip" data-id="${id}" data-close>Save with existing tags</button>
        <button class="btn btn--primary" data-act="disc-rel-use" data-id="${id}" data-close>Use this release</button>` });
  }
  App.action('disc-release', function (el) { openReleasePicker(el.getAttribute('data-id')); });
  App.action('disc-rel-use', function (el) {
    var id = el.getAttribute('data-id'), pickEl = $('input[name="disc-rel"]:checked'), r = releases(M.track(id)).filter(function (x) { return pickEl && x.id === pickEl.value; })[0];
    App.saves.set(id, { step: 3, status: 'saving', pct: 100 });
    App.toast('Matched to ' + (r ? r.title + ' (' + r.date.slice(0, 4) + ')' : 'that release') + '. Saving carries on.', { icon: 'check', kind: 'ok' });
  });
  App.action('disc-rel-skip', function (el) {
    App.saves.set(el.getAttribute('data-id'), { step: 3, status: 'saving', pct: 100 });
    App.toast('Saving with the file\'s own tags', { icon: 'save' });
  });

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
      <div class="progress${c.failed ? '' : ' progress--xfer'}" role="progressbar" aria-label="Saving ${label}" aria-valuemin="0" aria-valuemax="${c.total}" aria-valuenow="${c.saved}"><i style="--p:${(c.saved / c.total).toFixed(3)}"></i></div>
    </div>`;
  }
  function aggBody(ids, label) {
    var c = aggCounts(ids);
    return html`<div class="disc-agg disc-agg--sheet">
        <div class="disc-agg__top">${icon(c.failed ? 'warning' : 'save')}<span class="disc-agg__text">${aggText(c)}</span></div>
        <div class="progress" role="progressbar" aria-label="Saving ${label}" aria-valuemin="0" aria-valuemax="${c.total}" aria-valuenow="${c.saved}"><i style="--p:${(c.saved / c.total).toFixed(3)}"></i></div>
      </div>
      <ul class="disc-aggl">${ids.map(function (id) {
        var t = M.track(id), s = App.saves.get(id), status;
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
     5. Routes, states, public API
     ===================================================================== */
  App.route('/home', { render: renderHome, mount: mountHome, nav: 'home', title: 'Home', meta: function () { return 'updated ' + M.feed.updated; } });
  App.route('/search', { render: renderSearch, mount: mountSearch, nav: 'search', keepFocus: true, meta: function () { return S.src === 'library' ? 'library only' : 'library + soulseek'; },
    title: function () { return S.q ? 'Search: ' + S.q : 'Search'; } });

  var DEMO = 't-satellite-porch-1', ALBUM = 'al-satellite-porch';
  function clearDemo(ids) { (ids || M.album(ALBUM).trackIds).forEach(function (id) { if (App.saves.get(id)) App.saves.set(id, null); }); }
  function demoSave(rec) { return function () { clearDemo(); App.saves.set(DEMO, Object.assign({ frozen: true }, rec)); }; }
  function playDemo() { if (P.currentId() !== DEMO) P.playTrack(DEMO, M.album(ALBUM).trackIds); }

  App.registerStates('discover', [
    st('discover-home', 'Home: discovery feed', '/home'),
    st('discover-home-loading', 'Home: loading skeletons', '/home'),
    st('discover-home-shelf-error', 'Home: one shelf failed (new releases)', '/home'),
    st('discover-home-cold', 'Home: cold start, pick tags and artists', '/home'),
    st('discover-home-no-source', 'Home: pick with no source found yet', '/home'),
    st('discover-home-reason', 'Home: reason popover open', '/home', { open: function () { var c = $('.disc-pick .chip-more') || $('.disc-pick .chip-reason'); if (c) c.click(); } }),
    st('discover-radio', 'Radio: started from a radio starter', '/home', { open: function () { startRadio(M.feed.radioStarters[1].seed); } }),
    st('discover-search-empty', 'Search: empty, recent searches', '/search', { q: '' }),
    st('discover-search-typing', 'Search: typing, Soulseek still answering', '/search', { q: 'sat', p: .38 }),
    st('discover-search-results', 'Search: results (album top result)', '/search', { q: 'satellite' }),
    st('discover-search-dup', 'Search: song top result, better than saved', '/search', { q: 'lantern season' }),
    st('discover-search-none', 'Search: no results', '/search', { q: 'mirrorball' }),
    st('discover-search-slsk-down', 'Search: Soulseek unavailable', '/search', { q: 'satellite',
      apply: function () { App.setService('soulseek', { state: 'down', detail: 'Reconnecting, try 3 of 5' }); } }),
    st('discover-folder', 'Sheet: Soulseek album folder', '/search', { q: 'satellite', open: function () { openFolder(ALBUM); } }),
    st('discover-picker', 'Sheet: source picker', '/search', { q: 'satellite', open: function () { openPicker(DEMO, { loading: false }); } }),
    st('discover-picker-stalled', 'Sheet: source picker, stalled source + still searching', '/search', { q: 'satellite',
      apply: function () { var t = M.track(DEMO); FAILED[DEMO] = {}; FAILED[DEMO][t.sources[1].peer] = 'stalled 2 min ago'; },
      open: function () { openPicker(DEMO, { loading: true }); } }),
    st('discover-details', 'Sheet: source details (streaming)', '/home', { apply: playDemo, open: openDetails }),
    st('discover-details-fallback', 'Sheet: source details with fallback history', '/home', {
      apply: function () {
        playDemo(); var t = M.track(DEMO), s = t.sources[1];
        P.setState({ status: 'streaming', playing: true, received: .62, buffered: [[0, .58]], pos: 84, durationKnown: true,
          failedPeers: [{ peer: t.sources[0].peer, at: 38 }], fallback: { from: t.sources[0].peer, to: s.peer, n: 2, of: t.sources.length },
          peer: { name: s.peer, rank: 2, of: t.sources.length, speed: s.speed, format: s.format, sizeMB: s.sizeMB } });
      }, open: openDetails }),
    st('discover-save-duplicate', 'Save: duplicate dialog (better quality)', '/search', { q: 'lantern season',
      open: function () { var t = M.track('t-lantern-season-1'), s = t.sources[0]; openDuplicate(t.id, { trackId: t.id, peer: s.peer, format: s.format, speed: s.speed }); } }),
    st('discover-save-progress', 'Save: step 2 of 5 in progress', '/home', { apply: demoSave({ step: 2, status: 'saving', pct: 100 }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-fail-transfer', 'Save: transfer failed (step 1)', '/home', { apply: demoSave({ step: 1, status: 'failed', kind: 'transfer', error: 'Transfer failed: the peer stopped answering at 38%' }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-fail-match', 'Save: no MusicBrainz match (step 2)', '/home', { apply: demoSave({ step: 2, status: 'failed', kind: 'match', error: 'No confident MusicBrainz match (3 candidates)' }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-release-picker', 'Save: MusicBrainz release picker', '/home', { apply: demoSave({ step: 2, status: 'failed', kind: 'match', error: 'No confident MusicBrainz match (3 candidates)' }), open: function () { openReleasePicker(DEMO); } }),
    st('discover-save-no-cover', 'Save: no cover art (non-blocking)', '/home', { apply: demoSave({ step: 4, status: 'saving', pct: 100, nocover: true }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-fail-write', 'Save: write failed (disk full)', '/home', { apply: demoSave({ step: 4, status: 'failed', kind: 'write', error: 'Couldn\'t write the file: the library drive is full' }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-pending', 'Save: on disk, waiting for Jellyfin', '/home', { apply: demoSave({ step: 5, status: 'pending', pct: 100 }), open: function () { openSaveSheet(DEMO); } }),
    st('discover-save-album', 'Save: whole album, aggregate progress', '/search', { q: 'satellite',
      apply: function () {
        var ids = M.album(ALBUM).trackIds; clearDemo(ids);
        App.saves.set(ids[0], { step: 5, status: 'saved', pct: 100 }); App.saves.set(ids[1], { step: 5, status: 'saved', pct: 100 });
        App.saves.set(ids[2], { step: 3, status: 'saving', pct: 100, frozen: true });
        App.saves.set(ids[3], { step: 2, status: 'failed', kind: 'match', error: 'No confident MusicBrainz match (3 candidates)', frozen: true });
        App.saves.set(ids[4], { step: 1, status: 'saving', pct: 40, frozen: true });
      }, open: function () { openFolder(ALBUM); } }),
    st('discover-save-queue', 'Save: the queue, aggregate progress', '/home', {
      apply: function () {
        var ids = P.state.queue.map(function (x) { return x.trackId; }).filter(function (id) { return !M.track(id).saved; });
        clearDemo(ids);
        ids.forEach(function (id, i) {
          App.saves.set(id, i === 0 ? { step: 5, status: 'saved', pct: 100 } : i === 1 ? { step: 2, status: 'failed', kind: 'match', error: 'No confident MusicBrainz match', frozen: true }
            : i === 2 ? { step: 4, status: 'saving', pct: 100, frozen: true } : { step: 1, status: 'saving', pct: 30 * i % 100, frozen: true });
        });
      },
      open: function () { openAggregate('your queue', P.state.queue.map(function (x) { return x.trackId; })); } })
  ]);

  App.discover = { openFolder: openFolder, openPicker: openPicker, openDetails: openDetails, openSaveSheet: openSaveSheet,
    openAggregate: openAggregate, openReleasePicker: openReleasePicker, openDuplicate: openDuplicate, startRadio: startRadio };
})();
