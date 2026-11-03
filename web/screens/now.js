/* screens/now.js: Now Playing (device, info stack, source card, lyrics), phone sheet tabs,
   and the Queue / Lyrics / Activity tabs of the context panel.
   Everything about playback renders from App.player.state (the engine); lyrics and source lists come from the
   server; activity from the store ('activity', 'saves', 'activityError').

   Sections: 1 state + helpers, 2 device (full + compact), 3 info stack + source card,
   4 lyrics, 5 queue, 6 activity, 7 route + live updates, 8 actions, 9 panels, 10 the Now Playing column. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$;
  var P = App.player, ps = P.state, fmtTime = App.fmt.time;

  /* =====================================================================
     1. Module state + small helpers
     ===================================================================== */
  var PAGES = ['now', 'source', 'save'];               // device screen pages (select pill cycles them)
  var TABS = ['art', 'lyrics', 'queue', 'details'];    // phone sheet tabs
  var view = { page: 'now', tab: 'art', detailsOpen: false, detailsTrack: null };
  var lyAway = {};                                     // lyrics instance key -> user scrolled away
  var actFilter = 'all';
  var pendingFocus = null;                             // data-fk of the focused control before a re-render

  function cur() { return P.current(); }
  function busy() { return ['finding', 'queued', 'buffering', 'stalled'].indexOf(ps.status) >= 0; }
  function playingNow() { return ps.playing && cur() && ['idle', 'ended', 'failed', 'jellyfin-error'].indexOf(ps.status) < 0; }
  function onRoute() { return App.current.path === '/now'; }
  /* the song plays from the library (Jellyfin), not a Soulseek peer; a saved song buffers before it plays */
  function fromLibrary(t) {
    if (ps.status === 'library' || ps.status === 'jellyfin-error') return true;
    return ps.status === 'buffering' && !ps.peer && App.isSaved(t.id);
  }
  function peerName() { return ps.peer ? ps.peer.name : 'a peer'; }
  /* "2 of 7" ("2/7" with sep '/') once the number of answering peers is known, else "2" */
  function rankOf(n, sep) { return ps.peer && ps.peer.of ? n + (sep || ' of ') + ps.peer.of : String(n); }
  function nextRank() { return (ps.peer ? ps.peer.rank : 1) + 1; }
  function mbps(v) { return Math.round((+v || 0) * 10) / 10 + ' MB/s'; }
  /* a peer that failed: { peer, at } (at = % received, when the server says) */
  function stalledAt(f) { return f.at != null ? f.peer + ' at ' + f.at + '%' : f.peer; }
  /* the ranked source row of the peer now streaming: the one picked by hand, else its row in the fetched list */
  function peerSource(t) {
    if (!t || !ps.peer) return null;
    if (ps.source && ps.source.peer === ps.peer.name) return ps.source;
    return sourceList(t.id).filter(function (s) { return s.peer === ps.peer.name; })[0] || null;
  }

  /* Re-renders keep keyboard focus on the same logical control: focusable things carry data-fk.
     withFocus(key, fn) names the control to focus after fn's re-render (a moved row, the next row). */
  function rememberFocus(host) {
    var a = document.activeElement;
    host._fk = a && host.contains(a) ? a.getAttribute('data-fk') : null;
  }
  function restoreFocus(host) {
    var keys = [pendingFocus, host._fk]; host._fk = null;
    for (var i = 0; i < keys.length; i++) {
      var n = keys[i] && host.querySelector('[data-fk="' + keys[i] + '"]');
      if (n && n.offsetParent !== null) { n.focus({ preventScroll: true }); return; }
    }
  }
  function withFocus(key, fn) { pendingFocus = key; try { fn(); } finally { pendingFocus = null; } }
  function swap(el, markup) {
    if (!el) return;
    rememberFocus(el); App.patch(el, markup); restoreFocus(el);
  }

  /* =====================================================================
     2. Device: screen pages (now / source / save), waveform, D-pad, A/B, pills, grill
     ===================================================================== */
  var LED = { idle: 'off', finding: 'search', choose: 'pick', queued: 'queued', buffering: 'buffer', streaming: 'stream', stalled: 'stalled',
    failed: 'error', library: 'library', ended: 'end', 'jellyfin-error': 'error' };
  function ledText() {
    if (!cur()) return 'off';
    if (ps.crossfade) return 'xfade';
    if (!ps.playing && (ps.status === 'streaming' || ps.status === 'library')) return 'paused';
    return LED[ps.status] || 'stream';
  }
  function headText() {
    if (view.page === 'source') return 'source facts';
    if (view.page === 'save') return 'save pipeline';
    return stateText();
  }
  function stateText() {
    if (!cur()) return 'nothing playing';
    return { finding: 'finding a source', queued: 'queued at peer', buffering: 'buffering', stalled: 'peer stalled',
      failed: 'couldn\'t play', 'jellyfin-error': 'unavailable', ended: 'end of queue' }[ps.status] ||
      (ps.playing ? 'now playing' : 'paused');
  }
  function screenBtn(label, act, id) {
    return html`<button class="now-sbtn" data-act="${act}" ${id ? raw('data-id="' + App.esc(id) + '"') : ''} data-fk="sbtn-${act}">${label}</button>`;
  }
  function nowFacts(t) {
    var chip = ui.qualityChip(fromLibrary(t) || !ps.peer ? t.format : ps.peer.format || t.format, { solid: true });
    switch (ps.status) {
      case 'finding': return html`${chip}<span class="device__fact"><span class="spin-ring" aria-hidden="true"></span>${ps.peersFound}&nbsp;peers</span>`;
      case 'queued': return html`${chip}<span class="device__fact">${ps.queuePos ? 'queue #' + ps.queuePos : 'queued'}</span>`;
      case 'stalled': return html`${chip}<span class="device__fact">next ${rankOf(nextRank(), '/')}</span>`;
      case 'failed': return html`${screenBtn('retry', 'retry-search')}${screenBtn('pick source', 'pick-source')}`;
      case 'jellyfin-error': return screenBtn('try soulseek', 'try-soulseek');
      case 'ended': return html`${screenBtn('replay', 'play')}${screenBtn('radio', 'start-radio-current')}`;
      default:
        if (fromLibrary(t)) return html`${chip}<span class="device__fact">library</span>`;
        return html`${chip}<span class="device__fact">peer&nbsp;<span data-bind="received">${Math.round(ps.received * 100)}%</span></span>`;
    }
  }
  function pageNow(t) {
    if (!t) return html`<div class="now-dev__idle"><span class="now-dev__slot" aria-hidden="true"></span><p class="device__title">Nothing playing</p><p class="device__artist">Search or pick something from Home</p></div>`;
    return html`<div class="device__now">${ui.cover(t, 'var(--now-art)', { hero: true })}<div class="now-dev__meta">
      <p class="device__title">${t.title}</p><p class="device__artist">${t.artist}</p>
      <div class="device__facts">${nowFacts(t)}</div></div></div>`;
  }
  function kv(rows) {
    return html`<dl class="device__kv">${rows.map(function (r) { return html`<dt>${r[0]}</dt><dd>${r[1]}</dd>`; })}</dl>`;
  }
  function pageSource(t) {
    if (!t) return kv([['source', 'none']]);
    if (fromLibrary(t)) {
      return kv([['source', 'jellyfin'], ['format', t.format], ['size', App.fmt.size(t.sizeMB)], ['state', ps.status === 'jellyfin-error' ? 'missing' : 'saved']]);
    }
    if (!ps.peer) return kv([['source', 'soulseek'], ['state', stateText()], ['peers', ps.peersFound]]);
    var s = peerSource(t), last = ps.failedPeers[ps.failedPeers.length - 1];
    return kv([
      ['peer', ps.peer.name], ['rank', rankOf(ps.peer.rank)], ['speed', mbps(ps.status === 'stalled' ? 0 : ps.peer.speed)],
      s ? ['slot', s.freeSlot ? 'free' : 'queue ' + (s.queue || 0)] : ['peers', ps.peersFound],
      ['received', raw('<span data-bind="received">' + Math.round(ps.received * 100) + '%</span>')],
      last ? ['fallback', stalledAt(last)] : ['format', ps.peer.format || t.format]
    ]);
  }
  function pageSave(t) {
    if (!t) return html`<p class="now-dev__note">nothing to save</p>`;
    var s = App.saves.get(t.id);
    if (s) return html`<div class="now-dev__pipe">${ui.pipeline(s.step, s.status, s.error)}</div>`;
    if (App.isSaved(t.id)) return html`<p class="now-dev__note">${icon('saved')}in your library</p>${screenBtn('show album', 'go-album', t.id)}`;
    return html`<p class="now-dev__note">not saved yet</p>${screenBtn('save', 'save', t.id)}`;
  }
  function factsLine() { var t = cur(); return t ? String(ui.sourceBadge(t)) + String(nowFacts(t)) + (ps.belowMinimum ? String(ui.belowBadge()) : '') : ''; }
  function pageBody(t) { return view.page === 'source' ? pageSource(t) : view.page === 'save' ? pageSave(t) : pageNow(t); }
  function slide(t) {
    return html`<div class="now-slide"><div class="now-dev__page" data-now="page">${pageBody(t)}</div>
      ${ui.wave(t ? t.id : 'idle', 48)}${ui.waveTimes()}</div>`;
  }
  function selectLabel() { var next = PAGES[(PAGES.indexOf(view.page) + 1) % PAGES.length]; return 'Screen page: ' + view.page + '. Show ' + next + ' page'; }

  function playBtn(size) {
    var on = playingNow();
    return html`<button class="btn-gloss btn-gloss--${size}${busy() && ps.playing ? ' is-buffering' : ''}" data-act="play" data-bind="play" aria-label="${on ? 'Pause' : 'Play'}" data-tip="${on ? 'Pause (Space)' : 'Play (Space)'}" data-fk="a-${size}"><i class="btn-gloss__spin" aria-hidden="true"></i>${icon(on ? 'pause' : 'play', null, on ? '' : 'ic-play')}</button>`;
  }
  function likeBtn(size) {
    var t = cur(), on = t && App.isLiked(t.id);
    if (!t) return html`<button class="btn-gloss btn-gloss--${size}" data-act="now-nothing" aria-label="Like" data-fk="b-${size}">${icon('heart')}</button>`;
    return html`<button class="btn-gloss btn-gloss--${size}${on ? ' is-liked' : ''}" data-act="like" data-id="${t.id}" data-like="${t.id}" aria-pressed="${on ? 'true' : 'false'}" aria-label="Like (L)" data-tip="${on ? 'Liked' : 'Like'}" data-fk="b-${size}">${icon(on ? 'heart-fill' : 'heart')}</button>`;
  }
  function repeatLabel() { return { off: 'repeat', all: 'repeat', one: 'repeat 1' }[ps.repeat]; }

  function deviceFull() {
    var t = cur();
    return html`<div class="device now-dev" role="group" aria-label="Player device" style="--device-w:100%">
      <span class="device__led" aria-hidden="true"><i></i><span data-now="led">${ledText()}</span></span>
      <div class="device__bezel">
        <div class="device__screen s-screen">
          <div class="device__page-head"><span data-now="head">${headText()}</span>
            <button class="device__select" data-act="now-page" aria-label="${selectLabel()}" data-tip="Next screen page" data-fk="select">${view.page}${icon('chevron-right', null, 'ic-16')}</button></div>
          <div class="now-dev__host" data-now="screen">${slide(t)}</div>
          <div class="now-osd" data-now="osd" aria-hidden="true"></div>
        </div>
        <div class="device__brand">${ui.wordmark()}</div>
      </div>
      <div class="device__controls">
        <div class="dpad" role="group" aria-label="D-pad">
          <button class="dpad__btn dpad__btn--up" data-act="now-vol" data-dir="1" aria-label="Volume up (Up arrow)" data-tip="Volume up" data-fk="up">${icon('plus')}</button>
          <button class="dpad__btn dpad__btn--left" data-act="prev" aria-label="Previous (P)" data-tip="Previous" data-fk="left">${icon('prev')}</button>
          <span class="dpad__hub" aria-hidden="true"></span>
          <button class="dpad__btn dpad__btn--right" data-act="next" aria-label="Next (N)" data-tip="Next" data-fk="right">${icon('next')}</button>
          <button class="dpad__btn dpad__btn--down" data-act="now-vol" data-dir="-1" aria-label="Volume down (Down arrow)" data-tip="Volume down" data-fk="down">${icon('minus')}</button>
        </div>
        <div class="device__ab">
          <span class="device__key device__key--b"><span data-now="b">${likeBtn(52)}</span><span data-now="b-label" aria-hidden="true">${t && App.isLiked(t.id) ? 'liked' : 'like'}</span></span>
          <span class="device__key device__key--a"><span data-now="a">${playBtn(64)}</span><span data-now="a-label" aria-hidden="true">${playingNow() ? 'pause' : 'play'}</span></span>
        </div>
      </div>
      <div class="device__pills">
        <button class="device__pill device__pill--shuffle" data-act="shuffle" aria-pressed="${String(ps.shuffle)}" aria-label="Shuffle (S)" data-fk="shuffle"><i></i>shuffle</button>
        <button class="device__pill device__pill--repeat" data-act="repeat" aria-pressed="${String(ps.repeat !== 'off')}" aria-label="${{ off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat]} (R)" data-fk="repeat"><i></i><span data-now="repeat">${repeatLabel()}</span></button>
      </div>
      <div class="device__grill" aria-hidden="true">${raw(new Array(13).join('<i></i>'))}</div>
    </div>`;
  }
  /* nowHero slot: the Now Playing hero object. Default = the handheld device (full) or its one-row bezel (compact).
     A design.js override gets ctx = { variant: 'full'|'compact', track, player, parts } and may reuse
     parts.screen(), parts.playButton(size), parts.likeButton(size), parts.wave(bars), parts.device() / parts.compact().
     parts.facts() = source badge + quality chip + peer fact, parts.state() = the status word ("buffering", "peer stalled").
     Elements carrying data-now="screen|c-screen|c-art|a|b|ca|cb|led|head|state|facts|repeat|osd|page" stay live (see section 7). */
  function hero(variant) {
    var t = cur();
    var ctx = { variant: variant, track: t, player: ps, parts: {
      screen: function () { return slide(cur()); }, playButton: playBtn, likeButton: likeBtn,
      wave: function (bars) { return ui.wave(t ? t.id : 'idle', bars); }, device: deviceFull, compact: deviceCompact,
      facts: factsLine, state: stateText } };
    return raw(String(App.slot('nowHero', ctx, function (c) { return c.variant === 'compact' ? deviceCompact() : deviceFull(); })));
  }
  /* compact device: one bezel row (phone Lyrics / Queue / Details tabs, short landscape) */
  function compactSlide(t) {
    return html`<div class="now-slide">${t ? html`<p class="device__title">${t.title}</p><p class="device__artist">${t.artist}</p>`
      : html`<p class="device__title">Nothing playing</p><p class="device__artist">Pick something from Home</p>`}${ui.wave(t ? t.id : 'idle', 26)}</div>`;
  }
  function deviceCompact() {
    var t = cur();
    return html`<div class="device device--compact now-devc" role="group" aria-label="Player">
      <div class="device__bezel">
        <span class="now-devc__art" data-now="c-art">${t ? ui.cover(t, 56) : html`<span class="now-dev__slot" aria-hidden="true"></span>`}</span>
        <div class="device__screen s-screen" data-now="c-screen">${compactSlide(t)}</div>
        <div class="device__keys now-devc__keys">
          <span class="now-devc__skip"><button class="dpad__btn" data-act="prev" aria-label="Previous (P)" data-tip="Previous" data-fk="c-left">${icon('prev', null, 'ic-16')}</button><button class="dpad__btn" data-act="next" aria-label="Next (N)" data-tip="Next" data-fk="c-right">${icon('next', null, 'ic-16')}</button></span>
          <span data-now="cb">${likeBtn(40)}</span><span data-now="ca">${playBtn(52)}</span>
        </div>
      </div>
    </div>`;
  }

  /* track change: screen content slides out left and the new track slides in from the right */
  function slideSwap(host, makeMarkup) {
    if (!host) return;
    var old = host.firstElementChild;
    if (App.reducedMotion() || !old) { host.innerHTML = String(makeMarkup()); return; }
    old.classList.add('is-out');
    clearTimeout(host._t);
    host._t = setTimeout(function () {
      host.innerHTML = String(makeMarkup());
      var n = host.firstElementChild; n.classList.add('is-in'); n.getBoundingClientRect(); n.classList.remove('is-in');
    }, 120);
  }
  var osdTimer = null;
  function showOsd() {
    var v = ps.muted ? 0 : Math.round(ps.volume * 100);
    $$('[data-now="osd"]').forEach(function (o) {
      o.innerHTML = '<span>vol</span><span class="now-osd__bar" style="--v:' + v / 100 + '"><i></i></span><span>' + v + '%</span>';
      o.classList.add('is-on');
    });
    clearTimeout(osdTimer);
    osdTimer = setTimeout(function () { $$('[data-now="osd"]').forEach(function (o) { o.classList.remove('is-on'); }); }, 1200);
  }

  /* =====================================================================
     3. Info stack: playing-from label, title block, chips, action row, source card, up next
     ===================================================================== */
  /* inside every .now-head__from (the info head, the phone sheet's top bar, a design's hero: the playing-from part):
     "playing from <place>", a link that opens it at the song (App.goPlaying) */
  function fromLine() {
    return ui.playingFrom('', 'playing from') || html`${icon('sparkle', null, 'ic-16')}<span>nothing queued</span>`;
  }
  /* why this song: its pick in the home feed (when Home has loaded it), or the artist radio it came from */
  function reasonFor(t) {
    var feed = M.peek('/api/feed');
    var pick = feed && (feed.picks || []).filter(function (p) { return p.trackId === t.id; })[0];
    if (pick && pick.reasons && pick.reasons.length) return ui.reasonChip(pick.reasons[0], { more: pick.reasons.length - 1, via: pick.via, itemId: t.id });
    if (ps.radio && ps.radio.seed && ps.radio.seed.kind === 'artist' && t.artistId !== ps.radio.seed.id) {
      return ui.reasonChip({ kind: 'like', name: ps.radio.name, ref: ps.radio.seed.id }, { itemId: t.id });
    }
    return '';
  }
  function infoHead() {
    var t = cur();
    if (!t) return html`<p class="now-head__from">${fromLine()}</p><h1 class="t-title-xl now-head__title">Nothing playing</h1>`;
    var a = M.album(t.albumId);
    return html`<p class="now-head__from">${fromLine()}</p>
      <h1 class="t-title-xl now-head__title">${t.title}</h1>
      <p class="now-head__by"><a class="link" href="#/artist/${t.artistId}">${t.artist}</a>, <a class="link-plain" href="#/album/${t.albumId}">${t.album}${a && a.type !== 'album' ? ' ' + a.type : ''}</a>${t.year ? ', ' + t.year : ''}</p>
      <div class="now-head__chips">${t.tags.map(function (g) { return ui.tagChip(g); })}${reasonFor(t)}</div>`;
  }
  function actionRow() {
    var t = cur();
    if (!t) return html`<div class="now-acts"><a class="btn btn--primary" href="#/home">${icon('home')}Go home</a><a class="btn" href="#/search">${icon('search')}Search</a>${actBtn2('connect', 'Play on another device', 'connect', 'aria-haspopup="dialog"', 'Devices')}</div>`;
    var following = App.isFollowed(t.artistId);
    return html`<div class="now-acts">
      ${ui.saveButton(t.id, { cls: 'btn--primary', label: 'Save to library' })}
      ${actBtn2('plus', 'Add to playlist', 'add-to-playlist', 'data-id="' + t.id + '" aria-haspopup="menu"')}
      ${actBtn2('radio', 'Start radio', 'start-radio', 'data-kind="track" data-id="' + t.id + '"')}
      ${actBtn2('soulseek', 'Choose another source', 'pick-source', 'aria-haspopup="dialog"', 'Source')}
      ${actBtn2(following ? 'check' : 'follow', following ? 'Following ' + t.artist : 'Follow ' + t.artist, 'follow', 'data-id="' + t.artistId + '" aria-pressed="' + following + '"', following ? 'Following' : 'Follow')}
      ${actBtn2('connect', 'Play on another device', 'connect', 'aria-haspopup="dialog"', 'Devices')}
      ${actBtn2('classic', 'Classic mode', 'classic', '', 'Classic')}
    </div>`;
  }

  function actBtn2(ic, label, act, attrs, short) {
    return html`<button class="btn now-acts__btn" data-act="${act}" ${raw(attrs)} aria-label="${label}" data-tip="${label}" data-fk="act-${act}">${icon(ic)}<span class="now-acts__label" aria-hidden="true">${short || label}</span></button>`;
  }

  /* source card: where the audio comes from + the 5-step save pipeline + a details expander */
  function sourceHead(t) {
    var src = peerSource(t);
    var size = (ps.peer && ps.peer.sizeMB) || (src && src.sizeBytes / 1e6) || t.sizeMB;
    /* buffering after a peer fallback, or from the library: the engine's own sentence says which */
    if (ps.status === 'buffering' && (ps.fallback || fromLibrary(t))) return html`${icon(ps.fallback ? 'refresh' : 'saved')}<span>${P.statusText()}</span>`;
    switch (ps.status) {
      case 'library': return html`${icon('saved')}<span>Playing from <b>your library</b></span>${ui.qualityChip(t.format, { solid: true })}<span class="source-card__stat">jellyfin</span>`;
      case 'jellyfin-error': return html`${icon('warning')}<span><b>Library item unavailable.</b> ${ps.error || 'Jellyfin can\'t find the file.'}</span><button class="btn btn--sm" data-act="try-soulseek" data-fk="src-try">Try Soulseek instead</button>`;
      case 'finding': return html`${icon('soulseek')}<span>Finding a source, <b>${ps.peersFound}</b> peers answered</span>`;
      case 'choose': return html`${icon('warning')}<span><b>No copy meets your streaming minimum.</b> Pick one or skip.</span><button class="btn btn--sm" data-act="pick-source" data-fk="src-choose">Pick a copy</button>`;
      case 'queued': return html`${icon('clock')}<span>Queued at <b>${peerName()}</b>${ps.queuePos ? ', position ' + ps.queuePos : ''}</span>`;
      case 'failed': return html`${icon('warning')}<span><b>${P.statusText()}</b></span><button class="btn btn--sm" data-act="retry-search" data-fk="src-retry">Retry search</button>`;
      case 'stalled': return html`${icon('refresh')}<span><b>${peerName()}</b> stalled, switching to source ${rankOf(nextRank())}</span><button class="btn btn--sm" data-act="pick-source" data-fk="src-pick">Choose manually</button>`;
      case 'ended': return html`${icon('info')}<span>End of queue</span><button class="btn btn--sm" data-act="start-radio-current" data-fk="src-radio">Start radio</button>`;
      default: return html`${icon('soulseek')}<span>Streaming from <b>${peerName()}</b></span>${ui.qualityChip(ps.peer && ps.peer.format || t.format, { solid: true })}${ps.belowMinimum ? ui.belowBadge() : ''}<span class="source-card__stat">${mbps(ps.peer && ps.peer.speed)}</span><span class="source-card__stat"><span data-bind="received">${Math.round(ps.received * 100)}%</span> of ${App.fmt.size(size)}</span>`;
    }
  }
  function saveState(t) {
    var s = App.saves.get(t.id);
    if (s) return ui.pipeline(s.step, s.status, s.error);
    if (App.isSaved(t.id)) return ui.pipeline(5, 'saved');
    // not saved: five empty steps (pending = outline only); --idle lets a design style it without :has()
    return html`<div class="pipeline-wrap pipeline-wrap--idle" role="group" aria-label="Save progress"><div class="pipeline">${raw(new Array(6).join('<span class="pipeline__seg" aria-hidden="true"></span>'))}</div>
      <div class="pipeline__text"><span class="pipeline__step">not saved</span><span class="pipeline__label">Save keeps a tagged copy in your library</span></div></div>`;
  }
  function sourceDetails(t) {
    var src = peerSource(t), lib = fromLibrary(t), known = sources[t.id] && sources[t.id].data;
    var rows = [['Album', html`<a class="link" href="#/album/${t.albumId}">${t.album}</a>${t.year ? ', ' + t.year : ''}`],
      ['Format', (!lib && ps.peer && ps.peer.format) || t.format || 'unknown']];
    if (!lib && ps.peer) {
      rows.push(['Peer', ps.peer.name + ', source ' + rankOf(ps.peer.rank)]);
      if (src) rows.push(['Upload slot', src.freeSlot ? 'Free now' : 'Queue of ' + (src.queue || 0)]);
      rows.push(['Speed', mbps(ps.peer.speed)]);
    } else rows.push(['Source', lib ? 'Jellyfin library' : 'Soulseek, ' + stateText()]);
    rows.push(['Size', App.fmt.size((ps.peer && ps.peer.sizeMB) || t.sizeMB)]);
    if (ps.failedPeers.length) rows.push(['Fallbacks', ps.failedPeers.map(stalledAt).join(', ')]);
    if (!lib && ps.upgrade) rows.push(['Better copy', betterCopy(ps.upgrade)]);
    var label = view.detailsTrack ? 'Hide sources' : known ? 'See all ' + (known.sources || []).length + ' sources' : 'See all sources';
    return html`<dl class="now-src__dl">${rows.map(function (r) { return html`<div><dt>${r[0]}</dt><dd>${r[1]}</dd></div>`; })}</dl>
      <div class="now-src__more"><button class="btn btn--sm" data-act="now-other-sources" data-id="${t.id}" aria-expanded="${String(!!view.detailsTrack)}" data-fk="src-list">${icon('soulseek')}${label}</button></div>
      ${view.detailsTrack ? sourcesTable(view.detailsTrack) : ''}`;
  }
  /* the better copy the hub fetches beside a lesser one, and whether the player switched to it mid-song
     (docs/context/web.md#better-copies) */
  function betterCopy(u) {
    var copy = (u.format || 'A better copy') + ' from ' + u.peer;
    switch (u.state) {
      case 'arriving': return copy + ', ' + Math.round(u.pct * 100) + '% here. The song switches to it once enough has arrived.';
      case 'ready': return copy + ' is here. The song switches to it at a quiet moment.';
      case 'switching': return 'Lining ' + copy + ' up with the song.';
      case 'switched': return 'Switched from ' + (u.from || 'the first copy') + ' to ' + copy + ' at ' + App.fmt.time(u.at) + '.';
      default: return copy + '. ' + (u.note || 'It plays from the next play.');
    }
  }
  /* ranked sources per song (GET /api/tracks/{id}/sources), fetched when the list opens. Kept here rather than in an
     M.get read: a settled M.get re-renders the whole route (replaying the device's power-on); this swaps the card. */
  var sources = {};
  function loadSources(id) {
    var r = sources[id] = { loading: true, error: null, data: sources[id] ? sources[id].data : null };
    M.api('/api/tracks/' + encodeURIComponent(id) + '/sources').then(function (d) { r.data = d || {}; }, function (err) { r.error = err; })
      .then(function () { r.loading = false; if (sources[id] === r) refreshSourceCard(); });
    return r;
  }
  function sourcesRead(id) { return sources[id] || loadSources(id); }
  function sourceList(id) { var r = sources[id]; return r && r.data && r.data.sources || []; }
  function refreshSourceCard() { swap($('[data-now="src"]'), sourceCard()); swap($('[data-nowcol="src"]'), sourceCard()); }
  function sourcesTable(id) {
    var t = M.track(id), r = sourcesRead(id);
    var head = t && id !== P.currentId() ? html`<p class="now-src__line">${icon('info')}<span>Sources for <b>${t.title}</b>, ${t.artist}</span></p>` : '';
    var reload = function (label) { return html`<div class="now-src__more"><button class="btn btn--sm" data-act="now-src-reload" data-id="${id}" data-fk="src-reload">${label}</button></div>`; };
    if (r.error) return html`${head}<p class="now-src__line" role="alert">${icon('warning')}<span>${r.error.message}</span></p>${reload('Try again')}`;
    if (!r.data) return html`${head}<div class="now-src__table" role="status"><span class="vh">Looking for sources</span>${ui.skeleton('row', 3)}</div>`;
    var list = r.data.sources || [];
    var rows = list.map(function (s, i) {
      var on = !!(ps.peer && ps.peer.name === s.peer && id === P.currentId());
      return html`<div class="src-row${s.preferred ? ' is-preferred' : ''}${s.stalledBefore ? ' is-failed' : ''}" role="listitem"><span class="src-row__peer">${icon('peer')}${s.peer}</span><span>${ui.qualityChip(s.format)}</span><span class="src-row__opt">${mbps(s.speed)}</span><span class="score" style="--s:${(s.score || 0) / 100}" data-tip="score ${s.score}"><i></i></span>
        <button class="btn btn--sm" data-act="now-use-source" data-id="${id}" data-si="${i}" aria-label="${on ? 'Playing' : 'Use'} ${s.peer}${s.stalledBefore ? ', stalled before' : ''}" ${on ? raw('aria-current="true"') : ''} data-fk="use-${i}">${on ? 'Playing' : 'Use'}</button></div>`;
    });
    return html`${head}${list.length ? html`<div class="src-table now-src__table" role="list" aria-label="Sources for ${t ? t.title : 'this song'}">${rows}</div>`
      : html`<p class="now-src__line">${icon('info')}<span>No peer has this song right now.</span></p>`}
      ${r.data.done ? '' : html`<p class="now-src__line">${icon('soulseek')}<span>Still searching Soulseek, more peers may answer.</span></p>${reload('Refresh')}`}`;
  }
  function sourceCard() {
    var t = cur();
    if (!t) return html`<div class="source-card"><p class="now-src__line">${icon('info')}<span>Nothing is playing, so there is no source yet.</span></p></div>`;
    var open = view.detailsOpen || view.tab === 'details';
    return html`<div class="source-card">
        <div class="source-card__head">${sourceHead(t)}</div>
        <div class="now-src__save" data-now="pipeline">${saveState(t)}</div>
      </div>
      <div class="now-src__foot">
        <button class="now-src__toggle" data-act="now-details" aria-expanded="${String(open)}" aria-controls="now-src-details" data-fk="src-toggle">${icon(open ? 'chevron-up' : 'chevron-down', null, 'ic-16')}${open ? 'Hide details' : 'Source details'}</button>
        <button class="btn btn--quiet btn--sm" data-act="pick-source" data-fk="src-pick">Pick another source</button>
      </div>
      <div class="now-src__details" id="now-src-details" ${open ? '' : raw('hidden')}>${open ? sourceDetails(t) : ''}</div>`;
  }

  /* up next preview (2 songs) with a link into the queue */
  function upNext() {
    var rows = [], i = ps.index + 1;
    while (rows.length < 2 && i < ps.queue.length) { if (M.track(ps.queue[i].trackId)) rows.push(qRow(ps.queue[i], i, { preview: true })); i++; }
    return html`${ui.labelRule('up next', { action: { label: 'Queue', act: 'now-open-queue' } })}
      ${rows.length ? html`<ol class="tlist tlist--dense now-q__list" aria-label="Up next">${rows}</ol>`
        : html`<p class="now-next__none">${icon('sparkle', null, 'ic-16')}Nothing up next.</p><button class="btn btn--sm" data-act="start-radio-current">${icon('radio')}Start radio</button>`}`;
  }

  /* =====================================================================
     4. Lyrics (LRCLIB via the server): synced, plain, instrumental, none, loading, error (503 = rate-limited)
     ===================================================================== */
  /* GET /api/lyrics/{id} -> { kind: synced|plain|instrumental|none, lines [{ t, text }], plain, source }, once per song.
     Not an M.get read: the Lyrics panel shows on every route and a settled M.get only re-renders the route;
     this re-renders the lyrics blocks wherever they are. */
  var lyrics = {};
  function lyricsRead(id) {
    var r = lyrics[id];
    if (r) return r;
    r = lyrics[id] = { loading: true, error: null, data: null };
    M.api('/api/lyrics/' + encodeURIComponent(id)).then(function (d) { r.data = d || {}; }, function (err) { r.error = err; })
      .then(function () {
        if (lyrics[id] !== r || P.currentId() !== id) { r.loading = false; return; }
        var was = lyricsShown; r.loading = false;
        refreshLyrics();
        if (lyricsAvailable() !== was) lyricsTabsChanged();
      });
    return r;
  }
  /* A song with no words (no lyrics found, or instrumental) gets no Lyrics tab, in the panel or the phone sheet.
     While the next song's lyrics load, the tabs keep the last answer, so they only change when it does. A failed
     read keeps the tab, where "Try again" is. */
  var lyricsShown = true;
  function lyricsAvailable() {
    var t = cur(); if (!t) return lyricsShown;
    if (lyricsRead(t.id).data) { var st = lyricsFor(t).state; lyricsShown = st !== 'none' && st !== 'instrumental'; }
    return lyricsShown;
  }
  function lyricsTabsChanged() {
    App.refreshPanel();
    var seg = $('.now .now__seg'); if (!seg) return;
    if (tabList().indexOf(view.tab) < 0) view.tab = 'art';
    swap(seg, phoneTabs()); setTab(view.tab, false);
  }
  App.on('player', function (kind) { if (kind === 'track') { var was = lyricsShown; if (lyricsAvailable() !== was) lyricsTabsChanged(); } });
  function lyricsFor(t) {
    var r = lyricsRead(t.id), d = r.data;
    if (r.error) return { state: 'error', error: r.error };
    if (!d) return { state: 'loading' };
    var lines = d.lines || [];
    var text = d.plain || lines.map(function (l) { return l.text; }).join('\n');
    if (d.kind === 'synced' && lines.length) return { state: 'synced', lines: lines, source: d.source };
    if (d.kind === 'synced' || d.kind === 'plain') return { state: text ? 'plain' : 'none', text: text, source: d.source };
    if (d.kind === 'instrumental' || d.kind === 'none') return { state: d.kind, source: d.source };
    return { state: 'error', error: { message: 'The server sent lyrics of an unknown kind (' + d.kind + ').' } };
  }
  function lyStep() { var s = +(App.storage('lyricStep') || 0); return App.clamp(isNaN(s) ? 0 : s, -2, 2); }
  function lyTag(L) {
    var label = { synced: 'synced', plain: 'not synced' }[L.state];
    return label ? html`<span class="now-ly__tag">${label}</span>` : '';
  }
  function lyTools() {
    var s = lyStep();
    return html`<span class="now-ly__size" role="group" aria-label="Lyrics text size">
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-ly-size" data-dir="-1" aria-label="Smaller lyrics" data-tip="Smaller text" ${s <= -2 ? raw('disabled') : ''} data-fk="ly-minus"><span class="now-ly__a" aria-hidden="true">A-</span></button>
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-ly-size" data-dir="1" aria-label="Larger lyrics" data-tip="Larger text" ${s >= 2 ? raw('disabled') : ''} data-fk="ly-plus"><span class="now-ly__a" aria-hidden="true">A+</span></button></span>`;
  }
  function lyLines(L, key) {
    var i = currentLine(L.lines), first = firstSung(L.lines);
    return L.lines.map(function (l, n) {
      if (!l.text) return html`<li class="now-ly__gap" aria-hidden="true"><i></i><i></i><i></i></li>`;
      return html`<li><button class="now-ly__line ${n < i ? 'is-past' : n === i ? 'is-current' : ''}" data-t="${l.t}" data-i="${n}" tabindex="${n === Math.max(i, first) ? 0 : -1}" ${n === i ? raw('aria-current="true"') : ''} data-fk="ly-${key}-${n}">${l.text}</button></li>`;
    });
  }
  function firstSung(lines) { for (var i = 0; i < lines.length; i++) if (lines[i].text) return i; return 0; }
  function currentLine(lines) {
    var p = ps.pos + .15, i = -1;
    for (var n = 0; n < lines.length; n++) { if (lines[n].t <= p) i = n; else break; }
    return i;
  }
  /* key: 'page' (Now Playing card, also the phone Lyrics tab) or 'panel' (context panel) */
  function lyricsBlock(key) {
    var t = cur(), body;
    if (!t) return html`<div class="now-ly now-ly--${key}">${ui.emptyState({ art: 'wave', title: 'Nothing playing', text: 'Lyrics show up here while a song plays.' })}</div>`;
    var L = lyricsFor(t);
    if (L.state === 'loading') body = html`<div class="now-ly__state" role="status"><span class="vh">Loading lyrics</span>${ui.skeleton('text', 7)}</div>`;
    else if (L.state === 'error') body = html`<div class="now-ly__state">${ui.emptyState({ art: 'star', title: 'Lyrics unavailable right now', text: L.error.message, action: { label: 'Try again', act: 'now-ly-retry' } })}</div>`;
    else if (L.state === 'none') body = html`<div class="now-ly__state">${ui.emptyState({ art: 'slot', title: 'No lyrics found', text: 'LRCLIB has nothing for this song yet. Check again later.', action: { label: 'Check again', act: 'now-ly-retry' } })}</div>`;
    else if (L.state === 'instrumental') body = html`<div class="now-ly__state">${ui.emptyState({ art: 'wave', title: 'Instrumental', text: 'No words in this one. Just listen.' })}</div>`;
    else if (L.state === 'plain') body = html`<div class="now-ly__view"><div class="now-ly__scroll now-ly__plain" tabindex="0" aria-label="Lyrics, not synced">${L.text.split('\n\n').map(function (st) {
      return html`<p>${st.split('\n').map(function (ln, i) { return html`${i ? raw('<br>') : ''}${ln}`; })}</p>`;
    })}</div></div>`;
    else body = html`<div class="now-ly__view">
        <div class="now-ly__scroll" data-sync data-key="${key}" data-track="${t.id}">
          <ol class="now-ly__lines" aria-label="Lyrics, synced. Choose a line to jump there.">${lyLines(L, key)}</ol>
        </div>
        <button class="btn btn--sm btn--primary now-ly__back" data-act="now-ly-back" hidden data-fk="ly-back-${key}">${icon('arrow-up', null, 'ic-16')}Back to current line</button>
      </div>`;
    return html`<div class="now-ly now-ly--${key}" data-state="${L.state}" style="--ly-step:${lyStep()}">
      ${key === 'page' ? html`<div class="now-ly__head">${ui.labelRule('lyrics')}${lyTag(L)}</div>` : html`<div class="now-ly__head">${lyTag(L)}<span class="spacer"></span></div>`}
      ${body}
      <div class="now-ly__foot"><span class="now-ly__attr">Lyrics provided by ${L.source || 'LRCLIB'}</span>${L.state === 'synced' || L.state === 'plain' ? lyTools() : ''}</div>
    </div>`;
  }
  function refreshLyrics() {
    var c = $('[data-now="lyrics"]'); if (c) swap(c, lyricsBlock('page'));
    swap($('[data-nowcol="lyrics"]'), lyricsBlock('col'));
    if (App.store.get('ctxTab') === 'lyrics') App.refreshPanel();
    lyMountAll();
  }

  /* synced lyrics engine: one tick for every instance on the page */
  function lyInit(sc) {
    var key = sc.getAttribute('data-key');
    var st = sc._ly = { key: key, lines: $$('.now-ly__line', sc), cur: -2, auto: false, bk: '' };
    var back = sc.parentNode.querySelector('.now-ly__back');
    function away() { if (st.auto) return; lyAway[key] = true; if (back) back.hidden = false; }
    sc.addEventListener('wheel', away, { passive: true });
    sc.addEventListener('touchmove', away, { passive: true });
    sc.addEventListener('pointerdown', function (e) { if (e.target === sc) away(); });   // scrollbar drag
    sc.addEventListener('keydown', function (e) {
      var line = e.target.closest('.now-ly__line');
      if (line && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End')) {
        e.preventDefault();                                          // the app-wide arrows (volume / seek) stay out
        var i = st.lines.indexOf(line), n = e.key === 'Home' ? 0 : e.key === 'End' ? st.lines.length - 1 : i + (e.key === 'ArrowDown' ? 1 : -1);
        var nx = st.lines[App.clamp(n, 0, st.lines.length - 1)];
        st.lines.forEach(function (l) { l.tabIndex = -1; }); nx.tabIndex = 0; nx.focus(); away();
      } else if (['PageDown', 'PageUp'].indexOf(e.key) >= 0) away();
    });
    sc.addEventListener('scroll', function () { if (st.auto) { clearTimeout(st.autoT); st.autoT = setTimeout(function () { st.auto = false; }, 160); } });
    if (!st.lines.some(function (l) { return l.tabIndex === 0; }) && st.lines[0]) st.lines[0].tabIndex = 0;
    if (lyAway[key] && back) back.hidden = false;
    lyUpdate(sc, true);
    if (lyAway[key]) sc.scrollTop = 0;
  }
  function lyScroll(sc, line, smooth) {
    var st = sc._ly; st.auto = true;
    var top = line.offsetTop - sc.clientHeight * .35 + line.offsetHeight / 2;
    try { sc.scrollTo({ top: Math.max(0, top), behavior: smooth && !App.reducedMotion() ? 'smooth' : 'auto' }); } catch (e) { sc.scrollTop = Math.max(0, top); }
    clearTimeout(st.autoT); st.autoT = setTimeout(function () { st.auto = false; }, smooth ? 700 : 60);
  }
  /* a line can be jumped to only inside what has arrived (the engine's rule, P.seekable) */
  function lineSeekable(line) { return P.seekable(+line.getAttribute('data-t') / (P.duration() || 1)); }
  function lyUpdate(sc, jump) {
    var st = sc._ly, lines = st.lines; if (!lines.length) return;
    var p = ps.pos + .15, i = -1;
    lines.forEach(function (l, n) { if (+l.getAttribute('data-t') <= p) i = n; });
    if (i !== st.cur) {
      lines.forEach(function (l, n) {
        l.classList.toggle('is-past', n < i); l.classList.toggle('is-current', n === i);
        if (n === i) l.setAttribute('aria-current', 'true'); else l.removeAttribute('aria-current');
      });
      st.cur = i;
      if (!lyAway[st.key] && (i >= 0 || jump)) lyScroll(sc, lines[Math.max(i, 0)], !jump);
    }
    // lines outside the downloaded part cannot be jumped to: say so on hover / focus
    var bk = ps.buffered.map(function (r) { return r.join('-'); }).join('|') + '/' + ps.received;
    if (bk !== st.bk) {
      st.bk = bk;
      lines.forEach(function (l) {
        var ok = lineSeekable(l);
        l.setAttribute('data-tip', ok ? 'Jump to ' + fmtTime(+l.getAttribute('data-t')) : 'Not downloaded yet (' + Math.round(ps.received * 100) + '%)');
        l.classList.toggle('is-far', !ok);
      });
    }
  }
  function lyMountAll() { $$('.now-ly__scroll[data-sync]').forEach(function (sc) { if (!sc._ly) lyInit(sc); }); }
  function lyTick() { $$('.now-ly__scroll[data-sync]').forEach(function (sc) { if (!sc._ly) lyInit(sc); else lyUpdate(sc, false); }); }
  document.addEventListener('click', function (e) {
    var line = e.target.closest('.now-ly__line'); if (!line) return;
    var sc = line.closest('.now-ly__scroll');
    if (lineSeekable(line)) {
      P.seek(+line.getAttribute('data-t')); if (!ps.playing) P.play();
      if (sc && sc._ly) { lyAway[sc._ly.key] = false; var b = sc.parentNode.querySelector('.now-ly__back'); if (b) b.hidden = true; sc._ly.cur = -2; lyUpdate(sc, false); }
    } else {
      line.classList.remove('is-denied'); void line.offsetWidth; line.classList.add('is-denied');
      App.announce('Not downloaded yet, ' + Math.round(ps.received * 100) + '% received');
    }
  });

  /* =====================================================================
     5. Queue: now playing, next up, then, radio tail (+ keeps-adding toggle and steer slider)
     ===================================================================== */
  var GROUPS = [['next', 'next up'], ['context', 'then'], ['radio', 'radio keeps going']];
  function qMark(q, i) {
    if (i === ps.index) return '';
    var st = q.resolve;
    if (st === 'none') return html`<span class="resolve now-q__mark is-none">${icon('no-source', null, 'ic-16')}no source</span>`;
    if (st === 'resolving') return html`<span class="resolve now-q__mark"><span class="spin-ring" aria-hidden="true"></span>finding</span>`;
    if (i === ps.index + 1 && (App.isSaved(q.trackId) || P.isPreloaded(q.trackId))) return html`<span class="resolve now-q__mark is-ready">${icon('check', null, 'ic-16')}ready</span>`;
    return '';
  }
  function qRow(q, i, opts) {
    opts = opts || {};
    var t = M.track(q.trackId); if (!t) return '';
    var playing = i === ps.index, saved = App.isSaved(t.id);
    var where = playing ? 'now playing' : 'position ' + (i - ps.index) + ' in the queue';
    var markLabel = q.resolve === 'none' ? ', no source found' : q.resolve === 'resolving' ? ', finding a source' : i === ps.index + 1 ? ', ready' : '';
    return html`<li class="trow trow--dense now-q__row${playing ? ' is-playing' : ''}${playing && ps.playing ? ' is-playing-now' : ''}${q.resolve === 'none' && !playing ? ' is-unavailable' : ''}"
        data-track-id="${t.id}" data-qi="${i}" data-uid="${q.uid}" tabindex="0" ${opts.preview || playing ? '' : raw('data-sortable')} data-fk="row-${q.uid}"
        aria-label="${t.title} by ${t.artist}, ${where}${markLabel}">
      <div class="trow__art">${ui.cover(t, 40)}${playing ? html`<span class="eq now-q__eq" aria-hidden="true"><i></i><i></i><i></i></span>` : ''}</div>
      <div class="trow__main">
        <button class="trow__title" data-act="now-q-jump" data-qi="${i}" tabindex="-1">${t.title}</button>
        <div class="trow__artist"><span class="trow__mini" data-src-mini="${t.id}">${icon(saved ? 'saved' : 'soulseek', saved ? 'Saved' : 'Soulseek')}</span><span>${t.artist}</span>${qMark(q, i)}</div>
      </div>
      <div class="trow__more"><button class="btn-icon btn-icon--bare" data-act="now-q-menu" data-qi="${i}" aria-haspopup="menu" aria-label="Queue actions for ${t.title}" data-tip="More" data-fk="more-${q.uid}">${icon('more')}</button></div>
    </li>`;
  }
  function radioHead() {
    if (!ps.radio) return '';
    var steer = ps.radio.steer == null ? .4 : ps.radio.steer, steerId = App.uid('now-steer');
    return html`<div class="now-q__radio">
      <div class="now-q__radio-row">${icon('radio')}<span class="now-q__radio-name">Radio: <b>${ps.radio.name}</b></span>
        <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-radio-stop" aria-label="Stop radio" data-tip="Stop radio" data-fk="radio-stop">${icon('close', null, 'ic-16')}</button></div>
      <label class="switch-row now-q__keep"><span class="switch-row__label">Keeps adding songs</span>
        <button class="switch" role="switch" aria-checked="${ps.radio.keepsAdding ? 'true' : 'false'}" data-act="now-radio-keep" aria-label="Radio keeps adding songs" data-fk="radio-keep"></button></label>
      <div class="now-q__steer">
        <label class="now-q__steer-label" for="${steerId}">Steer</label>
        <input class="range" id="${steerId}" type="range" min="0" max="100" step="5" value="${Math.round(steer * 100)}" style="--fill:${Math.round(steer * 100)}%" data-now-steer aria-valuetext="${steerText(steer)}" data-fk="radio-steer">
        <div class="now-q__steer-ends" aria-hidden="true"><span>Familiar</span><span>Adventurous</span></div>
      </div>
      ${ps.radio.error ? html`<div class="now-q__err" role="alert">${icon('warning', null, 'ic-16')}<span>Radio paused: ${ps.radio.error}</span><button class="btn btn--sm" data-act="now-radio-retry" data-fk="radio-retry">Try again</button></div>` : ''}
    </div>`;
  }
  function steerText(v) { return v < .25 ? 'Mostly familiar' : v < .45 ? 'A little adventurous' : v < .7 ? 'Balanced' : 'Very adventurous'; }
  function queueBlock() {
    if (!ps.queue.length) {
      return html`<div class="now-q">${ui.emptyState({ art: 'moon', title: 'Nothing queued', text: 'Search or pick something from Home. Songs you add show up here.', action: { label: 'Go home', href: '#/home' }, secondary: { label: 'Search', act: 'now-go-search' } })}</div>`;
    }
    var rows = [html`<li class="now-q__group label-rule label-rule--sm"><h3>now playing</h3></li>`, qRow(ps.queue[ps.index], ps.index)];
    GROUPS.forEach(function (g) {
      var items = [];
      ps.queue.forEach(function (q, i) { if (i > ps.index && q.group === g[0] && M.track(q.trackId)) items.push(qRow(q, i)); });
      if (!items.length) return;
      var label = g[0] === 'radio' && !(ps.radio && ps.radio.keepsAdding) ? 'from radio'
        : g[0] === 'context' && ps.context && ps.context.kind !== 'radio' ? 'next from ' + App.placeName(ps.context) : g[1];
      rows.push(html`<li class="now-q__group label-rule label-rule--sm"><h3>${label}</h3><span class="now-q__count">${items.length}</span></li>`);
      rows = rows.concat(items);
    });
    var after = ps.queue.length - ps.index - 1, from = P.playingFrom();
    return html`<div class="now-q">
      ${from && App.placePath(from) ? ui.playingFrom('now-q__from') : ''}
      ${radioHead()}
      <ol class="tlist tlist--dense now-q__list" aria-label="Queue" data-now-qlist>${rows}</ol>
      ${after ? '' : html`<p class="now-next__none">${icon('sparkle', null, 'ic-16')}Nothing after this song.</p>`}
      <p class="now-q__hint">Drag to reorder, or focus a song and press ${ui.keycap('Alt')} + arrows. ${ui.keycap('Del')} removes.</p>
      <div class="now-q__foot">
        <button class="btn btn--sm" data-act="now-q-playlist" data-fk="q-pl">${icon('playlists')}Save as playlist</button>
        <button class="btn btn--sm" data-act="now-q-library" data-fk="q-lib">${icon('save')}Save to library</button>
        <button class="btn btn--sm btn--quiet" data-act="now-q-clear" ${after ? '' : raw('disabled')} data-fk="q-clear">${icon('trash')}Clear</button>
      </div>
    </div>`;
  }
  function moveRow(qi, dir) {
    var to = qi + dir;
    if (qi === ps.index) { App.toast('That song is playing. Move the others around it.', { icon: 'info' }); return; }
    if (to <= ps.index || to >= ps.queue.length) { App.announce(dir < 0 ? 'Already first after the current song' : 'Already last'); return; }
    var q = ps.queue[qi], t = M.track(q.trackId);
    withFocus('row-' + q.uid, function () { P.move(qi, to); });
    App.announce('Moved ' + (t ? t.title : 'the song') + ' to position ' + (to - ps.index));
  }
  function jumpTo(qi) {
    if (qi === ps.index) { P.toggle(); return; }
    var sh = ps.shuffle;                     // play that song now, keeping the rest of the queue
    P.move(qi, ps.index + 1); ps.shuffle = false; P.next(); ps.shuffle = sh;
  }
  App.on('row:move', function (d) { if (d.row.closest('.now-q')) moveRow(+d.row.getAttribute('data-qi'), d.dir); });
  App.on('row:delete', function (d) {
    if (!d.row.closest('.now-q')) return;
    var qi = +d.row.getAttribute('data-qi'), next = ps.queue[qi + 1] || ps.queue[qi - 1];
    withFocus(next ? 'row-' + next.uid : null, function () { P.removeAt(qi); });
  });
  /* keyboard Enter on a queue row jumps to it (instead of replacing the queue) */
  document.addEventListener('keydown', function (e) {
    var row = e.target.closest && e.target.closest('.now-q__row');
    if (row && e.target === row && e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); jumpTo(+row.getAttribute('data-qi')); }
  }, true);
  /* drag to reorder inside the queue, with pointer events rather than native drag and drop: the row follows the
     pointer and the rows it passes slide out of its way, all by transforms, so nothing is laid out again until the
     drop. Mouse: press anywhere on a row (not its buttons) and move 5 px. Touch: the cover is the handle (it takes no
     scrolling), so the list still scrolls under a finger. Positions are measured once, when the drag starts. */
  var qd = null, qDropped = 0;
  /* a drag is not a click: the click that ends one doesn't jump to the song */
  document.addEventListener('click', function (e) { if (Date.now() - qDropped < 300) { e.stopPropagation(); e.preventDefault(); } }, true);
  function qRows(list) { return $$('.now-q__row', list); }
  document.addEventListener('pointerdown', function (e) {
    if (e.button !== 0 || qd) return;
    var row = e.target.closest && e.target.closest('.now-q__row[data-sortable]');
    if (!row || e.target.closest('.trow__more')) return;
    if (e.pointerType === 'touch' && !e.target.closest('.trow__art')) return;
    qd = { row: row, list: row.closest('[data-now-qlist]'), id: e.pointerId, y0: e.clientY, live: false, frame: 0, y: e.clientY };
    if (!qd.list) qd = null;
  });
  /* the first nearly opaque background behind the list, so the lifted row hides the rows it passes over */
  function solidBehind(el) {
    for (; el && el !== document.documentElement; el = el.parentElement) {
      var c = getComputedStyle(el).backgroundColor, m = /rgba?\(([^)]+)\)|\/\s*([\d.]+)\s*\)$/.exec(c) || [];
      var alpha = m[2] != null ? +m[2] : m[1] ? (m[1].split(',')[3] != null ? +m[1].split(',')[3] : 1) : 1;
      if (c !== 'transparent' && alpha >= .95) return c;
    }
    return '';
  }
  function qStart() {
    var rows = qRows(qd.list), r0 = qd.row.getBoundingClientRect();
    qd.live = true; qd.from = rows.indexOf(qd.row); qd.to = qd.from;
    qd.mids = rows.map(function (r) { var b = r.getBoundingClientRect(); return b.top + b.height / 2; });
    qd.rows = rows; qd.step = r0.height + 2;   // .now-q__list's 2 px gap
    qd.scroller = qd.list.closest('.ctx__body, #ctx-body, .main') || document.scrollingElement; qd.s0 = qd.scroller.scrollTop;
    try { qd.row.setPointerCapture(qd.id); } catch (err) { /* already released */ }
    var bg = solidBehind(qd.list); if (bg) qd.row.style.setProperty('--lift-bg', bg);
    qd.list.classList.add('is-sorting'); qd.row.classList.add('is-lifted');
  }
  function qPaint() {
    qd.frame = 0;
    if (!qd.row.isConnected) { qEnd(false); return; }   // the queue re-rendered under the drag
    var box = qd.scroller.getBoundingClientRect ? qd.scroller.getBoundingClientRect() : { top: 0, bottom: innerHeight };
    /* near an edge of the scrolling list, it scrolls: faster the closer the pointer gets */
    var edge = Math.min(qd.y - box.top, box.bottom - qd.y), speed = edge < 32 ? Math.ceil((32 - Math.max(edge, 0)) / 3) : 0;
    if (speed) qd.scroller.scrollTop += qd.y - box.top < box.bottom - qd.y ? -speed : speed;
    var scrolled = qd.scroller.scrollTop - qd.s0, dy = qd.y - qd.y0 + scrolled, mid = qd.mids[qd.from] + dy, to = qd.from;
    while (to < qd.mids.length - 1 && mid > qd.mids[to + 1]) to++;
    while (to > 0 && mid < qd.mids[to - 1]) to--;
    qd.to = to;
    qd.rows.forEach(function (r, k) {
      var shift = k === qd.from ? dy : k > qd.from && k <= to ? -qd.step : k < qd.from && k >= to ? qd.step : 0;
      r.style.transform = shift ? 'translateY(' + shift + 'px)' : '';
    });
    if (speed) qd.frame = requestAnimationFrame(qPaint);
  }
  document.addEventListener('pointermove', function (e) {
    if (!qd || e.pointerId !== qd.id) return;
    qd.y = e.clientY;
    if (!qd.live) { if (Math.abs(qd.y - qd.y0) < 5) return; qStart(); }
    e.preventDefault();
    if (!qd.frame) qd.frame = requestAnimationFrame(qPaint);
  });
  function qEnd(drop) {
    var d = qd; qd = null; if (!d) return;
    if (d.frame) cancelAnimationFrame(d.frame);
    if (!d.live) return;
    d.list.classList.remove('is-sorting'); d.row.classList.remove('is-lifted');
    (d.rows || []).forEach(function (r) { r.style.transform = ''; }); d.row.style.removeProperty('--lift-bg');
    qDropped = Date.now();
    if (!drop || d.to === d.from || !d.row.isConnected) return;
    var from = +d.row.getAttribute('data-qi'), to = +d.rows[d.to].getAttribute('data-qi');
    if (to <= ps.index) to = ps.index + 1;
    if (to !== from) { var t = M.track(ps.queue[from].trackId); P.move(from, to); App.announce('Moved ' + (t ? t.title : 'the song')); }
  }
  document.addEventListener('pointerup', function (e) { if (qd && e.pointerId === qd.id) qEnd(true); });
  document.addEventListener('pointercancel', function (e) { if (qd && e.pointerId === qd.id) qEnd(false); });
  document.addEventListener('keydown', function (e) { if (qd && qd.live && e.key === 'Escape') { e.preventDefault(); qEnd(false); } }, true);

  /* =====================================================================
     6. Activity: transfers (hatched progress), saves (5 steps), imports, rescans; filters; All quiet
     ===================================================================== */
  var SAVE_STATUS = { running: 'saving', done: 'saved', failed: 'failed', pending: 'pending', waiting: 'waiting' };   // save job stepState -> record status
  /* Rows: transfers, imports and rescans from store 'activity'; saves from store 'saves' (the live record per song),
     plus the save jobs the server listed for songs this page holds no record for. */
  function jobs() {
    var saves = App.store.get('saves') || {};
    var list = (App.store.get('activity') || []).filter(function (j) { return !(j.kind === 'save' && saves[j.trackId]); })
      .map(function (j) { return j.kind === 'save' ? Object.assign({}, j, { status: SAVE_STATUS[j.stepState] || 'saving' }) : j; });
    Object.keys(saves).forEach(function (id) {
      var s = saves[id];
      list.unshift({ id: s.jobId || 'save-' + id, jobId: s.jobId, kind: 'save', trackId: id, step: s.step, status: s.status, error: s.error, upgrade: s.upgrade, belowMinimum: s.belowMinimum });
    });
    return list;
  }
  function jobState(j) {
    if (j.kind === 'transfer') return j.state === 'failed' ? 'failed' : j.state === 'cancelled' ? 'cancelled' : ['transferring', 'queued', 'stalled'].indexOf(j.state) >= 0 ? 'active' : 'done';
    if (j.kind === 'save') return j.status === 'saving' || j.status === 'waiting' ? 'active' : j.status === 'failed' ? 'failed' : 'done';
    return j.state === 'running' ? 'active' : j.state === 'failed' ? 'failed' : 'done';
  }
  function jobTitle(j) {
    var t = j.trackId && M.track(j.trackId);
    return t ? t.title : j.kind === 'import' ? j.name || 'Playlist import' : j.kind === 'rescan' ? 'Jellyfin rescan' : 'Unknown song';
  }
  function actBtn(ic, label, act, j, extra) {
    return html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="${act}" data-job="${j.id}" ${extra ? raw(extra) : ''} aria-label="${label}" data-tip="${label}" data-fk="${act}-${j.id}">${icon(ic, null, 'ic-16')}</button>`;
  }
  function transferParts(j) {
    var pct = Math.round(j.pct || 0), cancel = actBtn('close', 'Cancel transfer', 'now-job-cancel', j);
    switch (j.state) {
      case 'transferring': return { meta: html`${icon('soulseek', null, 'ic-16')}<span>${j.peer}, ${mbps(j.speed)}, ${pct}%</span>`, extra: html`<div class="progress progress--xfer" style="--p:${pct / 100}"><i></i></div>`, acts: cancel };
      case 'queued': return { meta: html`${icon('clock', null, 'ic-16')}<span>Queued at ${j.peer}${j.queuePos ? ', position ' + j.queuePos : ''}</span>`, extra: html`<div class="progress" style="--p:0"><i></i></div>`, acts: cancel };
      case 'stalled': return { meta: html`${icon('refresh', null, 'ic-16')}<span>${j.peer} stalled at ${pct}%</span>`, extra: html`<div class="progress" style="--p:${pct / 100}"><i></i></div>`, acts: cancel };
      case 'failed': return { meta: html`${ui.statusGlyph('down', 'Failed')}<span>${j.error || 'Transfer failed'}</span>`, extra: html`<div class="progress progress--failed" style="--p:${pct / 100}"><i></i></div>`,
        acts: html`${actBtn('refresh', 'Retry transfer', 'now-job-retry', j)}${j.trackId ? actBtn('soulseek', 'Pick another source', 'show-sources', j, 'data-id="' + App.esc(j.trackId) + '"') : ''}` };
      case 'cancelled': return { meta: html`${icon('close', null, 'ic-16')}<span>Cancelled at ${pct}%</span>`, extra: '', acts: actBtn('refresh', 'Start again', 'now-job-retry', j) };
      default: return { meta: html`${ui.statusGlyph('ok', 'Done')}<span>${j.peer ? 'Transferred from ' + j.peer : 'Transferred'}</span>`, extra: '', acts: '' };
    }
  }
  function saveParts(j, st) {
    var acts;
    if (st === 'failed') acts = html`${App.discover ? actBtn('info', 'What went wrong', 'now-job-fix', j) : ''}${actBtn('refresh', 'Retry save', 'now-job-retry', j)}${actBtn('close', 'Remove this save', 'now-job-cancel', j)}`;
    else if (j.status === 'waiting') acts = actBtn('close', 'Stop waiting', 'now-job-cancel', j);
    else if (st === 'done') acts = actBtn('library', 'Show album', 'go-album', j, 'data-id="' + App.esc(j.trackId) + '"');
    return { meta: j.belowMinimum ? ui.belowBadge() : '', extra: ui.pipeline(j.step || 1, j.status, j.error), acts: acts || '' };
  }
  function otherParts(j, st) {
    if (st === 'failed') return { meta: html`${ui.statusGlyph('down', 'Failed')}<span>${j.error || 'Failed'}</span>`, extra: '', acts: '' };
    if (st === 'active') return { meta: html`<span class="spin-ring" aria-hidden="true"></span><span>${j.detail || (j.pct != null ? 'Importing, ' + Math.round(j.pct * (j.pct <= 1 ? 100 : 1)) + '%' : 'Running')}</span>`, extra: '', acts: '' };
    if (j.kind === 'import' && 'added' in j) {        // listening history (POST /api/import/history)
      return { meta: html`${ui.statusGlyph('ok', 'Done')}<span>${j.added || 0} plays added${j.duplicates ? ', ' + j.duplicates + ' already there' : ''}${j.invalid ? ', ' + j.invalid + ' unreadable' : ''}</span>`, extra: '', acts: '' };
    }
    if (j.kind === 'import' && j.link) {        // a playlist link (POST /api/import/link): its review lists the songs not found
      return { meta: html`${ui.statusGlyph('ok', 'Done')}<span>In your playlists: ${j.matched || 0} matched${j.unmatched ? ', ' + j.unmatched + ' not found' : ''}${j.saving ? ', saving ' + j.saving : ''}</span>`, extra: '',
        acts: html`${j.playlistId ? html`<a class="btn btn--sm btn--quiet" href="#/playlist/${j.playlistId}" data-fk="open-${j.id}">Open</a>` : ''}<button class="btn btn--sm btn--quiet" data-act="lib-link-review" data-id="${j.link}" data-fk="review-${j.id}">Review</button>` };
    }
    if (j.kind === 'import') {
      return { meta: html`${ui.statusGlyph('ok', 'Done')}<span>${j.matched || 0} matched, ${j.search || 0} searching, ${j.notFound || 0} not found</span>`, extra: '',
        acts: html`<button class="btn btn--sm btn--quiet" data-act="now-job-review" data-fk="review-${j.id}">Review</button>` };
    }
    return { meta: html`${ui.statusGlyph('ok', 'Done')}<span>${j.detail || 'Done'}</span>`, extra: '', acts: '' };
  }
  function jobRow(j) {
    var t = j.trackId ? M.track(j.trackId) : null, st = jobState(j);
    var parts = j.kind === 'transfer' ? transferParts(j) : j.kind === 'save' ? saveParts(j, st) : otherParts(j, st);
    var sub = t ? (j.upgrade ? 'Upgrade, ' : '') + t.artist : j.kind === 'import' ? ('added' in j ? 'Listening history' : j.link ? 'Playlist link' : 'Playlist import') : j.kind === 'rescan' ? 'Library' : '';
    var art = t ? ui.cover(t, 40) : html`<span class="now-act__glyph" aria-hidden="true">${icon(j.kind === 'import' ? 'import' : j.kind === 'rescan' ? 'refresh' : 'soulseek')}</span>`;
    return html`<li class="now-act__row is-${st}">
      <div class="now-act__art">${art}</div>
      <div class="now-act__main">
        <div class="now-act__title"><b>${jobTitle(j)}</b><span>${sub}</span></div>
        ${parts.meta ? html`<div class="now-act__meta">${parts.meta}</div>` : ''}
        ${parts.extra}
      </div>
      <div class="now-act__acts">${parts.acts}</div>
    </li>`;
  }
  function activityBlock() {
    var err = App.store.get('activityError');
    if (err) return html`<div class="now-act">${ui.emptyState({ art: 'star', title: 'Couldn\'t load activity', text: err, action: { label: 'Try again', act: 'now-act-reload' } })}</div>`;
    var all = jobs(), counts = { all: all.length, active: 0, done: 0, failed: 0, cancelled: 0 };
    all.forEach(function (j) { counts[jobState(j)]++; });
    if (!all.length) return html`<div class="now-act">${ui.emptyState({ art: 'moon', title: 'All quiet', text: 'Transfers, saves and imports show up here while they run.' })}</div>`;
    var shown = all.filter(function (j) { return actFilter === 'all' || jobState(j) === actFilter; });
    var order = { active: 0, failed: 1, done: 2, cancelled: 3 };
    var filters = ['all', 'active', 'done', 'failed'].concat(counts.cancelled || actFilter === 'cancelled' ? ['cancelled'] : []);   // Cancelled shows only when something was
    shown.sort(function (a, b) { return order[jobState(a)] - order[jobState(b)]; });
    var groups = [['transfer', 'transfers'], ['save', 'saves'], ['other', 'imports and rescans']].map(function (g) {
      var items = shown.filter(function (j) { return g[0] === 'other' ? (j.kind === 'import' || j.kind === 'rescan') : j.kind === g[0]; });
      return items.length ? html`${ui.labelRule(g[1], { level: 'h3', sm: true })}<ul class="now-act__list">${items.map(jobRow)}</ul>` : '';
    });
    return html`<div class="now-act">
      <div class="now-act__filters" role="group" aria-label="Filter activity">${filters.map(function (f) {
        return html`<button class="pill-filter" data-act="now-act-filter" data-value="${f}" aria-pressed="${String(actFilter === f)}" data-fk="f-${f}">${f[0].toUpperCase() + f.slice(1)} <span class="now-act__n">${counts[f]}</span></button>`;
      })}</div>
      ${shown.length ? groups : html`<p class="now-next__none">${icon('check', null, 'ic-16')}Nothing ${actFilter} right now.<button class="btn btn--sm btn--quiet" data-act="now-act-filter" data-value="all">Show all</button></p>`}
      <div class="now-q__foot"><button class="btn btn--sm" data-act="now-act-clear" ${counts.done || counts.cancelled ? '' : raw('disabled')} data-fk="act-clear">${icon('check')}Clear finished</button></div>
    </div>`;
  }

  /* =====================================================================
     7. Route: /now (desktop two columns, tablet, phone sheet with Art / Lyrics / Queue / Details)
     ===================================================================== */
  function tabList() { return TABS.filter(function (tb) { return tb !== 'lyrics' || lyricsAvailable(); }); }
  function phoneTabs() {
    var list = tabList(), i = list.indexOf(view.tab);
    return html`<div class="seg now-seg" role="tablist" aria-label="Now Playing views" style="--n:${list.length};--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${list.map(function (tb, n) {
      return html`<button class="seg__btn" role="tab" id="now-tab-${tb}" aria-selected="${String(n === i)}" tabindex="${n === i ? 0 : -1}" aria-controls="now-panel" data-act="now-tab" data-tab="${tb}">${tb[0].toUpperCase() + tb.slice(1)}</button>`;
    })}</div>`;
  }
  function render(ctx) {
    var q = ctx.query;
    if (q.tab && TABS.indexOf(q.tab) >= 0) view.tab = q.tab;
    if (tabList().indexOf(view.tab) < 0) view.tab = 'art';
    view.detailsTrack = q.track || null;
    if (q.tab === 'details') view.detailsOpen = true;
    return html`<div class="now" data-tab="${view.tab}" data-page="${view.page}">
      <div class="now__top"><p class="now-head__from">${fromLine()}</p>${ui.iconButton('more', 'More player actions', { 'data-act': 'player-more', 'aria-haspopup': 'menu', cls: 'btn-icon--bare' })}</div>
      <div class="now__grid" id="now-panel" role="region" aria-label="Now Playing">
        <header class="now-head" data-sec="art" data-now="head-info">${infoHead()}</header>
        <div class="now__acts" data-sec="art" data-now="acts">${actionRow()}</div>
        <div class="now__dev">${hero('full')}
          <p class="now__keys"><span class="now__kg">${ui.keycap('P')}${ui.keycap('N')} skip</span><span class="now__kg">${ui.keycap('Up')}${ui.keycap('Down')} volume</span><span class="now__kg">${ui.keycap('Space')} play</span></p>
        </div>
        <div class="now__devc">${hero('compact')}</div>
        <div class="now__seg">${phoneTabs()}</div>
        <section class="card now-srccard" data-sec="details" data-now="src" aria-label="Source">${sourceCard()}</section>
        <section class="card now-lycard" data-sec="lyrics" data-now="lyrics" aria-label="Lyrics">${lyricsBlock('page')}</section>
        <section class="card now-next" data-sec="art" data-now="next" aria-label="Up next" data-meta="${queueMeta()}">${upNext()}</section>
        <section class="card now-qcard" data-sec="queue" data-now="queue" aria-label="Queue" data-meta="${queueMeta()}">${queueBlock()}</section>
      </div>
      ${ui.sparkle('--s:18px; right:2%; top:4px')}
    </div>`;
  }
  var lastRender = 0;
  /* "power on": the device lifts in and the screen wakes (reduced motion: a short fade) */
  function powerOn(root) {
    var devs = $$('.now-dev, .now-devc', root);
    if (App.reducedMotion()) {
      devs.forEach(function (d) { d.classList.add('now-fade'); });
      requestAnimationFrame(function () { requestAnimationFrame(function () { devs.forEach(function (d) { d.classList.remove('now-fade'); }); }); });
    } else {
      devs.forEach(function (d) { d.classList.add('is-power-on'); });
      setTimeout(function () { devs.forEach(function (d) { d.classList.remove('is-power-on'); }); }, 1100);
    }
  }
  function mount(ctx) {
    var root = $('.now', ctx.el); if (!root) return;
    updateTextHooks();
    if (!ctx.refresh) powerOn(root);             // once per visit, not on every refresh of the page
    lastRender = Date.now();
    lyMountAll();
    App.syncRows();
    if (!ctx.refresh && ctx.query.tab === 'queue' && window.innerWidth >= 640) App.openPanel('queue');
    if (!ctx.refresh && (ctx.query.tab === 'details' || ctx.query.tab === 'lyrics')) {
      var target = $(ctx.query.tab === 'details' ? '.now-srccard' : '.now-lycard', root);
      if (target && window.innerWidth >= 640) setTimeout(function () { target.scrollIntoView({ block: 'start', behavior: App.reducedMotion() ? 'auto' : 'smooth' }); }, 650);
    }
    var offs = [
      App.on('player', onPlayer),
      App.on('save', function (d) { if (d.id === P.currentId()) { updateSaveBits(); } }),
      App.on('like', function () { var t = cur(); $$('[data-now="b-label"]').forEach(function (el) { App.setText(el, t && App.isLiked(t.id) ? 'liked' : 'like'); }); }),
      App.on('follow', function () { var a = $('[data-now="acts"]'); if (a) swap(a, actionRow()); })
    ];
    ctx.onCleanup(function () { offs.forEach(function (off) { off(); }); });
  }
  function updateSaveBits() {
    var t = cur(); if (!t) return;
    var pipe = $('[data-now="pipeline"]'); if (pipe) swap(pipe, saveState(t));
    if (view.page === 'save') $$('[data-now="page"]').forEach(function (p) { swap(p, pageBody(t)); });
  }
  function updateControls() {
    var on = playingNow();
    $$('[data-now="a-label"]').forEach(function (el) { App.setText(el, on ? 'pause' : 'play'); });
    $$('[data-now="led"]').forEach(function (el) { App.setText(el, ledText()); });
    $$('[data-now="head"]').forEach(function (el) { App.setText(el, headText()); });
    $$('[data-now="state"]').forEach(function (el) { App.setText(el, stateText()); });
    $$('.now .device__pill--shuffle').forEach(function (b) { b.setAttribute('aria-pressed', String(ps.shuffle)); });
    $$('.now .device__pill--repeat').forEach(function (b) {
      b.setAttribute('aria-pressed', String(ps.repeat !== 'off'));
      b.setAttribute('aria-label', { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat] + ' (R)');
    });
    $$('[data-now="repeat"]').forEach(function (el) { App.setText(el, repeatLabel()); });
  }
  /* text-only live hooks for custom heroes: data-now="title|artist|album|format" (format = App.fmt.formatLine) */
  function updateTextHooks() {
    var t = cur();
    var text = { title: t ? t.title : 'Nothing playing', artist: t ? t.artist : '', album: t ? t.album : '', format: App.fmt.formatLine(t) };
    Object.keys(text).forEach(function (k) { $$('[data-now="' + k + '"]').forEach(function (el) { App.setText(el, text[k]); }); });
  }
  function updateMeta() {
    $$('[data-now="next"], [data-now="queue"]').forEach(function (c) { c.setAttribute('data-meta', queueMeta()); });
    App.setMeta(App.fmt.formatLine(cur()));
  }
  function onPlayer(kind) {
    if (!onRoute()) return;
    var t = cur();
    if (kind === 'track' || kind === 'queue') updateMeta();
    if (kind === 'track') updateTextHooks();
    if (kind === 'track') {
      $$('[data-now="screen"]').forEach(function (h) { slideSwap(h, function () { return slide(cur()); }); });
      $$('[data-now="c-screen"]').forEach(function (h) { slideSwap(h, function () { return compactSlide(cur()); }); });
      $$('[data-now="c-art"]').forEach(function (h) { swap(h, t ? ui.cover(t, 56) : '<span class="now-dev__slot" aria-hidden="true"></span>'); });
      $$('[data-now="b"]').forEach(function (h) { swap(h, likeBtn(52)); });
      $$('[data-now="cb"]').forEach(function (h) { swap(h, likeBtn(40)); });
      $$('[data-now="b-label"]').forEach(function (el) { App.setText(el, t && App.isLiked(t.id) ? 'liked' : 'like'); });
      lyAway = {};
      $$('[data-now="facts"]').forEach(function (el) { swap(el, factsLine()); });
      swap($('[data-now="head-info"]'), infoHead());
      swap($('[data-now="acts"]'), actionRow());
      swap($('[data-now="src"]'), sourceCard());
      swap($('[data-now="lyrics"]'), lyricsBlock('page'));
      swap($('[data-now="next"]'), upNext());
      swap($('[data-now="queue"]'), queueBlock());
      $$('.now .now-head__from').forEach(function (el) { swap(el, fromLine()); });
      lyMountAll(); App.syncRows();
    } else if (kind === 'status' || kind === 'source' || kind === 'ended' || kind === 'state' || kind === 'play') {
      $$('[data-now="facts"]').forEach(function (el) { swap(el, factsLine()); });
      if (kind !== 'play') {
        $$('[data-now="page"]').forEach(function (p) { swap(p, pageBody(t)); });
        swap($('[data-now="src"]'), sourceCard());
      } else if (view.page === 'now') {
        $$('[data-now="page"]').forEach(function (p) { swap(p, pageBody(t)); });   // paused / playing label + facts
      }
    } else if (kind === 'queue') {
      swap($('[data-now="next"]'), upNext());
      swap($('[data-now="queue"]'), queueBlock());
      $$('.now .now-head__from').forEach(function (el) { swap(el, fromLine()); });
      App.syncRows();
    } else if (kind === 'volume') {
      if (Date.now() - lastRender > 400) showOsd();
    }
    updateControls();
  }

  /* synced lyrics follow the position on every engine tick */
  App.on('tick', lyTick);

  /* =====================================================================
     8. Actions
     ===================================================================== */
  App.action('now-page', function () {
    view.page = PAGES[(PAGES.indexOf(view.page) + 1) % PAGES.length];
    var t = cur();
    $$('.now').forEach(function (n) { n.setAttribute('data-page', view.page); });
    $$('[data-now="page"]').forEach(function (p) { swap(p, pageBody(t)); });
    $$('.now .device__select').forEach(function (b) { b.innerHTML = view.page + window.icon('chevron-right', null, 'ic-16'); b.setAttribute('aria-label', selectLabel()); });
    updateControls();
    App.announce('Screen: ' + view.page + ' page');
  });
  App.action('now-vol', function (el) { P.setVolume((ps.muted ? 0 : ps.volume) + .05 * +el.getAttribute('data-dir')); showOsd(); });
  App.action('now-nothing', function () { App.toast('Nothing playing yet. Pick something from Home.', { icon: 'info' }); });
  App.action('now-tab', function (el) { setTab(el.getAttribute('data-tab'), true); });
  function setTab(tab, focus) {
    view.tab = tab;
    try { history.replaceState(history.state, '', '#/now?tab=' + tab); } catch (e) { /* file:// in some browsers */ }
    var root = $('.now'); if (!root) return;
    root.setAttribute('data-tab', tab);
    var seg = $('.now-seg', root), i = tabList().indexOf(tab);
    seg.style.setProperty('--i', i);
    $$('.seg__btn', seg).forEach(function (b, n) { b.setAttribute('aria-selected', String(n === i)); b.tabIndex = n === i ? 0 : -1; if (n === i && focus) b.focus(); });
    if (tab === 'details' && !view.detailsOpen) { view.detailsOpen = true; refreshSourceCard(); }
    $$('.now-ly__scroll[data-sync]', root).forEach(function (sc) { if (sc._ly) { sc._ly.cur = -2; lyUpdate(sc, true); } });
  }
  document.addEventListener('keydown', function (e) {        // arrow keys move between the phone tabs
    var b = e.target.closest && e.target.closest('.now-seg .seg__btn'); if (!b) return;
    var list = tabList(), i = list.indexOf(b.getAttribute('data-tab')), n = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: list.length - 1 }[e.key];
    if (n == null) return;
    e.preventDefault(); setTab(list[(n + list.length) % list.length], true);
  });
  App.action('now-details', function () {
    view.detailsOpen = !view.detailsOpen;
    refreshSourceCard();
  });
  App.action('now-other-sources', function (el) {
    var id = el.getAttribute('data-id'), r = sources[id];
    view.detailsTrack = view.detailsTrack ? null : id;
    // reopening a list that failed or was still filling asks again
    if (view.detailsTrack && r && !r.loading && (r.error || !(r.data && r.data.done))) loadSources(id);
    refreshSourceCard();
  });
  App.action('now-src-reload', function (el) { loadSources(el.getAttribute('data-id')); refreshSourceCard(); });
  App.action('now-use-source', function (el) {
    var id = el.getAttribute('data-id'), s = sourceList(id)[+el.getAttribute('data-si')]; if (!s) return;
    P.useSource(id, s);
    App.toast('Now from ' + s.peer + (s.format ? ', ' + s.format : ''), { icon: 'soulseek' });
  });
  App.action('now-open-queue', function () {
    if (window.innerWidth < 640 && onRoute()) { setTab('queue', true); return; }
    App.openPanel('queue');
  });
  App.action('now-go-search', function () { if (window.innerWidth < 1360) App.closePanel(); App.go('/search'); });

  // lyrics
  App.action('now-ly-size', function (el) {
    var s = App.clamp(lyStep() + +el.getAttribute('data-dir'), -2, 2);
    App.storage('lyricStep', s);
    $$('.now-ly').forEach(function (n) { n.style.setProperty('--ly-step', s); });
    $$('[data-act="now-ly-size"]').forEach(function (b) { var d = +b.getAttribute('data-dir'); b.disabled = d < 0 ? s <= -2 : s >= 2; });
    $$('.now-ly__scroll[data-sync]').forEach(function (sc) { if (sc._ly) { sc._ly.cur = -2; lyUpdate(sc, true); } });
    App.announce('Lyrics text size ' + (s === 0 ? 'default' : s > 0 ? 'larger ' + s : 'smaller ' + -s));
  });
  App.action('now-ly-back', function (el) {
    var sc = el.parentNode.querySelector('.now-ly__scroll'); if (!sc || !sc._ly) return;
    lyAway[sc._ly.key] = false; el.hidden = true; sc._ly.cur = -2; lyUpdate(sc, false);
    var c = $('.now-ly__line.is-current', sc); if (c) { $$('.now-ly__line', sc).forEach(function (l) { l.tabIndex = -1; }); c.tabIndex = 0; c.focus({ preventScroll: true }); }
  });
  App.action('now-ly-retry', function () {
    var t = cur(); if (!t) return;
    delete lyrics[t.id]; refreshLyrics();
  });

  // queue
  App.action('now-q-jump', function (el) { jumpTo(+el.getAttribute('data-qi')); });
  App.action('now-q-menu', function (el) {
    var qi = +el.getAttribute('data-qi'), q = ps.queue[qi], t = q && M.track(q.trackId), playing = qi === ps.index;
    if (!t) return;
    var items = [
      { label: playing ? (ps.playing ? 'Pause' : 'Play') : 'Play now', icon: playing && ps.playing ? 'pause' : 'play', run: function () { jumpTo(qi); } }
    ];
    if (!playing) {
      items.push({ label: 'Play next', icon: 'queue', run: function () { withFocus('row-' + q.uid, function () { P.move(qi, ps.index + 1); }); App.announce(t.title + ' plays next'); } });
      items.push({ label: 'Move up', icon: 'chevron-up', run: function () { moveRow(qi, -1); } });
      items.push({ label: 'Move down', icon: 'chevron-down', run: function () { moveRow(qi, 1); } });
      items.push({ label: 'Remove from queue', icon: 'trash', run: function () { P.removeAt(qi); } });
    }
    items.push('sep');
    items.push({ label: App.isSaved(t.id) ? 'In your library' : 'Save to library', icon: App.isSaved(t.id) ? 'saved' : 'save', run: function () { App.saves.start([t.id]); } });
    items.push({ label: 'Add to playlist', icon: 'playlists', run: function () { App.hooks.addToPlaylist([t.id], el); } });
    items.push({ label: 'Go to album', icon: 'library', run: function () { App.go('/album/' + t.albumId); } });
    items.push(App.coverItem('track', t.id));
    items.push({ label: 'Show sources', icon: 'soulseek', run: function () { App.hooks.sourcePicker(t.id); } });
    App.menu(el, items, { label: 'Queue actions for ' + t.title });
  });
  App.action('now-q-clear', function () { P.clear(); });
  App.action('now-q-playlist', function () {
    var ids = ps.queue.map(function (q) { return q.trackId; });
    var name = ps.radio && ps.radio.name ? ps.radio.name.toLowerCase() + ' radio' : 'saved queue';
    App.createPlaylist({ name: name, desc: 'Saved from the queue.' }).then(function (p) {
      if (!p) return;                                         // createPlaylist already said why
      return App.savePlaylistOrder(p.id, ids).then(function () {
        App.toast('Saved ' + ids.length + ' songs as playlist "' + name + '"', { icon: 'playlists', action: 'Open', onAction: function () { App.go('/playlist/' + p.id); } });
      }, function (err) { App.toast('Couldn\'t add the songs to "' + name + '": ' + err.message, { kind: 'error' }); });
    });
  });
  App.action('now-q-library', function () {
    var ids = ps.queue.map(function (q) { return q.trackId; }).filter(function (id, i, a) { return a.indexOf(id) === i && !App.isSaved(id); });
    if (!ids.length) { App.toast('Everything in the queue is already in your library', { icon: 'saved' }); return; }
    App.saves.start(ids);
    if (App.discover && ids.length > 1) App.discover.openAggregate('your queue', ids);
    else App.toast('Saving ' + ids.length + ' songs to your library', { icon: 'save', action: 'Show', onAction: function () { App.openPanel('activity'); } });
  });
  App.action('now-radio-keep', function () {
    var on = !(ps.radio && ps.radio.keepsAdding);
    withFocus('radio-keep', function () { P.setRadio({ keepsAdding: on }); });
    App.announce(on ? 'Radio keeps adding songs' : 'Radio stops after the queue');
  });
  App.action('now-radio-stop', function () {
    var before = ps.radio;
    P.setRadio(null);
    App.toast('Radio stopped. The queue stays.', { icon: 'radio', action: 'Undo', onAction: function () { P.setRadio(before); } });
  });
  App.action('now-radio-retry', function () {
    withFocus('radio-keep', function () { P.setRadio({ error: null }); });
    App.toast('Looking for more radio songs', { icon: 'radio' });
  });
  /* the slider only repaints while it moves; the steer goes to the engine on release (which re-fills the radio) */
  document.addEventListener('input', function (e) {
    var r = e.target.closest && e.target.closest('[data-now-steer]'); if (!r) return;
    r.style.setProperty('--fill', r.value + '%'); r.setAttribute('aria-valuetext', steerText(+r.value / 100));
  });
  document.addEventListener('change', function (e) {
    var r = e.target.closest && e.target.closest('[data-now-steer]'); if (!r || !ps.radio) return;
    var v = +r.value / 100;
    withFocus('radio-steer', function () { P.setRadio({ steer: v }); });
    App.toast('Radio steering: ' + steerText(v).toLowerCase() + '. Upcoming songs will follow.', { icon: 'radio' });
  });

  // activity
  function findJob(id) { return jobs().filter(function (j) { return j.id === id; })[0]; }
  function toastError(err) { App.toast(err.message, { kind: 'error' }); }
  /* a save job the server listed that this page holds no record for: record it, so retry and the recovery sheet
     act on that job (App.saves keys saves by song) */
  function adoptSave(j) {
    if (!App.saves.get(j.trackId)) {
      App.saves.set(j.trackId, { jobId: j.id, step: j.step || 1, status: j.status, pct: j.pct || 0, error: j.error || null,
        kind: j.failKind || null, nocover: !!j.nocover });
    }
  }
  App.action('now-act-filter', function (el) { actFilter = el.getAttribute('data-value'); withFocus('f-' + actFilter, App.refreshPanel); });
  App.action('now-act-reload', function () { App.loadJobs(); });
  /* DELETE /api/jobs clears the server's finished rows (failed saves stay); the finished save records go too,
     since the songs keep their saved state on the track itself */
  App.action('now-act-clear', function () {
    M.api('/api/jobs', { method: 'DELETE' }).then(function () {
      var saves = App.store.get('saves') || {};
      Object.keys(saves).forEach(function (id) { if (saves[id].status === 'saved') App.saves.set(id, null); });
      App.toast('Cleared finished jobs', { icon: 'check' });
      return App.loadJobs();
    }, toastError);
  });
  App.action('now-job-retry', function (el) {
    var j = findJob(el.getAttribute('data-job')); if (!j) return;
    if (j.kind === 'save') {
      adoptSave(j); App.saves.retry(j.trackId);
      App.toast('Retrying save: ' + jobTitle(j), { icon: 'refresh' });
      return;
    }
    M.api('/api/transfers/' + encodeURIComponent(j.id) + '/retry', { method: 'POST' }).then(function () {
      App.toast('Retrying transfer: ' + jobTitle(j), { icon: 'soulseek' });
    }, toastError);
  });
  /* a transfer is cancelled with DELETE /api/transfers/{id}; for a save, DELETE /api/saves/{jobId}: a waiting save
     stops looking, a failed one goes away */
  App.action('now-job-cancel', function (el) {
    var j = findJob(el.getAttribute('data-job')); if (!j) return;
    if (j.kind === 'transfer') {
      M.api('/api/transfers/' + encodeURIComponent(j.id), { method: 'DELETE' }).then(function () {
        App.toast('Transfer cancelled: ' + jobTitle(j), { icon: 'close' });
      }, toastError);
      return;
    }
    adoptSave(j); App.saves.cancel(j.trackId);
  });
  App.action('now-job-fix', function (el) {              // the recovery sheet reads the live save record
    var j = findJob(el.getAttribute('data-job')); if (!j || !App.discover) return;
    adoptSave(j);
    App.discover.openSaveSheet(j.trackId);
  });
  App.action('now-job-review', function () { if (window.innerWidth < 1360) App.closePanel(); App.go('/playlists'); });

  /* =====================================================================
     9. Context panel tabs (Queue / Lyrics / Activity)
     ===================================================================== */
  /* meta line for title bars: "14 songs, 52 min" still to play after the current song */
  function queueMeta() {
    var rest = ps.queue.slice(ps.index + 1).map(function (q) { return M.track(q.trackId); }).filter(Boolean);
    if (!rest.length) return ps.queue.length ? 'nothing up next' : '';
    var sec = rest.reduce(function (n, t) { return n + t.duration; }, 0);
    return rest.length + (rest.length === 1 ? ' song, ' : ' songs, ') + App.fmt.length(sec);
  }
  App.panel('queue', {
    label: 'Queue', meta: queueMeta,
    render: function (body) { rememberFocus(body); return queueBlock(); },
    mount: function (body) { restoreFocus(body); App.syncRows(); }
  });
  App.panel('lyrics', {
    label: 'Lyrics', meta: function () { var t = cur(); return t ? t.title : ''; },
    hidden: function () { return !lyricsAvailable(); }, hiddenNote: 'No lyrics for this song',
    render: function (body) { rememberFocus(body); return lyricsBlock('panel'); },
    mount: function (body) { lyMountAll(); restoreFocus(body); }
  });
  App.panel('activity', {
    label: 'Activity',
    render: function (body) { rememberFocus(body); return activityBlock(); },
    mount: function (body) { restoreFocus(body); }
  });

  /* =====================================================================
     10. The Now Playing column (layout.js, ADR-0007): art, the transfer state and synced lyrics beside any screen
     ===================================================================== */
  function colOpen() { return $('#app').getAttribute('data-nowcol') === 'open'; }
  function colBody() {
    var t = cur();
    if (!t) return ui.emptyState({ art: 'wave', title: 'Nothing playing', text: 'Pick something from Home and it shows here.' });
    return html`<div class="nowcol__art cover-zoom">${ui.cover(t, '100%', { label: t.album, hero: true })}${ui.coverZoom(t.id)}</div>
      <div class="nowcol__info">${colInfo(t)}</div>
      <section class="nowcol__src" data-nowcol="src" aria-label="Source">${sourceCard()}</section>
      <section class="nowcol__ly" data-nowcol="lyrics" aria-label="Lyrics">${lyricsBlock('col')}</section>`;
  }
  function colInfo(t) {
    return html`${ui.playingFrom('nowcol__from')}<a class="nowcol__song" href="#/album/${t.albumId}">${t.title}</a>
      <a class="nowcol__artist" href="#/artist/${t.artistId}">${t.artist}</a>`;
  }
  App.nowColumn = {
    render: function (host) { if (!host) return; swap(host, colBody()); lyMountAll(); }
  };
  App.on('player', function (kind) {
    if (!colOpen()) return;
    if (kind === 'track') App.nowColumn.render($('#nowcol-body'));
    else if (kind === 'status' || kind === 'source' || kind === 'ended' || kind === 'state') swap($('[data-nowcol="src"]'), sourceCard());
    else if (kind === 'queue' && cur()) swap($('.nowcol__info'), colInfo(cur()));   // a radio started or stopped changes where it plays from
  });
  App.on('save', function (d) { if (colOpen() && d.id === P.currentId()) swap($('[data-nowcol="src"]'), sourceCard()); });

  App.route('/now', { render: render, mount: mount, title: function () { var t = cur(); return t ? t.title + ', Now Playing' : 'Now Playing'; }, view: 'now', nav: null,
    meta: function () { return App.fmt.formatLine(cur()); } });
})();
