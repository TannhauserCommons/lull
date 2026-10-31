/* screens/now.js: Now Playing (device, info stack, source card, lyrics), phone sheet tabs,
   and the Queue / Lyrics / Activity tabs of the context panel.

   Sections: 1 state + helpers, 2 device (full + compact), 3 info stack + source card,
   4 lyrics, 5 queue, 6 activity, 7 route + live updates, 8 actions, 9 panels, 10 prototype states. */
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
  var demo = { lyrics: null, radioError: false };   // forced by the state switcher
  var lyricsLoaded = {};                               // trackId -> true once "fetched" (mock lazy fetch)
  var lyAway = {};                                     // lyrics instance key -> user scrolled away
  var actFilter = 'all';
  var pendingFocus = null;                             // data-fk of the focused control before a re-render

  function cur() { return P.current(); }
  function busy() { return ['finding', 'queued', 'buffering', 'stalled'].indexOf(ps.status) >= 0; }
  function playingNow() { return ps.playing && cur() && ['idle', 'ended', 'failed', 'jellyfin-error'].indexOf(ps.status) < 0; }
  function peerSource(t) {
    if (!t || !ps.peer) return null;
    return t.sources.filter(function (s) { return s.peer === ps.peer.name; })[0] || null;
  }
  function onRoute() { return App.current.path === '/now'; }
  function bufferedAt(f) { return ps.buffered.some(function (r) { return f >= r[0] && f <= r[1]; }); }

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
    rememberFocus(el); el.innerHTML = String(markup); restoreFocus(el);
  }

  /* =====================================================================
     2. Device: screen pages (now / source / save), waveform, D-pad, A/B, pills, grill
     ===================================================================== */
  var LED = { idle: 'off', finding: 'search', queued: 'queued', buffering: 'buffer', streaming: 'stream', stalled: 'stalled',
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
    var fmt = ps.status === 'library' || !ps.peer ? t.format : ps.peer.format || t.format;
    var chip = ui.qualityChip(fmt, { solid: true });
    switch (ps.status) {
      case 'finding': return html`${chip}<span class="device__fact"><span class="spin-ring" aria-hidden="true"></span><span data-now-bind="peers">${ps.peersFound}</span>&nbsp;peers</span>`;
      case 'queued': return html`${chip}<span class="device__fact">queue #${ps.queuePos}</span>`;
      case 'stalled': return html`${chip}<span class="device__fact">next ${(ps.peer ? ps.peer.rank : 1) + 1}/${ps.peer ? ps.peer.of : 1}</span>`;
      case 'failed': return html`${screenBtn('retry', 'retry-search')}${screenBtn('pick source', 'pick-source')}`;
      case 'jellyfin-error': return html`${screenBtn('try soulseek', 'try-soulseek')}`;
      case 'ended': return html`${screenBtn('replay', 'play')}${screenBtn('radio', 'start-radio-current')}`;
      case 'library': return html`${chip}<span class="device__fact">library</span>`;
      default: return html`${chip}<span class="device__fact">peer&nbsp;<span data-bind="received">${Math.round(ps.received * 100)}%</span></span>`;
    }
  }
  function pageNow(t) {
    if (!t) return html`<div class="now-dev__idle"><span class="now-dev__slot" aria-hidden="true"></span><p class="device__title">Nothing playing</p><p class="device__artist">Search or pick something from Home</p></div>`;
    return html`<div class="device__now">${ui.cover(t, 'var(--now-art)')}<div class="now-dev__meta">
      <p class="device__title">${t.title}</p><p class="device__artist">${t.artist}</p>
      <div class="device__facts">${nowFacts(t)}</div></div></div>`;
  }
  function kv(rows) {
    return html`<dl class="device__kv">${rows.map(function (r) { return html`<dt>${r[0]}</dt><dd>${r[1]}</dd>`; })}</dl>`;
  }
  function pageSource(t) {
    if (!t) return kv([['source', 'none']]);
    if (ps.status === 'library' || ps.status === 'jellyfin-error' || !ps.peer) {
      return kv([['source', 'jellyfin'], ['format', t.format], ['size', t.sizeMB + ' MB'], ['state', ps.status === 'jellyfin-error' ? 'missing' : 'saved']]);
    }
    var s = peerSource(t) || {};
    var last = ps.failedPeers[ps.failedPeers.length - 1];
    return kv([
      ['peer', ps.peer.name], ['rank', ps.peer.rank + ' of ' + ps.peer.of], ['speed', (ps.status === 'stalled' ? 0 : ps.peer.speed) + ' MB/s'],
      ['slot', s.freeSlot ? 'free' : 'queue ' + (s.queue || 0)],
      ['received', raw('<span data-bind="received">' + Math.round(ps.received * 100) + '%</span>')],
      last ? ['fallback', last.peer + ' ' + last.at + '%'] : ['format', ps.peer.format || t.format]
    ]);
  }
  function pageSave(t) {
    if (!t) return html`<p class="now-dev__note">nothing to save</p>`;
    var s = App.saves.get(t.id);
    if (s) return html`<div class="now-dev__pipe">${ui.pipeline(s.step, s.status, s.error)}</div>`;
    if (App.isSaved(t.id)) return html`<p class="now-dev__note">${icon('saved')}in your library</p>${screenBtn('show album', 'go-album', t.id)}`;
    return html`<p class="now-dev__note">not saved yet</p>${screenBtn('save', 'save', t.id)}`;
  }
  function factsLine() { var t = cur(); return t ? String(ui.sourceBadge(t)) + String(nowFacts(t)) : ''; }
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
  function fromLine() {
    if (ps.radio) return html`${icon('radio', null, 'ic-16')}<span>playing from <b>${ps.radio.name}</b> radio</span>`;
    var t = cur();
    return t ? html`${icon('queue', null, 'ic-16')}<span>playing from your queue</span>` : html`${icon('sparkle', null, 'ic-16')}<span>nothing queued</span>`;
  }
  function reasonFor(t) {
    var pick = M.feed.picks.filter(function (p) { return p.trackId === t.id; })[0];
    if (pick) return ui.reasonChip(pick.reasons[0], { more: pick.reasons.length - 1, via: pick.via, itemId: t.id });
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
      <p class="now-head__by"><a class="link" href="#/artist/${t.artistId}">${t.artist}</a>, <a class="link-plain" href="#/album/${t.albumId}">${t.album}${a && a.type !== 'album' ? ' ' + a.type : ''}</a>, ${t.year}</p>
      <div class="now-head__chips">${t.tags.map(function (g) { return ui.tagChip(g); })}${reasonFor(t)}</div>`;
  }
  function actionRow() {
    var t = cur();
    if (!t) return html`<div class="now-acts"><a class="btn btn--primary" href="#/home">${icon('home')}Go home</a><a class="btn" href="#/search">${icon('search')}Search</a></div>`;
    var following = App.isFollowed(t.artistId);
    return html`<div class="now-acts">
      ${ui.saveButton(t.id, { cls: 'btn--primary', label: 'Save to library' })}
      ${actBtn2('plus', 'Add to playlist', 'add-to-playlist', 'data-id="' + t.id + '" aria-haspopup="menu"')}
      ${actBtn2('radio', 'Start radio', 'start-radio', 'data-kind="track" data-id="' + t.id + '"')}
      ${actBtn2(following ? 'check' : 'follow', following ? 'Following ' + t.artist : 'Follow ' + t.artist, 'follow', 'data-id="' + t.artistId + '" aria-pressed="' + following + '"', following ? 'Following' : 'Follow')}
      ${actBtn2('classic', 'Classic mode', 'classic', '', 'Classic')}
    </div>`;
  }

  function actBtn2(ic, label, act, attrs, short) {
    return html`<button class="btn now-acts__btn" data-act="${act}" ${raw(attrs)} aria-label="${label}" data-tip="${label}" data-fk="act-${act}">${icon(ic)}<span class="now-acts__label" aria-hidden="true">${short || label}</span></button>`;
  }

  /* source card: where the audio comes from + the 5-step save pipeline + a details expander */
  function sourceHead(t) {
    var src = peerSource(t);
    var size = (ps.peer && ps.peer.sizeMB) || (src && src.sizeMB) || t.sizeMB;
    switch (ps.status) {
      case 'library': return html`${icon('saved')}<span>Playing from <b>your library</b></span>${ui.qualityChip(t.format, { solid: true })}<span class="source-card__stat">jellyfin</span>`;
      case 'jellyfin-error': return html`${icon('warning')}<span><b>Library item unavailable.</b> Jellyfin can't find the file.</span><button class="btn btn--sm" data-act="try-soulseek" data-fk="src-try">Try Soulseek instead</button>`;
      case 'finding': return html`${icon('soulseek')}<span>Finding a source, <b data-now-bind="peers">${ps.peersFound}</b> peers answered</span>`;
      case 'queued': return html`${icon('clock')}<span>Queued at <b>${ps.peer.name}</b>, position ${ps.queuePos}</span>`;
      case 'failed': return html`${icon('warning')}<span><b>Couldn't play:</b> no peer responded</span><button class="btn btn--sm" data-act="retry-search" data-fk="src-retry">Retry search</button>`;
      case 'stalled': return html`${icon('refresh')}<span><b>${ps.peer.name}</b> stalled, switching to source ${ps.peer.rank + 1} of ${ps.peer.of}</span><button class="btn btn--sm" data-act="pick-source" data-fk="src-pick">Choose manually</button>`;
      case 'ended': return html`${icon('info')}<span>End of queue</span><button class="btn btn--sm" data-act="start-radio-current" data-fk="src-radio">Start radio</button>`;
      default: return html`${icon('soulseek')}<span>Streaming from <b>${ps.peer ? ps.peer.name : 'a peer'}</b></span>${ui.qualityChip(ps.peer && ps.peer.format || t.format, { solid: true })}<span class="source-card__stat">${ps.peer ? ps.peer.speed : 0} MB/s</span><span class="source-card__stat"><span data-bind="received">${Math.round(ps.received * 100)}%</span> of ${size} MB</span>`;
    }
  }
  function saveState(t) {
    var s = App.saves.get(t.id);
    if (s) return ui.pipeline(s.step, s.status, s.error);
    if (App.isSaved(t.id)) return ui.pipeline(5, 'saved');
    // not saved: five empty steps (pending = outline only)
    return html`<div class="pipeline-wrap" role="group" aria-label="Save progress"><div class="pipeline">${raw(new Array(6).join('<span class="pipeline__seg" aria-hidden="true"></span>'))}</div>
      <div class="pipeline__text"><span class="pipeline__step">not saved</span><span class="pipeline__label">Save keeps a tagged copy in your library</span></div></div>`;
  }
  function sourceDetails(t) {
    var src = peerSource(t), f = M.formats[(ps.peer && ps.peer.format) || t.format] || {};
    var rows = [['Album', html`<a class="link" href="#/album/${t.albumId}">${t.album}</a>, ${t.year}`], ['Format', f.detail || t.format]];
    if (ps.peer && ps.status !== 'library') {
      rows.push(['Peer', ps.peer.name + ', source ' + ps.peer.rank + ' of ' + ps.peer.of]);
      if (src) rows.push(['Upload slot', src.freeSlot ? 'Free now' : 'Queue of ' + src.queue]);
      rows.push(['Speed', ps.peer.speed + ' MB/s']);
    } else rows.push(['Source', 'Jellyfin library']);
    rows.push(['Size', ((ps.peer && ps.peer.sizeMB) || t.sizeMB) + ' MB']);
    if (ps.failedPeers.length) rows.push(['Fallbacks', ps.failedPeers.map(function (p) { return p.peer + ' stalled at ' + p.at + '%'; }).join(', ')]);
    return html`<dl class="now-src__dl">${rows.map(function (r) { return html`<div><dt>${r[0]}</dt><dd>${r[1]}</dd></div>`; })}</dl>
      <div class="now-src__more"><button class="btn btn--sm" data-act="now-other-sources" data-id="${t.id}" aria-expanded="${String(!!view.detailsTrack)}" data-fk="src-list">${icon('soulseek')}${view.detailsTrack ? 'Hide sources' : 'See all ' + t.sources.length + ' sources'}</button></div>
      ${view.detailsTrack && M.track(view.detailsTrack) ? sourcesTable(M.track(view.detailsTrack)) : ''}`;
  }
  function sourcesTable(t) {
    return html`${t.id !== P.currentId() ? html`<p class="now-src__line">${icon('info')}<span>Sources for <b>${t.title}</b>, ${t.artist}</span></p>` : ''}<div class="src-table now-src__table" role="list" aria-label="Sources for ${t.title}">${t.sources.map(function (s, i) {
      var on = ps.peer && ps.peer.name === s.peer;
      return html`<div class="src-row ${on ? 'is-preferred' : ''}" role="listitem"><span class="src-row__peer">${icon('peer')}${s.peer}</span><span>${ui.qualityChip(s.format)}</span><span class="src-row__opt">${s.speed} MB/s</span><span class="score" style="--s:${s.score / 100}" data-tip="score ${s.score}"><i></i></span>
        <button class="btn btn--sm" data-act="now-use-source" data-id="${t.id}" data-si="${i}" ${on ? raw('aria-current="true"') : ''} data-fk="use-${i}">${on ? 'Playing' : 'Use'}</button></div>`;
    })}</div>`;
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
    while (rows.length < 2 && i < ps.queue.length) { rows.push(qRow(ps.queue[i], i, { preview: true })); i++; }
    return html`${ui.labelRule('up next', { action: { label: 'Queue', act: 'now-open-queue' } })}
      ${rows.length ? html`<ol class="tlist tlist--dense now-q__list" aria-label="Up next">${rows}</ol>`
        : html`<p class="now-next__none">${icon('sparkle', null, 'ic-16')}Nothing up next.</p><button class="btn btn--sm" data-act="start-radio-current">${icon('radio')}Start radio</button>`}`;
  }

  /* =====================================================================
     4. Lyrics (LRCLIB shape): synced + word sync, plain, instrumental, not found, loading, error
     ===================================================================== */
  function parseLRC(text) {
    return text.split('\n').map(function (l) {
      var m = /^\[(\d+):(\d+(?:\.\d+)?)\]\s?(.*)$/.exec(l);
      return m ? { t: +m[1] * 60 + +m[2], key: (+m[1] * 60 + +m[2]).toFixed(2), text: m[3].trim() } : null;
    }).filter(Boolean);
  }
  function lyricsFor(t) {
    if (demo.lyrics === 'loading' || demo.lyrics === 'error') return { state: demo.lyrics };
    var rec = M.lyrics[t.id];
    if (!lyricsLoaded[t.id]) { fetchLyrics(t.id); return { state: 'loading' }; }
    if (!rec) return { state: 'notfound' };
    if (rec.instrumental) return { state: 'instrumental' };
    if (rec.syncedLyrics) return { state: 'synced', lines: parseLRC(rec.syncedLyrics), words: rec.wordSync || {} };
    if (rec.plainLyrics) return { state: 'plain', text: rec.plainLyrics };
    return { state: 'notfound' };
  }
  /* mock lazy fetch: the first look at a song's lyrics shows the loading state briefly */
  function fetchLyrics(id) {
    if (fetchLyrics[id]) return;
    /* PROTO: simulated delay */ fetchLyrics[id] = setTimeout(function () { lyricsLoaded[id] = true; delete fetchLyrics[id]; refreshLyrics(); }, 650);
  }
  function lyStep() { var s = +(App.storage('lyricStep') || 0); return App.clamp(isNaN(s) ? 0 : s, -2, 2); }
  function lyTag(L) {
    var map = { synced: Object.keys(L.words || {}).length ? 'word sync' : 'synced', plain: 'not synced' };
    return map[L.state] ? html`<span class="now-ly__tag">${map[L.state]}</span>` : '';
  }
  function lyTools() {
    var s = lyStep();
    return html`<span class="now-ly__size" role="group" aria-label="Lyrics text size">
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-ly-size" data-dir="-1" aria-label="Smaller lyrics" data-tip="Smaller text" ${s <= -2 ? raw('disabled') : ''} data-fk="ly-minus"><span class="now-ly__a" aria-hidden="true">A-</span></button>
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-ly-size" data-dir="1" aria-label="Larger lyrics" data-tip="Larger text" ${s >= 2 ? raw('disabled') : ''} data-fk="ly-plus"><span class="now-ly__a" aria-hidden="true">A+</span></button></span>`;
  }
  function lyLines(L, key) {
    var i = currentLine(L.lines), out = [];
    L.lines.forEach(function (l, n) {
      if (!l.text) { out.push(html`<li class="now-ly__gap" aria-hidden="true"><i></i><i></i><i></i></li>`); return; }
      var words = L.words[l.key], cls = n < i ? 'is-past' : n === i ? 'is-current' : '';
      var inner = words ? raw(words.map(function (w, k) {
        return '<span class="now-ly__w" data-t="' + w[0] + '">' + App.esc(w[1]) + (k < words.length - 1 ? ' ' : '') + '</span>';
      }).join('')) : l.text;
      out.push(html`<li><button class="now-ly__line ${cls}" data-t="${l.t}" data-i="${n}" tabindex="${n === Math.max(i, firstSung(L.lines)) ? 0 : -1}" ${n === i ? raw('aria-current="true"') : ''} data-fk="ly-${key}-${n}">${inner}</button></li>`);
    });
    return out;
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
    else if (L.state === 'error') body = html`<div class="now-ly__state">${ui.emptyState({ art: 'star', title: 'Lyrics unavailable right now', text: 'LRCLIB asked us to slow down (too many requests). Trying again in 40\u00a0s.', action: { label: 'Try again', act: 'now-ly-retry' } })}</div>`;
    else if (L.state === 'notfound') body = html`<div class="now-ly__state">${ui.emptyState({ art: 'slot', title: 'No lyrics found', text: 'LRCLIB has nothing for this song yet. Check again later.', action: { label: 'Check again', act: 'now-ly-retry' } })}</div>`;
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
      <div class="now-ly__foot"><span class="now-ly__attr">Lyrics from LRCLIB</span>${L.state === 'synced' || L.state === 'plain' ? lyTools() : ''}</div>
    </div>`;
  }
  function refreshLyrics() {
    var c = $('[data-now="lyrics"]'); if (c) swap(c, lyricsBlock('page'));
    if (App.store.get('ctxTab') === 'lyrics' || $('#ctx-body .now-ly')) App.refreshPanel();
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
    if (i >= 0) $$('.now-ly__w', lines[i]).forEach(function (w) { w.classList.toggle('is-sung', +w.getAttribute('data-t') <= p); });
    // lines outside the downloaded part cannot be jumped to: say so on hover / focus
    var dur = P.duration() || 1, bk = ps.buffered.map(function (r) { return r.join('-'); }).join('|');
    if (bk !== st.bk) {
      st.bk = bk;
      lines.forEach(function (l) {
        var ok = bufferedAt(+l.getAttribute('data-t') / dur);
        l.setAttribute('data-tip', ok ? 'Jump to ' + fmtTime(+l.getAttribute('data-t')) : 'Not downloaded yet (' + Math.round(ps.received * 100) + '%)');
        l.classList.toggle('is-far', !ok);
      });
    }
  }
  function lyMountAll() { $$('.now-ly__scroll[data-sync]').forEach(function (sc) { if (!sc._ly) lyInit(sc); }); }
  function lyTick() { $$('.now-ly__scroll[data-sync]').forEach(function (sc) { if (!sc._ly) lyInit(sc); else lyUpdate(sc, false); }); }
  document.addEventListener('click', function (e) {
    var line = e.target.closest('.now-ly__line'); if (!line) return;
    var t = +line.getAttribute('data-t'), sc = line.closest('.now-ly__scroll');
    if (bufferedAt(t / (P.duration() || 1))) {
      P.seek(t); if (!ps.playing) P.play();
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
    if (i === ps.index + 1) return html`<span class="resolve now-q__mark is-ready">${icon('check', null, 'ic-16')}ready</span>`;
    return '';
  }
  function qRow(q, i, opts) {
    opts = opts || {};
    var t = M.track(q.trackId); if (!t) return '';
    var playing = i === ps.index, saved = App.isSaved(t.id);
    var where = playing ? 'now playing' : 'position ' + (i - ps.index) + ' in the queue';
    var markLabel = q.resolve === 'none' ? ', no source found' : q.resolve === 'resolving' ? ', finding a source' : i === ps.index + 1 ? ', ready' : '';
    return html`<li class="trow trow--dense now-q__row${playing ? ' is-playing' : ''}${playing && ps.playing ? ' is-playing-now' : ''}${q.resolve === 'none' && !playing ? ' is-unavailable' : ''}"
        data-track-id="${t.id}" data-qi="${i}" data-uid="${q.uid}" tabindex="0" ${opts.preview || playing ? '' : raw('draggable="true"')} data-fk="row-${q.uid}"
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
    var steer = ps.radio.steer == null ? .4 : ps.radio.steer;
    return html`<div class="now-q__radio">
      <div class="now-q__radio-row">${icon('radio')}<span class="now-q__radio-name">Radio: <b>${ps.radio.name}</b></span>
        <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="now-radio-stop" aria-label="Stop radio" data-tip="Stop radio" data-fk="radio-stop">${icon('close', null, 'ic-16')}</button></div>
      <label class="switch-row now-q__keep"><span class="switch-row__label">Keeps adding songs</span>
        <button class="switch" role="switch" aria-checked="${ps.radio.keepsAdding ? 'true' : 'false'}" data-act="now-radio-keep" aria-label="Radio keeps adding songs" data-fk="radio-keep"></button></label>
      <div class="now-q__steer">
        <label class="now-q__steer-label" for="now-steer-${App.uid('s')}">Steer</label>
        <input class="range" type="range" min="0" max="100" step="5" value="${Math.round(steer * 100)}" style="--fill:${Math.round(steer * 100)}%" data-now-steer aria-valuetext="${steerText(steer)}" data-fk="radio-steer">
        <div class="now-q__steer-ends" aria-hidden="true"><span>Familiar</span><span>Adventurous</span></div>
      </div>
      ${demo.radioError ? html`<div class="now-q__err" role="alert">${icon('warning', null, 'ic-16')}<span>Radio paused: couldn't find playable songs.</span><button class="btn btn--sm" data-act="now-radio-retry" data-fk="radio-retry">Try again</button></div>` : ''}
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
      ps.queue.forEach(function (q, i) { if (i > ps.index && q.group === g[0]) items.push(qRow(q, i)); });
      if (!items.length) return;
      var label = g[0] === 'radio' && !(ps.radio && ps.radio.keepsAdding) ? 'from radio' : g[1];
      rows.push(html`<li class="now-q__group label-rule label-rule--sm"><h3>${label}</h3><span class="now-q__count">${items.length}</span></li>`);
      rows = rows.concat(items);
    });
    var after = ps.queue.length - ps.index - 1;
    return html`<div class="now-q">
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
    App.announce('Moved ' + t.title + ' to position ' + (to - ps.index));
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
  /* drag to reorder inside the queue: insertion line between rows (drops from other lists still append) */
  var qDrag = null;
  document.addEventListener('dragstart', function (e) {
    var row = e.target.closest && e.target.closest('.now-q__row[draggable="true"]');
    qDrag = row ? { from: +row.getAttribute('data-qi') } : null;
  });
  document.addEventListener('dragend', function () { qDrag = null; clearDropLine(); });
  function clearDropLine() { $$('.now-q__row.is-drop-before, .now-q__row.is-drop-after').forEach(function (r) { r.classList.remove('is-drop-before', 'is-drop-after'); }); }
  function qTarget(e) {   // the row under the pointer, or 'end' anywhere else in the queue
    var host = e.target.closest && e.target.closest('.now-q, #ctx-body'); if (!host) return null;
    var row = e.target.closest('.now-q__row');
    return row && row.closest('[data-now-qlist]') ? row : 'end';
  }
  document.addEventListener('dragover', function (e) {
    if (!qDrag) return;
    var row = qTarget(e); if (!row) return;
    e.preventDefault(); e.stopPropagation(); e.dataTransfer.dropEffect = 'move';
    clearDropLine(); if (row === 'end') return;
    var r = row.getBoundingClientRect(), after = e.clientY > r.top + r.height / 2;
    row.classList.add(after ? 'is-drop-after' : 'is-drop-before');
  }, true);
  document.addEventListener('drop', function (e) {
    if (!qDrag) return;
    var row = qTarget(e); if (!row) return;
    e.preventDefault(); e.stopPropagation();
    var to = ps.queue.length;
    if (row !== 'end') { var r = row.getBoundingClientRect(); to = +row.getAttribute('data-qi') + (e.clientY > r.top + r.height / 2 ? 1 : 0); }
    var from = qDrag.from; if (to > from) to -= 1;
    clearDropLine(); qDrag = null;
    if (from === ps.index) { App.toast('That song is playing. Move the others around it.', { icon: 'info' }); return; }
    to = Math.max(to, ps.index + 1);
    if (to !== from) { var t = M.track(ps.queue[from].trackId); P.move(from, to); App.announce('Moved ' + t.title); }
  }, true);

  /* =====================================================================
     6. Activity: transfers (hatched progress), saves (5 steps), imports, rescans; filters; All quiet
     ===================================================================== */
  function jobs() {
    var list = (App.store.get('activity') || []).slice(), saves = App.store.get('saves') || {};
    Object.keys(saves).forEach(function (id) {
      var s = saves[id];
      list = list.filter(function (j) { return !(j.kind === 'save' && j.trackId === id); });
      list.unshift({ id: 'save-' + id, kind: 'save', trackId: id, step: s.step, live: true, error: s.error,
        stepState: s.status === 'saving' ? 'running' : s.status === 'failed' ? 'failed' : 'done' });
    });
    return list;
  }
  function jobState(j) {
    if (j.kind === 'transfer') return j.state === 'failed' ? 'failed' : j.state === 'transferring' || j.state === 'queued' ? 'active' : 'done';
    if (j.kind === 'save') return j.stepState === 'running' ? 'active' : j.stepState === 'failed' ? 'failed' : 'done';
    return j.state === 'running' ? 'active' : j.state === 'failed' ? 'failed' : 'done';
  }
  function actBtn(ic, label, act, j, extra) {
    return html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="${act}" data-job="${j.id}" ${extra ? raw(extra) : ''} aria-label="${label}" data-tip="${label}" data-fk="${act}-${j.id}">${icon(ic, null, 'ic-16')}</button>`;
  }
  function jobRow(j) {
    var t = j.trackId ? M.track(j.trackId) : null, st = jobState(j), meta, extra = '', acts = '';
    var live = j.kind === 'transfer' && j.trackId === P.currentId() && j.state === 'transferring';
    if (j.kind === 'transfer') {
      var pct = live ? Math.round(ps.received * 100) : j.pct;
      if (j.state === 'transferring') { meta = html`${icon('soulseek', null, 'ic-16')}<span>${j.peer}, ${live && ps.peer ? ps.peer.speed : j.speed} MB/s, <span ${live ? raw('data-bind="received"') : ''}>${pct}%</span></span>`; extra = html`<div class="progress progress--xfer" style="--p:${pct / 100}" ${live ? raw('data-now-xfer') : ''}><i></i></div>`; acts = html`${actBtn('close', 'Cancel transfer', 'now-job-cancel', j)}`; }
      else if (j.state === 'queued') { meta = html`${icon('clock', null, 'ic-16')}<span>Queued at ${j.peer}, position ${j.queuePos}</span>`; extra = html`<div class="progress" style="--p:0"><i></i></div>`; acts = actBtn('close', 'Cancel transfer', 'now-job-cancel', j); }
      else if (j.state === 'failed') { meta = html`${ui.statusGlyph('down', 'Failed')}<span>${j.error}</span>`; extra = html`<div class="progress progress--failed" style="--p:${j.pct / 100}"><i></i></div>`; acts = html`${actBtn('refresh', 'Retry transfer', 'now-job-retry', j)}${actBtn('soulseek', 'Pick another source', 'show-sources', j, 'data-id="' + j.trackId + '"')}`; }
      else { meta = html`${icon('close', null, 'ic-16')}<span>Cancelled at ${j.pct}%</span>`; acts = actBtn('refresh', 'Start again', 'now-job-retry', j); }
    } else if (j.kind === 'save') {
      meta = '';
      extra = ui.pipeline(j.step, j.stepState === 'running' ? 'saving' : j.stepState === 'failed' ? 'failed' : 'saved', j.error);
      if (st === 'active') acts = actBtn('close', 'Cancel save', 'now-job-cancel', j);
      else if (st === 'failed') acts = html`${App.discover ? actBtn('info', 'What went wrong', 'now-job-fix', j) : ''}${actBtn('refresh', 'Retry save', 'now-job-retry', j)}`;
      else acts = actBtn('library', 'Show album', 'go-album', j, 'data-id="' + j.trackId + '"');
    } else if (j.kind === 'import') {
      meta = html`${ui.statusGlyph('ok', 'Done')}<span>${j.matched} matched, ${j.search} searching, ${j.notFound} not found</span>`;
      acts = html`<button class="btn btn--sm btn--quiet" data-act="now-job-review" data-fk="review-${j.id}">Review</button>`;
    } else {
      meta = html`${ui.statusGlyph('ok', 'Done')}<span>${j.detail}</span>`;
    }
    var title = t ? t.title : j.kind === 'import' ? j.name : 'Jellyfin rescan';
    var sub = t ? t.artist : j.kind === 'import' ? 'Playlist import' : 'Library';
    var art = t ? ui.cover(t, 40) : html`<span class="now-act__glyph" aria-hidden="true">${icon(j.kind === 'import' ? 'import' : 'refresh')}</span>`;
    return html`<li class="now-act__row is-${st}">
      <div class="now-act__art">${art}</div>
      <div class="now-act__main">
        <div class="now-act__title"><b>${title}</b><span>${sub}</span></div>
        ${meta ? html`<div class="now-act__meta">${meta}</div>` : ''}
        ${extra}
      </div>
      <div class="now-act__acts">${acts}</div>
    </li>`;
  }
  function activityBlock() {
    var all = jobs(), counts = { all: all.length, active: 0, done: 0, failed: 0 };
    all.forEach(function (j) { counts[jobState(j)]++; });
    if (!all.length) return html`<div class="now-act">${ui.emptyState({ art: 'moon', title: 'All quiet', text: 'Transfers, saves and imports show up here while they run.' })}</div>`;
    var shown = all.filter(function (j) { return actFilter === 'all' || jobState(j) === actFilter; });
    var order = { active: 0, failed: 1, done: 2 };
    shown.sort(function (a, b) { return order[jobState(a)] - order[jobState(b)]; });
    var groups = [['transfer', 'transfers'], ['save', 'saves'], ['other', 'imports and rescans']].map(function (g) {
      var items = shown.filter(function (j) { return g[0] === 'other' ? (j.kind === 'import' || j.kind === 'rescan') : j.kind === g[0]; });
      return items.length ? html`${ui.labelRule(g[1], { level: 'h3', sm: true })}<ul class="now-act__list">${items.map(jobRow)}</ul>` : '';
    });
    return html`<div class="now-act">
      <div class="now-act__filters" role="group" aria-label="Filter activity">${['all', 'active', 'done', 'failed'].map(function (f) {
        return html`<button class="pill-filter" data-act="now-act-filter" data-value="${f}" aria-pressed="${String(actFilter === f)}" data-fk="f-${f}">${f[0].toUpperCase() + f.slice(1)} <span class="now-act__n">${counts[f]}</span></button>`;
      })}</div>
      ${shown.length ? groups : html`<p class="now-next__none">${icon('check', null, 'ic-16')}Nothing ${actFilter} right now.<button class="btn btn--sm btn--quiet" data-act="now-act-filter" data-value="all">Show all</button></p>`}
      <div class="now-q__foot"><button class="btn btn--sm" data-act="now-act-clear" ${counts.done ? '' : raw('disabled')} data-fk="act-clear">${icon('check')}Clear finished</button></div>
    </div>`;
  }
  function setJob(id, patch) {
    App.store.update('activity', function (list) { return list.map(function (j) { return j.id === id ? Object.assign({}, j, patch) : j; }); });
  }

  /* =====================================================================
     7. Route: /now (desktop two columns, tablet, phone sheet with Art / Lyrics / Queue / Details)
     ===================================================================== */
  function phoneTabs() {
    var i = TABS.indexOf(view.tab);
    return html`<div class="seg now-seg" role="tablist" aria-label="Now Playing views" style="--n:4;--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${TABS.map(function (tb, n) {
      return html`<button class="seg__btn" role="tab" id="now-tab-${tb}" aria-selected="${String(n === i)}" tabindex="${n === i ? 0 : -1}" aria-controls="now-panel" data-act="now-tab" data-tab="${tb}">${tb[0].toUpperCase() + tb.slice(1)}</button>`;
    })}</div>`;
  }
  function render(ctx) {
    var q = ctx.query;
    if (q.tab && TABS.indexOf(q.tab) >= 0) view.tab = q.tab;
    view.detailsTrack = q.track || null;
    if (q.tab === 'details') view.detailsOpen = true;
    var t = cur();
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
  function mount(ctx) {
    var root = $('.now', ctx.el); if (!root) return;
    updateTextHooks();
    // "power on": the device lifts in and the screen wakes (reduced motion: a short fade)
    var devs = $$('.now-dev, .now-devc', root);
    if (App.reducedMotion()) {
      devs.forEach(function (d) { d.classList.add('now-fade'); });
      requestAnimationFrame(function () { requestAnimationFrame(function () { devs.forEach(function (d) { d.classList.remove('now-fade'); }); }); });
    } else {
      devs.forEach(function (d) { d.classList.add('is-power-on'); });
      setTimeout(function () { devs.forEach(function (d) { d.classList.remove('is-power-on'); }); }, 1100);
    }
    lastRender = Date.now();
    lyMountAll();
    App.syncRows();
    if (ctx.query.tab === 'queue' && window.innerWidth >= 640) App.openPanel('queue');
    if (ctx.query.tab === 'details' || ctx.query.tab === 'lyrics') {
      var target = $(ctx.query.tab === 'details' ? '.now-srccard' : '.now-lycard', root);
      if (target && window.innerWidth >= 640) setTimeout(function () { target.scrollIntoView({ block: 'start', behavior: App.reducedMotion() ? 'auto' : 'smooth' }); }, 650);
    }
    var offs = [
      App.on('player', onPlayer),
      App.on('save', function (d) { if (d.id === P.currentId()) { updateSaveBits(); } }),
      App.on('like', function () { var t = cur(); $$('[data-now="b-label"]').forEach(function (el) { el.textContent = t && App.isLiked(t.id) ? 'liked' : 'like'; }); }),
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
    $$('[data-now="a-label"]').forEach(function (el) { el.textContent = on ? 'pause' : 'play'; });
    $$('[data-now="led"]').forEach(function (el) { el.textContent = ledText(); });
    $$('[data-now="head"]').forEach(function (el) { el.textContent = headText(); });
    $$('[data-now="state"]').forEach(function (el) { el.textContent = stateText(); });
    $$('.now .device__pill--shuffle').forEach(function (b) { b.setAttribute('aria-pressed', String(ps.shuffle)); });
    $$('.now .device__pill--repeat').forEach(function (b) {
      b.setAttribute('aria-pressed', String(ps.repeat !== 'off'));
      b.setAttribute('aria-label', { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat] + ' (R)');
    });
    $$('[data-now="repeat"]').forEach(function (el) { el.textContent = repeatLabel(); });
  }
  /* text-only live hooks for custom heroes: data-now="title|artist|album|format" (format = App.fmt.formatLine) */
  function updateTextHooks() {
    var t = cur();
    var text = { title: t ? t.title : 'Nothing playing', artist: t ? t.artist : '', album: t ? t.album : '', format: App.fmt.formatLine(t) };
    Object.keys(text).forEach(function (k) { $$('[data-now="' + k + '"]').forEach(function (el) { if (el.textContent !== text[k]) el.textContent = text[k]; }); });
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
      $$('[data-now="b-label"]').forEach(function (el) { el.textContent = t && App.isLiked(t.id) ? 'liked' : 'like'; });
      lyAway = {};
      $$('[data-now="facts"]').forEach(function (el) { swap(el, factsLine()); });
      swap($('[data-now="head-info"]'), infoHead());
      swap($('[data-now="acts"]'), actionRow());
      swap($('[data-now="src"]'), sourceCard());
      swap($('[data-now="lyrics"]'), lyricsBlock('page'));
      swap($('[data-now="next"]'), upNext());
      swap($('[data-now="queue"]'), queueBlock());
      $$('.now .now__top .now-head__from').forEach(function (el) { swap(el, fromLine()); });
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

  /* one live tick for lyrics, peer counters and the current transfer's progress */
  App.on('tick', function () {
    lyTick();
    $$('[data-now-bind="peers"]').forEach(function (el) { el.textContent = ps.peersFound; });
    $$('[data-now-xfer]').forEach(function (el) { el.style.setProperty('--p', ps.received.toFixed(3)); });
  });

  /* =====================================================================
     8. Actions
     ===================================================================== */
  App.action('now-page', function () {
    view.page = PAGES[(PAGES.indexOf(view.page) + 1) % PAGES.length];
    var t = cur();
    $$('.now').forEach(function (n) { n.setAttribute('data-page', view.page); });
    $$('[data-now="page"]').forEach(function (p) { p.innerHTML = String(pageBody(t)); });
    $$('.now .device__select').forEach(function (b) { b.innerHTML = view.page + window.icon('chevron-right', null, 'ic-16'); b.setAttribute('aria-label', selectLabel()); });
    updateControls();
    App.announce('Screen: ' + view.page + ' page');
  });
  App.action('now-vol', function (el) { P.setVolume((ps.muted ? 0 : ps.volume) + .05 * +el.getAttribute('data-dir')); showOsd(); });
  App.action('now-nothing', function () { App.toast('Nothing playing yet. Pick something from Home.', { icon: 'info' }); });
  App.action('now-tab', function (el) { setTab(el.getAttribute('data-tab'), true); });
  function setTab(tab, focus) {
    view.tab = tab;
    try { history.replaceState(null, '', '#/now?tab=' + tab); } catch (e) { /* file:// in some browsers */ }
    var root = $('.now'); if (!root) return;
    root.setAttribute('data-tab', tab);
    var seg = $('.now-seg', root), i = TABS.indexOf(tab);
    seg.style.setProperty('--i', i);
    $$('.seg__btn', seg).forEach(function (b, n) { b.setAttribute('aria-selected', String(n === i)); b.tabIndex = n === i ? 0 : -1; if (n === i && focus) b.focus(); });
    if (tab === 'details' && !view.detailsOpen) { view.detailsOpen = true; swap($('[data-now="src"]'), sourceCard()); }
    $$('.now-ly__scroll[data-sync]', root).forEach(function (sc) { if (sc._ly) { sc._ly.cur = -2; lyUpdate(sc, true); } });
  }
  document.addEventListener('keydown', function (e) {        // arrow keys move between the phone tabs
    var b = e.target.closest && e.target.closest('.now-seg .seg__btn'); if (!b) return;
    var i = TABS.indexOf(b.getAttribute('data-tab')), n = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: TABS.length - 1 }[e.key];
    if (n == null) return;
    e.preventDefault(); setTab(TABS[(n + TABS.length) % TABS.length], true);
  });
  App.action('now-details', function () {
    view.detailsOpen = !view.detailsOpen;
    swap($('[data-now="src"]'), sourceCard());
  });
  App.action('now-other-sources', function (el) {
    view.detailsTrack = view.detailsTrack ? null : el.getAttribute('data-id');
    swap($('[data-now="src"]'), sourceCard());
  });
  App.action('now-use-source', function (el) {
    var id = el.getAttribute('data-id'), si = +el.getAttribute('data-si'), s = M.track(id).sources[si];
    P.useSource(id, si); App.toast('Now from ' + s.peer + ', ' + s.format, { icon: 'soulseek' });
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
    demo.lyrics = null; lyricsLoaded[t.id] = false; refreshLyrics();
  });

  // queue
  App.action('now-q-jump', function (el) { jumpTo(+el.getAttribute('data-qi')); });
  App.action('now-q-menu', function (el) {
    var qi = +el.getAttribute('data-qi'), q = ps.queue[qi], t = M.track(q.trackId), playing = qi === ps.index;
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
    items.push({ label: 'Show sources', icon: 'soulseek', run: function () { App.hooks.sourcePicker(t.id); } });
    App.menu(el, items, { label: 'Queue actions for ' + t.title });
  });
  App.action('now-q-clear', function () { P.clear(); });
  App.action('now-q-playlist', function () {
    var ids = ps.queue.map(function (q) { return q.trackId; }), id = 'pl-queue-' + Date.now();
    var name = ps.radio ? ps.radio.name.toLowerCase() + ' radio' : 'saved queue';
    App.store.update('playlists', function (l) {
      l.push({ id: id, name: name, desc: 'Saved from the queue.', created: '2026-10-31', updated: '2026-10-31',
        items: ids.map(function (tid) { return { trackId: tid, added: '2026-10-31' }; }) });
    });
    App.toast('Saved ' + ids.length + ' songs as playlist "' + name + '"', { icon: 'playlists', action: 'Open', onAction: function () { App.go('/playlist/' + id); } });
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
  App.action('now-radio-retry', function () { demo.radioError = false; App.refreshPanel(); swap($('[data-now="queue"]'), queueBlock()); App.toast('Looking for more radio songs', { icon: 'radio' }); });
  document.addEventListener('input', function (e) {
    var r = e.target.closest && e.target.closest('[data-now-steer]'); if (!r) return;
    var v = +r.value / 100; r.style.setProperty('--fill', r.value + '%'); r.setAttribute('aria-valuetext', steerText(v));
    if (ps.radio) ps.radio.steer = v;                      // no queue event: keeps the slider under the pointer
  });
  document.addEventListener('change', function (e) {
    var r = e.target.closest && e.target.closest('[data-now-steer]'); if (!r) return;
    App.toast('Radio steering: ' + steerText(+r.value / 100).toLowerCase() + '. Upcoming songs will follow.', { icon: 'radio' });
  });

  // activity
  App.action('now-act-filter', function (el) { actFilter = el.getAttribute('data-value'); withFocus('f-' + actFilter, App.refreshPanel); });
  App.action('now-act-clear', function () {
    var before = App.store.get('activity').slice(), savesBefore = Object.assign({}, App.store.get('saves'));
    App.store.set('activity', before.filter(function (j) { return jobState(j) !== 'done'; }));
    Object.keys(savesBefore).forEach(function (id) { if (savesBefore[id].status === 'saved') App.saves.set(id, null); });
    App.toast('Cleared finished jobs', { icon: 'check', action: 'Undo', onAction: function () {
      App.store.set('activity', before); Object.keys(savesBefore).forEach(function (id) { App.saves.set(id, savesBefore[id]); });
    } });
  });
  function findJob(id) { return jobs().filter(function (j) { return j.id === id; })[0]; }
  App.action('now-job-cancel', function (el) {
    var j = findJob(el.getAttribute('data-job')); if (!j) return;
    var t = M.track(j.trackId);
    if (j.kind === 'save') {
      if (j.live) App.saves.set(j.trackId, null); else App.store.update('activity', function (l) { return l.filter(function (x) { return x.id !== j.id; }); });
      App.toast('Save cancelled: ' + t.title, { icon: 'close' });
      return;
    }
    var pct = j.trackId === P.currentId() ? Math.round(ps.received * 100) : j.pct;
    setJob(j.id, { state: 'cancelled', pct: pct });
    App.toast('Transfer cancelled: ' + t.title, { icon: 'close', action: 'Undo', onAction: function () { setJob(j.id, { state: j.state, pct: j.pct }); } });
  });
  App.action('now-job-retry', function (el) {
    var j = findJob(el.getAttribute('data-job')); if (!j) return;
    var t = M.track(j.trackId);
    if (j.kind === 'save') {
      if (!j.live) App.store.update('activity', function (l) { return l.filter(function (x) { return x.id !== j.id; }); });
      App.saves.set(j.trackId, { step: j.step || 1, status: 'saving', pct: 0 });
      App.toast('Retrying save: ' + t.title, { icon: 'refresh' });
      return;
    }
    var next = t.sources[1] || t.sources[0];
    setJob(j.id, { state: 'transferring', peer: next.peer, speed: next.speed, error: null });
    App.toast('Retrying from ' + next.peer, { icon: 'soulseek' });
  });
  App.action('now-job-fix', function (el) {              // the recovery sheet reads the live save record
    var j = findJob(el.getAttribute('data-job')); if (!j || !App.discover) return;
    if (!j.live) App.saves.set(j.trackId, { step: j.step, status: 'failed', error: j.error, frozen: true });
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
    render: function (body) { rememberFocus(body); return lyricsBlock('panel'); },
    mount: function (body) { lyMountAll(); restoreFocus(body); }
  });
  App.panel('activity', {
    label: 'Activity',
    render: function (body) { rememberFocus(body); return activityBlock(); },
    mount: function (body) { restoreFocus(body); }
  });

  App.route('/now', { render: render, mount: mount, title: function () { var t = cur(); return t ? t.title + ', Now Playing' : 'Now Playing'; }, view: 'now', nav: null,
    meta: function () { return App.fmt.formatLine(cur()); } });

  /* =====================================================================
     10. Prototype states (listed by the state switcher)
     ===================================================================== */
  function reset() {
    demo.lyrics = null; demo.radioError = false; lyAway = {}; view.page = 'now'; view.detailsOpen = false;
    App.saves.set('t-satellite-porch-1', null);
  }
  function streamingRadio(pos, playing) {
    P.setRadio(null);
    P.playList(M.queue.items.map(function (q) { return q.trackId; }), 0, { radio: Object.assign({}, M.queue.radio) });
    ps.queue.forEach(function (q, i) { q.group = M.queue.items[i].group; q.resolve = M.queue.items[i].resolve; });
    lyricsLoaded['t-satellite-porch-1'] = true;
    P.setState({ status: 'streaming', playing: playing !== false, received: .62, buffered: [[0, .58]], pos: pos == null ? 84 : pos, durationKnown: false,
      peer: { name: 'otter-lamp', rank: 1, of: 7, speed: 2.4, format: 'FLAC 16/44.1', sizeMB: 31.4 } });
  }
  function lyricState(id, label, trackIds, extra) {
    return { id: id, label: label, route: '/now', apply: function () {
      reset(); view.tab = 'lyrics';
      if (trackIds) { P.playList(trackIds, 0); P.setState({ status: 'library', playing: true, received: 1, buffered: [[0, 1]], durationKnown: true, pos: 40 }); trackIds.forEach(function (i) { lyricsLoaded[i] = true; }); }
      else streamingRadio(27, true);
      if (extra) extra();
      if (window.innerWidth >= 1100) App.openPanel('lyrics');
    } };
  }
  App.registerStates('now', [
    { id: 'now-streaming', label: 'Now Playing: streaming from a peer', route: '/now', apply: function () { reset(); view.tab = 'art'; streamingRadio(84, true); } },
    { id: 'now-library', label: 'Now Playing: playing from library', route: '/now', apply: function () { reset(); view.tab = 'art'; P.playList(['t-lantern-season-1', 't-lantern-season-2', 't-glasshouse-bloom-1'], 0); P.setState({ pos: 61, playing: true }); lyricsLoaded['t-lantern-season-1'] = true; } },
    { id: 'now-paused', label: 'Now Playing: paused', route: '/now', apply: function () { reset(); streamingRadio(84, false); } },
    { id: 'now-page-source', label: 'Now Playing: device Source page', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.page = 'source'; } },
    { id: 'now-page-save', label: 'Now Playing: device Save page, saving step 2', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.page = 'save'; App.saves.set('t-satellite-porch-1', { step: 2, status: 'saving', pct: 100, frozen: true }); } },
    { id: 'now-save-failed', label: 'Now Playing: save failed at cover art', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.page = 'save'; App.saves.set('t-satellite-porch-1', { step: 3, status: 'failed', error: 'No cover art found. Retry or save without art.' }); } },
    { id: 'now-finding', label: 'Now Playing: finding a source', route: '/now', apply: function () { reset(); streamingRadio(0, true); P.setState({ status: 'finding', peersFound: 23, received: 0, buffered: [], pos: 0 }); } },
    { id: 'now-stalled', label: 'Now Playing: peer stalled, falling back', route: '/now', apply: function () { reset(); streamingRadio(70, true); P.setState({ status: 'stalled', received: .38, buffered: [[0, .36]], failedPeers: [{ peer: 'mossbank', at: 21 }] }); view.detailsOpen = true; } },
    { id: 'now-failed', label: 'Now Playing: all sources failed', route: '/now', apply: function () { reset(); streamingRadio(0, false); P.setState({ status: 'failed', playing: false, received: 0, buffered: [], error: 'Couldn\'t play: no peer responded' }); } },
    { id: 'now-idle', label: 'Now Playing: nothing queued', route: '/now', apply: function () { reset(); P.setState({ queue: [], index: 0, status: 'idle', playing: false, radio: null }); } },
    { id: 'now-phone-details', label: 'Now Playing (phone): Details tab', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.tab = 'details'; view.detailsOpen = true; } },
    { id: 'now-phone-queue', label: 'Now Playing (phone): Queue tab', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.tab = 'queue'; } },
    lyricState('now-lyrics-word', 'Lyrics: synced with word highlight'),
    lyricState('now-lyrics-synced', 'Lyrics: synced (line by line)', ['t-marble-soda-club-1']),
    lyricState('now-lyrics-scrolled', 'Lyrics: scrolled away, back to current line', null, function () { ps.pos = 96; lyAway.page = lyAway.panel = true; }),
    lyricState('now-lyrics-plain', 'Lyrics: plain, not synced', ['t-glasshouse-bloom-1']),
    lyricState('now-lyrics-instrumental', 'Lyrics: instrumental', ['t-harbour-lights-2']),
    lyricState('now-lyrics-notfound', 'Lyrics: not found', ['t-night-bus-2']),
    lyricState('now-lyrics-loading', 'Lyrics: loading', null, function () { demo.lyrics = 'loading'; }),
    lyricState('now-lyrics-error', 'Lyrics: unavailable (rate limited)', null, function () { demo.lyrics = 'error'; }),
    { id: 'now-queue-radio', label: 'Queue: radio with keeps-adding and steer', route: '/now', apply: function () { reset(); streamingRadio(84, true); view.tab = 'queue'; App.openPanel('queue'); } },
    { id: 'now-queue-user', label: 'Queue: your own queue, no radio', route: '/now', apply: function () {
      reset(); view.tab = 'queue';
      P.playList(['t-lantern-season-1', 't-glasshouse-bloom-1', 't-marble-soda-club-1', 't-lantern-season-2', 't-lantern-season-3', 't-lantern-season-4'], 0);
      ps.queue[1].group = 'next'; ps.queue[2].group = 'next'; ps.queue[2].resolve = 'resolving'; ps.queue[5].resolve = 'none';
      P.setState({ pos: 30, playing: true }); App.openPanel('queue');
    } },
    { id: 'now-queue-radio-error', label: 'Queue: radio paused, no playable songs', route: '/now', apply: function () { reset(); streamingRadio(84, true); demo.radioError = true; view.tab = 'queue'; App.openPanel('queue'); } },
    { id: 'now-queue-empty', label: 'Queue: empty', route: '/now', apply: function () { reset(); P.setState({ queue: [], index: 0, status: 'idle', playing: false, radio: null }); view.tab = 'queue'; App.openPanel('queue'); } },
    { id: 'now-activity', label: 'Activity: transfers, saves, imports', route: '/now', apply: function () { reset(); actFilter = 'all'; App.store.set('activity', M.activity.map(function (j) { return Object.assign({}, j); })); App.openPanel('activity'); } },
    { id: 'now-activity-failed', label: 'Activity: failed filter', route: '/now', apply: function () { reset(); actFilter = 'failed'; App.store.set('activity', M.activity.map(function (j) { return Object.assign({}, j); })); App.openPanel('activity'); } },
    { id: 'now-activity-quiet', label: 'Activity: all quiet', route: '/now', apply: function () {
      reset(); actFilter = 'all'; App.store.set('activity', []);
      Object.keys(App.store.get('saves')).forEach(function (id) { App.saves.set(id, null); });
      App.openPanel('activity');
    } }
  ]);
})();
