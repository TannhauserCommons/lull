/* screens/classic.js: Classic mode (Webamp stage, skin drawer, Classic window chrome).
   A full-viewport stage over the dimmed app: glossy title pill with three orbs (Exit / Skins / Help),
   the Webamp mount slot, a skin drawer (Browse / Favourites / Downloaded) and a footer notice.
   The prototype never loads Webamp; it draws a placeholder of the slot tinted with the active skin.

   ---------------------------------------------------------------------------------------------
   Real-app integration (Webamp 2.3.1, pinned, lazy-loaded the first time the stage opens, never on boot)

     var webamp = new Webamp({
       initialSkin: { url: skinUrl(appliedMd5) },               // same-origin cached .wsz (see museum notes below)
       availableSkins: favourites.map(function (s) { return { url: skinUrl(s.md5), name: s.name }; }),
                                                                 // also listed in Webamp's own Options > Skins menu
       initialTracks: queue.map(function (t) {                   // always pass metaData + duration, otherwise
         return { url: t.streamUrl, metaData: { artist: t.artist, title: t.title }, duration: t.duration };
       }),                                                       // Webamp fetches each file's head to read them
       windowLayout: {                                           // px; phones close the equalizer (275 px wide fits)
         main: { position: { top: 0, left: 0 } },
         equalizer: { position: { top: 116, left: 0 }, closed: isPhone },
         playlist: { position: { top: isPhone ? 116 : 232, left: 0 }, size: { extraWidth: 0, extraHeight: 0 } }
       },
       enableHotkeys: false,                                     // lull owns shortcuts ([data-webamp] silences ours)
       enableMediaSession: false,                                // lull owns Media Session and forwards to Webamp
       zIndex: 36,                                               // just above --z-classic (35), below sheets/popovers/toasts
       handleTrackDropEvent: dropIntoAppQueue,                   // returns Track[] | null
       handleSaveListEvent: exportM3u, handleLoadListEvent: importM3u
     });
     webamp.onWillClose(function (cancel) { cancel(); exitClassic(); });   // Webamp's own close = leave Classic
     webamp.renderInto(document.getElementById('webamp-stage'));            // since 2.3.0; host is position: relative
     Preview / apply: webamp.setSkinFromUrl(url); webamp.skinIsLoaded().then(onApplied). The server validates each
       .wsz when it caches it (a zip holding main.bmp); a load that has not resolved after ~10 s counts as failed and
       setSkinFromUrl(previousUrl) restores the old skin.
     Hand-off: on enter, pause the app engine and give Webamp the queue, index and position (setCurrentTrack,
       seekToTime). Webamp has no play/pause/seek events, so on exit read getMediaStatus(), getPlaylistTracks(),
       isShuffleEnabled(), isRepeatEnabled() back into the app engine. Keep the instance: dispose() still leaks,
       so create once, close() on exit and reopen() on the next visit.
     A dropped local .wsz: webamp.setSkinFromUrl(URL.createObjectURL(file)).

   Winamp Skin Museum (public GraphQL, https://api.webamp.org/graphql, CORS allowed; ~105k skins):
     Browse:  search_classic_skins(query, first: 30, offset, nsfw: false) when searching (debounce 300 ms),
              skins(first: 30, offset, sort: MUSEUM, filter: APPROVED) otherwise -> { count, nodes }.
     Fields:  md5 (stable id for favourites), filename, screenshot_url (275 x 348 png), download_url (.wsz),
              museum_url (https://skins.webamp.org/skin/<md5>), nsfw.
     download_url sends no CORS headers, so the server downloads and caches skins and serves them same-origin.
     Cards show <img src=screenshot_url width=275 height=348> with image-rendering: pixelated; the prototype
     draws a stand-in from each mock skin's palette in the same box (.cls-shot).
   --------------------------------------------------------------------------------------------- */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$, $$ = App.$$;

  var MUSEUM_TOTAL = 105577;      // count from skins(sort: MUSEUM); shown as context, mock list is 8
  var PREVIEW_SECONDS = 15;

  /* ---------- model (kept across visits, so exit + re-enter finds things as they were) ---------- */
  var S;
  function freshModel() {
    var saved = App.storage('classic') || {};
    var favs = saved.favs, dl = saved.downloaded;
    return {
      skins: M.skins.map(function (s) {
        return Object.assign({}, s, {
          favourite: favs ? favs.indexOf(s.id) >= 0 : s.favourite,
          downloaded: dl ? dl.indexOf(s.id) >= 0 : s.downloaded
        });
      }),
      applied: saved.applied || M.settings.appearance.defaultSkin,
      tab: 'browse', query: '', searching: false, loading: false, offline: false,
      drawer: null,                 // null = by width (docked open on desktop, closed below)
      noticeHidden: !!saved.noticeHidden,
      preview: null,                // { id, prev, left }
      busy: null,                   // { id, mode: 'apply'|'preview', phase: 'download'|'apply', pct }
      failed: null,                 // { id, kept }
      failOnce: { '1be94d08': true } // Graphite 98: first load fails (damaged file), retry works
    };
  }
  S = freshModel();
  function persist() {
    App.storage('classic', {
      applied: S.applied, noticeHidden: S.noticeHidden,
      favs: S.skins.filter(function (s) { return s.favourite; }).map(function (s) { return s.id; }),
      downloaded: S.skins.filter(function (s) { return s.downloaded; }).map(function (s) { return s.id; })
    });
  }
  function skin(id) { return S.skins.filter(function (s) { return s.id === id; })[0]; }
  function skinVars(s) {
    return s ? '--sk1:' + s.palette[0] + ';--sk2:' + s.palette[1] + ';--sk3:' + s.palette[2] : '';
  }
  function museumUrl(s) { return 'https://skins.webamp.org/skin/' + s.id; }  // real field: museum_url (md5 ids)
  function isDocked() { return window.innerWidth >= 1100; }
  function drawerOpen() { return S.drawer === null ? isDocked() : S.drawer; }

  /* ---------- the stage element (lives on <body>, above the inert app) ---------- */
  var root = null, lastPage = null, restore = null, closedAt = 0, timers = [];
  var searchTimer = 0, previewTimer = 0;

  /* remember the page Classic was opened from: it renders dimmed behind the stage, and Exit returns to it */
  App.on('route', function (cur) {
    if (cur.path === '/classic') return;
    lastPage = { path: cur.path, params: cur.params, query: cur.query, route: cur.route };
    if (restore && restore.path === cur.path) {         // exit: scroll position and focus come back too
      var r = restore; restore = null;
      $('#view').scrollTop = r.scroll;
      if (r.opener && document.contains(r.opener) && r.opener.offsetParent !== null) r.opener.focus({ preventScroll: true });
    } else restore = null;
  });

  function behindMarkup(ctx) {
    if (!lastPage || !lastPage.route || !lastPage.route.render) return '';
    var q = Object.assign({}, lastPage.query); delete q.state;
    try {
      return lastPage.route.render({ params: lastPage.params, query: q, state: null, el: ctx.el, route: lastPage.route, onCleanup: function () {} });
    } catch (e) { return ''; }
  }
  function lastHref() {
    if (!lastPage) return '/home';
    var q = Object.keys(lastPage.query).filter(function (k) { return k !== 'state'; })
      .map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(lastPage.query[k]); }).join('&');
    return lastPage.path + (q ? '?' + q : '');
  }

  /* ---------- markup ---------- */
  function stageMarkup() {
    return html`<div class="cls-scrim" aria-hidden="true"></div>
      <div class="cls-crt">
        <header class="cls-bar win">
          <div class="win__bar">
            <span class="win__orbs">
              <button class="orb orb--blue" data-act="cls-exit" aria-label="Exit Classic mode" data-tip="Exit (Esc)"></button>
              <button class="orb orb--lav" data-act="cls-drawer" aria-controls="cls-drawer" aria-expanded="${drawerOpen() ? 'true' : 'false'}" aria-label="Skins" data-tip="Skins"></button>
              <button class="orb orb--mag" data-act="cls-help" aria-haspopup="dialog" aria-expanded="false" aria-label="Help" data-tip="Help"></button>
            </span>
            <h1 class="win__title" id="cls-title">classic</h1>
          </div>
        </header>
        <div class="cls-desk" id="cls-desk">${deskMarkup()}</div>
        <footer class="cls-foot" id="cls-foot">${footMarkup()}</footer>
        <aside class="cls-drawer win s-panel" id="cls-drawer" aria-label="Skins" ${drawerOpen() ? '' : raw('inert')}>${drawerMarkup()}</aside>
      </div>
      <i class="cls-crt-line" aria-hidden="true"></i>`;
  }

  /* the Webamp slot: the real app calls webamp.renderInto() on #webamp-stage */
  function deskMarkup() {
    var shown = S.preview ? skin(S.preview.id) : skin(S.applied);
    var t = App.player.current();
    return html`<div class="cls-slot" id="webamp-stage" data-webamp style="${skinVars(shown)}">
        <span class="cls-slot__tag s-card">${icon('classic', null, 'ic-16')}Webamp renders here</span>
        <span class="cls-slot__tag cls-slot__tag--drop s-card">${icon('import', null, 'ic-16')}Drop a .wsz to apply it</span>
        <div class="cls-ghost">
          <div class="cls-ghost__body" role="img" aria-label="${'Placeholder for the Webamp windows (main, equalizer, playlist) in the ' + (shown ? shown.name : '') + ' skin'}">
            <span class="cls-shot cls-shot--ghost"><i class="cls-shot__win cls-shot__win--main"></i><i class="cls-shot__win cls-shot__win--eq"></i><i class="cls-shot__win cls-shot__win--pl"></i></span>
            <span class="cls-ghost__legend">
              <span class="cls-ghost__tag s-card">main <b>275x116</b></span>
              <span class="cls-ghost__tag s-card">equalizer <b>275x116</b></span>
              <span class="cls-ghost__tag s-card">playlist <b>275x116+</b></span>
            </span>
          </div>
          <p class="cls-ghost__cap s-card">
            <span class="cls-ghost__skin">${shown ? shown.name : ''}${S.preview ? html` <em>(preview)</em>` : ''}</span>
            <span class="fg-2">${t ? t.title + ', ' + t.artist : 'Queue is empty'}</span>
          </p>
        </div>
        ${S.busy ? busyMarkup() : ''}
        ${S.failed ? failedMarkup() : ''}
      </div>
      ${S.preview ? previewMarkup() : ''}`;
  }
  function busyMarkup() {
    var s = skin(S.busy.id), dl = S.busy.phase === 'download';
    return html`<div class="cls-busy s-card" role="status">
      <span class="spin-ring" aria-hidden="true"></span>
      <div class="cls-busy__text">
        <strong>${dl ? 'Downloading ' : 'Applying '}${s.name}</strong>
        <span class="fg-2">${dl ? 'Fetching the skin from the museum and caching it' : 'Waiting for Webamp to load the skin'}</span>
        <span class="progress${dl ? ' progress--xfer' : ''}" aria-hidden="true"><i style="--p:${dl ? S.busy.pct : 1}"></i></span>
      </div>
    </div>`;
  }
  function failedMarkup() {
    var s = skin(S.failed.id), kept = skin(S.failed.kept);
    return html`<div class="cls-failed banner banner--down s-card" role="alert">
      ${icon('warning')}<span class="banner__msg">Couldn't load ${s.name}: the skin file is damaged. ${kept ? kept.name : 'The previous skin'} stays on.</span>
      <button class="btn btn--sm" data-act="cls-retry" data-id="${s.id}">Try again</button>
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-dismiss-failed" aria-label="Dismiss" data-tip="Dismiss">${icon('close', null, 'ic-16')}</button>
    </div>`;
  }
  function previewMarkup() {
    var s = skin(S.preview.id), left = S.preview.left;
    return html`<div class="cls-preview s-card" role="group" aria-label="Skin preview">
      <div class="cls-preview__text">
        <strong>Previewing ${s.name}</strong>
        <span class="fg-2 num" id="cls-count">Reverts in ${left} s unless you keep it</span>
        <span class="progress" aria-hidden="true"><i id="cls-count-bar" style="--p:${left / PREVIEW_SECONDS}"></i></span>
      </div>
      <div class="cls-preview__acts">
        <button class="btn" data-act="cls-revert">Revert</button>
        <button class="btn btn--primary" data-act="cls-keep">Keep</button>
      </div>
    </div>`;
  }
  function footMarkup() {
    return html`${S.noticeHidden ? '' : html`<div class="cls-notice s-card" role="note">
        ${icon('info')}<p>Some features differ in classic mode: Webamp plays your queue, so crossfade, loudness normalisation and gapless playback pause, and repeat is on or off only.</p>
        <button class="btn btn--quiet btn--sm" data-act="cls-notice">Got it</button>
      </div>`}
      <div class="cls-exit s-card">
        <button class="btn btn--sm cls-skins-btn" data-act="cls-drawer" aria-controls="cls-drawer" aria-expanded="${drawerOpen() ? 'true' : 'false'}">${icon('grid')}Skins</button>
        <kbd class="kbd" aria-hidden="true">Esc</kbd>
        <button class="btn btn--sm" data-act="cls-exit">${icon('chevron-left')}Back to lull</button>
      </div>`;
  }

  var TABS = [['browse', 'Browse'], ['favourites', 'Favourites'], ['downloaded', 'Downloaded']];
  function drawerMarkup() {
    var i = TABS.map(function (t) { return t[0]; }).indexOf(S.tab);
    var ph = S.tab === 'browse' ? (S.offline ? 'Search downloaded skins' : 'Search ' + MUSEUM_TOTAL.toLocaleString('en') + ' skins')
      : S.tab === 'favourites' ? 'Search favourites' : 'Search downloaded skins';
    return html`<div class="win__bar">
        <span class="win__title" id="cls-drawer-title">skins</span>
        <button class="btn-icon btn-icon--bare btn-icon--sm cls-drawer__close" data-act="cls-drawer" aria-label="Close skins" data-tip="Close skins">${icon('close', null, 'ic-16')}</button>
      </div>
      <div class="cls-drawer__head">
        <div class="seg" role="tablist" aria-label="Skin lists" style="--n:3;--i:${i}" id="cls-tabs">
          <span class="seg__ind" aria-hidden="true"></span>
          ${TABS.map(function (t) {
            var on = t[0] === S.tab;
            return html`<button class="seg__btn" role="tab" id="cls-tab-${t[0]}" aria-controls="cls-list" aria-selected="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-act="cls-tab" data-id="${t[0]}">${t[1]}</button>`;
          })}
        </div>
        <div class="search-field" role="search">
          ${icon('search')}
          <label class="vh" for="cls-search">${ph}</label>
          <input class="input" id="cls-search" type="search" placeholder="${ph}" autocomplete="off" spellcheck="false" value="${S.query}">
          <span class="search-field__end">
            <button class="btn-icon btn-icon--bare btn-icon--sm search-field__clear" data-act="cls-clear" aria-label="Clear search" data-tip="Clear" ${S.query ? '' : raw('hidden')}>${icon('close', null, 'ic-16')}</button>
          </span>
        </div>
      </div>
      <div class="cls-drawer__body" id="cls-list" role="tabpanel" aria-labelledby="cls-tab-${S.tab}" tabindex="-1">${listMarkup()}</div>
      <div class="cls-drawer__foot">
        <button class="btn btn--sm" data-act="cls-open-file">${icon('import')}Open .wsz file</button>
        <input type="file" accept=".wsz" id="cls-file" hidden>
        <a class="cls-attrib" href="https://skins.webamp.org/" target="_blank" rel="noopener">Skins from the Winamp Skin Museum</a>
      </div>`;
  }

  function visibleSkins() {
    var q = S.query.trim().toLowerCase();
    return S.skins.filter(function (s) {
      if (s.nsfw && !M.settings.appearance.showNsfwSkins) return false;
      if (S.tab === 'favourites' && !s.favourite) return false;
      if ((S.tab === 'downloaded' || S.offline) && !s.downloaded) return false;
      return !q || (s.name + ' ' + s.file).toLowerCase().indexOf(q) >= 0;
    });
  }
  function listMarkup() {
    var notes = [];
    if (S.offline) notes.push(html`<div class="banner banner--warn s-card cls-note">${icon('offline')}<span class="banner__msg">You're offline, so only downloaded skins are listed.</span></div>`);
    if (S.loading) {
      return html`${notes}<p class="vh" role="status">Loading skins</p><ul class="cls-grid" aria-hidden="true">${[0, 1, 2, 3].map(function () {
        return raw('<li class="cls-card cls-card--skel"><span class="skel cls-skel-shot"></span><span class="skel skel--title" style="width:70%"></span><span class="skel skel--text" style="width:45%"></span></li>');
      })}</ul>`;
    }
    if (S.searching) return html`${notes}<p class="cls-meta" role="status"><span class="spin-ring" aria-hidden="true"></span>Searching the museum</p>`;
    var list = visibleSkins();
    if (!list.length) return html`${notes}${emptyMarkup()}`;
    var meta = S.tab === 'browse' && !S.offline
      ? (S.query ? list.length + (list.length === 1 ? ' skin matches' : ' skins match') + ' "' + S.query + '"'
        : 'Most loved first, ' + list.length + ' of ' + MUSEUM_TOTAL.toLocaleString('en') + ' shown')
      : list.length + (list.length === 1 ? ' skin' : ' skins');
    return html`${notes}<p class="cls-meta" role="status">${meta}</p>
      <ul class="cls-grid">${list.map(cardMarkup)}</ul>
      ${S.tab === 'browse' && !M.settings.appearance.showNsfwSkins ? html`<p class="cls-nsfw" id="cls-nsfw">${icon('lock', null, 'ic-16')}<span>Skins marked NSFW are hidden. <a href="#/settings/appearance">Show them in Settings</a></span></p>` : ''}`;
  }
  function emptyMarkup() {
    if (S.query) return ui.emptyState({ art: 'star', title: 'No skins match "' + S.query + '"', text: 'Try a shorter word, or a colour such as purple or mint.', action: { label: 'Clear search', act: 'cls-clear' } });
    if (S.tab === 'favourites') return ui.emptyState({ art: 'star', title: 'No favourite skins yet', text: 'Star a skin to keep it here. Favourites also appear in Webamp\'s own skins menu.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
    return ui.emptyState({ art: 'slot', title: 'Nothing downloaded yet', text: 'Skins you preview or apply are cached here, so they work offline.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
  }
  function cardMarkup(s) {
    var applied = S.applied === s.id && !S.preview, previewing = S.preview && S.preview.id === s.id;
    var busy = S.busy && S.busy.id === s.id, failed = S.failed && S.failed.id === s.id;
    var status = busy ? html`<span class="spin-ring" aria-hidden="true"></span>${S.busy.phase === 'download' ? 'Downloading' : 'Applying'}`
      : failed ? html`${icon('warning', null, 'ic-16')}Couldn't load`
      : previewing ? html`${icon('sparkle', null, 'ic-16')}Previewing`
      : applied ? html`${icon('check', null, 'ic-16')}Applied`
      : s.downloaded ? html`${icon('saved', null, 'ic-16')}Downloaded`
      : html`${icon('external', null, 'ic-16')}On the museum`;
    var offlineBlocked = S.offline && !s.downloaded;
    return html`<li class="cls-card s-card${applied ? ' is-applied' : ''}${previewing ? ' is-previewing' : ''}${failed ? ' is-failed' : ''}" data-skin="${s.id}">
      <button class="cls-card__shot" data-act="cls-preview" data-id="${s.id}" data-tip="Preview" aria-label="${'Preview ' + s.name}" ${busy ? raw('aria-disabled="true"') : ''}>
        <span class="cls-shot" style="${skinVars(s)}" aria-hidden="true"><i class="cls-shot__win cls-shot__win--main"></i><i class="cls-shot__win cls-shot__win--eq"></i><i class="cls-shot__win cls-shot__win--pl"></i></span>
      </button>
      <div class="cls-card__text">
        <span class="cls-card__name ellipsis">${s.name}</span>
        <span class="cls-card__file ellipsis">${s.file}</span>
        <span class="cls-card__status${failed ? ' is-bad' : ''}">${status}</span>
      </div>
      <div class="cls-card__acts">
        ${applied ? html`<button class="btn btn--sm" aria-disabled="true" data-act="cls-applied" data-id="${s.id}">Applied</button>`
          : failed ? html`<button class="btn btn--sm" data-act="cls-retry" data-id="${s.id}">Retry</button>`
          : html`<button class="btn btn--sm" data-act="cls-apply" data-id="${s.id}" ${busy || offlineBlocked ? raw('aria-disabled="true"') : ''}>Apply</button>`}
        <span class="spacer"></span>
        <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-fav" data-id="${s.id}" aria-pressed="${s.favourite ? 'true' : 'false'}" aria-label="${'Favourite ' + s.name}" data-tip="${s.favourite ? 'Remove from favourites' : 'Add to favourites'}">${icon(s.favourite ? 'star-fill' : 'star', null, 'ic-16')}</button>
        <a class="btn-icon btn-icon--bare btn-icon--sm" href="${museumUrl(s)}" target="_blank" rel="noopener" data-act="cls-museum" data-id="${s.id}" aria-label="${'View ' + s.name + ' on the Winamp Skin Museum'}" data-tip="${S.offline ? 'Needs a connection' : 'View on the Winamp Skin Museum'}" ${S.offline ? raw('aria-disabled="true"') : ''}>${icon('external', null, 'ic-16')}</a>
      </div>
    </li>`;
  }

  /* ---------- partial re-renders (focus stays where it was) ---------- */
  function focusKey() {
    var a = document.activeElement;
    if (!root || !a || !root.contains(a)) return null;
    if (a.id) return '#' + a.id;
    var act = a.getAttribute('data-act');
    return act ? '[data-act="' + act + '"]' + (a.getAttribute('data-id') ? '[data-id="' + a.getAttribute('data-id') + '"]' : '') : null;
  }
  function refocus(key, fallback) {
    if (!root) return;
    var el = key && $(key, root);
    if (!el && fallback) el = $(fallback, root);
    if (el && document.activeElement !== el) el.focus({ preventScroll: true });
  }
  function renderList() {
    if (!root) return;
    var k = focusKey(), body = $('#cls-list', root), top = body.scrollTop;
    body.innerHTML = String(listMarkup()); body.scrollTop = top;
    if (k && k !== '#cls-search') refocus(k, '#cls-list');
  }
  function renderDesk() {
    if (!root) return;
    var k = focusKey();
    $('#cls-desk', root).innerHTML = String(deskMarkup());
    if (k) refocus(k);
  }
  function renderDrawer() {
    if (!root) return;
    var k = focusKey(), d = $('#cls-drawer', root);
    d.innerHTML = String(drawerMarkup());
    bindDrawer();
    if (k) refocus(k);
  }
  function renderFoot() { if (root) $('#cls-foot', root).innerHTML = String(footMarkup()); }
  function renderAll() { renderDesk(); renderList(); }

  function syncDrawer() {
    if (!root) return;
    var open = drawerOpen(), d = $('#cls-drawer', root);
    root.setAttribute('data-drawer', open ? 'open' : 'closed');
    if (open) d.removeAttribute('inert'); else d.setAttribute('inert', '');
    $$('[data-act="cls-drawer"][aria-expanded]', root).forEach(function (b) { b.setAttribute('aria-expanded', open ? 'true' : 'false'); });
  }
  var drawerOpener = null;
  /* byUser: move focus into the drawer on open, and back to whatever opened it on close */
  function setDrawer(open, byUser, opener) {
    S.drawer = open;
    if (!root) return;
    syncDrawer();
    if (!byUser) return;
    if (open) {
      drawerOpener = opener || null;
      later(function () { var t = $('#cls-drawer .seg__btn[aria-selected="true"]', root); if (t) t.focus(); }, 30);
    } else {
      var back = drawerOpener && root.contains(drawerOpener) && drawerOpener.offsetParent !== null ? drawerOpener : $('.orb--lav', root);
      back.focus();
    }
  }

  /* ---------- skin flows: preview (Keep / Revert), apply, failure, local .wsz ---------- */
  function load(id, mode) {
    var s = skin(id);
    if (!s) return;
    if (S.busy) { App.toast('Wait for ' + skin(S.busy.id).name + ' to finish loading', { icon: 'clock' }); return; }
    if (S.offline && !s.downloaded) { App.toast('You\'re offline. ' + s.name + ' isn\'t downloaded yet.', { icon: 'offline' }); return; }
    if (S.preview && mode === 'apply' && S.preview.id === id) { keep(); return; }
    if (S.preview) endPreview(false, true);
    S.failed = null;
    S.busy = { id: id, mode: mode, phase: s.downloaded ? 'apply' : 'download', pct: 0 };
    App.announce((s.downloaded ? 'Applying ' : 'Downloading ') + s.name);
    renderAll();
    step();
    function step() {
      if (!S.busy || S.busy.id !== id) return;
      if (S.busy.phase === 'download') {
        S.busy.pct = Math.min(1, S.busy.pct + 0.25);
        var bar = root && $('.cls-busy .progress > i', root); if (bar) bar.style.setProperty('--p', S.busy.pct);
        if (S.busy.pct >= 1) { s.downloaded = true; S.busy.phase = 'apply'; renderAll(); }
        later(step, 300); return;
      }
      later(finish, 700);                              // skinIsLoaded()
    }
    function finish() {
      if (!S.busy || S.busy.id !== id) return;
      S.busy = null;
      if (S.failOnce[id]) {
        delete S.failOnce[id];
        S.failed = { id: id, kept: S.applied };
        App.announce('Couldn\'t load ' + s.name + '. ' + skin(S.applied).name + ' stays on.');
        renderAll(); persist(); return;
      }
      if (mode === 'preview') startPreview(id);
      else { S.applied = id; App.toast(s.name + ' is your classic skin', { icon: 'check', kind: 'ok' }); App.announce(s.name + ' applied'); }
      persist(); renderAll();
    }
  }
  function startPreview(id) {
    S.preview = { id: id, prev: S.applied, left: PREVIEW_SECONDS };
    App.announce('Previewing ' + skin(id).name + '. Keep it or it reverts in ' + PREVIEW_SECONDS + ' seconds.');
    clearInterval(previewTimer);
    previewTimer = setInterval(tick, 1000);
  }
  function tick() {
    if (!S.preview) { clearInterval(previewTimer); return; }
    var bar = root && $('.cls-preview', root);
    if (bar && (bar.matches(':hover') || bar.contains(document.activeElement))) return;   // paused while in use
    S.preview.left -= 1;
    if (S.preview.left <= 0) { endPreview(false); return; }
    var c = root && $('#cls-count', root), b = root && $('#cls-count-bar', root);
    if (c) c.textContent = 'Reverts in ' + S.preview.left + ' s unless you keep it';
    if (b) b.style.setProperty('--p', S.preview.left / PREVIEW_SECONDS);
  }
  function endPreview(kept, quiet) {
    if (!S.preview) return;
    var p = S.preview, s = skin(p.id), focusInBar = root && $('.cls-preview', root) && $('.cls-preview', root).contains(document.activeElement);
    clearInterval(previewTimer); S.preview = null;
    if (kept) S.applied = p.id;
    persist();
    if (!quiet) {
      App.toast(kept ? s.name + ' is your classic skin' : 'Back to ' + skin(p.prev).name,
        kept ? { icon: 'check', kind: 'ok' } : { icon: 'undo', action: 'Preview again', onAction: function () { load(p.id, 'preview'); } });
      renderAll();
      if (focusInBar) refocus('[data-act="cls-preview"][data-id="' + p.id + '"]', '#cls-search');
    }
  }
  function keep() { endPreview(true); }

  function wszName(file) {
    return file.replace(/\.wsz$/i, '').replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim()
      .replace(/\b\w/g, function (c) { return c.toUpperCase(); }) || 'Local skin';
  }
  /* a dropped or opened .wsz: applied locally (real app: setSkinFromUrl(URL.createObjectURL(file))) */
  function applyLocal(file) {
    var h = 0; for (var i = 0; i < file.name.length; i++) h = (h * 31 + file.name.charCodeAt(i)) >>> 0;
    var id = 'local-' + h.toString(16);
    if (!skin(id)) {
      var base = M.skins[h % M.skins.length];   // stand-in thumbnail colours; the real app reads the file
      S.skins.unshift({ id: id, name: wszName(file.name), file: file.name, palette: base.palette.slice(), favourite: false, downloaded: true, nsfw: false, local: true });
    }
    S.tab = 'downloaded'; S.query = '';
    if (root) { renderDrawer(); setDrawer(true); }
    load(id, 'apply');
  }

  var prevImport = App.hooks.importFile, pendingFile = null;
  App.hooks.importFile = function (file) {
    if (!/\.wsz$/i.test(file.name)) { prevImport(file); return; }
    if (App.current.path === '/classic') applyLocal(file);
    else { pendingFile = file; App.go('/classic'); }
  };

  /* ---------- actions ---------- */
  function exit() {
    if (!root || root.classList.contains('is-leaving')) return;
    if (S.preview) endPreview(false, true);
    root.classList.add('is-leaving');
    later(function () { App.go(lastHref()); }, App.reducedMotion() ? 130 : 200);
  }
  App.action('cls-exit', exit);
  App.action('cls-drawer', function (el) { setDrawer(!drawerOpen(), true, el); });
  App.action('cls-help', function (el) {
    App.popover(el, html`<div class="popover__head">${icon('classic')}<span>classic mode</span></div>
      <div class="popover__body cls-help">
        <p>Webamp plays your queue here with its own skins. Drag its windows anywhere on the stage.</p>
        <ul class="cls-help__keys">
          <li><kbd class="kbd">Esc</kbd><span>Back to lull, where you left off</span></li>
          <li><kbd class="kbd">W</kbd><span>Classic mode on or off</span></li>
        </ul>
        <p class="fg-2">Crossfade, loudness normalisation and gapless playback pause in classic mode, and repeat is on or off only.</p>
      </div>
      <div class="popover__actions">
        <a class="btn btn--quiet btn--sm" href="#/settings/appearance">Default skin in Settings</a>
        <button class="btn btn--sm" data-act="cls-help-close">Got it</button>
      </div>`, { label: 'Classic mode help', className: 'cls-pop' });
  });
  App.action('cls-help-close', function () { App.closePopover(true); });
  App.action('cls-tab', function (el) { setTab(el.getAttribute('data-id'), true); });
  function clearSearch() {
    S.query = ''; S.searching = false; clearTimeout(searchTimer);
    var i = root && $('#cls-search', root); if (i) { i.value = ''; i.focus(); }
    syncClear(); renderList();
  }
  App.action('cls-clear', clearSearch);
  App.action('cls-open-file', function () { var f = root && $('#cls-file', root); if (f) { f.value = ''; f.click(); } });
  App.action('cls-preview', function (el) {
    var id = el.getAttribute('data-id');
    if (S.preview && S.preview.id === id) { App.toast('Already previewing ' + skin(id).name, { icon: 'sparkle' }); return; }
    if (S.applied === id && !S.preview) { App.toast(skin(id).name + ' is already on', { icon: 'check' }); return; }
    load(id, 'preview');
  });
  App.action('cls-apply', function (el) {
    var s = skin(el.getAttribute('data-id'));
    if (S.busy && S.busy.id === s.id) return;
    if (S.offline && !s.downloaded) { App.toast('You\'re offline. ' + s.name + ' isn\'t downloaded yet.', { icon: 'offline' }); return; }
    load(s.id, 'apply');
  });
  App.action('cls-applied', function (el) { App.toast(skin(el.getAttribute('data-id')).name + ' is already your classic skin', { icon: 'check' }); });
  App.action('cls-retry', function (el) { load(el.getAttribute('data-id'), 'apply'); });
  App.action('cls-dismiss-failed', function () { S.failed = null; renderAll(); refocus(null, '#cls-search'); });
  App.action('cls-keep', keep);
  App.action('cls-revert', function () { endPreview(false); });
  App.action('cls-notice', function () {
    S.noticeHidden = true; persist(); renderFoot();
    refocus('.cls-exit .btn');
    App.toast('Notice hidden. Help keeps it.', { icon: 'info' });
  });
  App.action('cls-fav', function (el) {
    var s = skin(el.getAttribute('data-id'));
    s.favourite = !s.favourite; persist();
    if (S.tab === 'favourites' && !s.favourite) {
      App.toast('Removed ' + s.name + ' from favourites', { icon: 'star', action: 'Undo', onAction: function () { s.favourite = true; persist(); renderList(); } });
    } else App.announce(s.favourite ? s.name + ' added to favourites' : s.name + ' removed from favourites');
    renderList();
    if (!$('[data-act="cls-fav"][data-id="' + s.id + '"]', root)) refocus(null, '#cls-list');
  });
  /* museum links: the prototype's ids are fictional, so it says where the link goes instead of opening a 404 */
  App.action('cls-museum', function (el, e) {
    if (e) e.preventDefault();
    var s = skin(el.getAttribute('data-id'));
    if (S.offline) { App.toast('The museum needs a connection', { icon: 'offline' }); return; }
    App.toast(s ? 'Opens ' + s.name + ' on the Winamp Skin Museum in your browser' : 'Opens the Winamp Skin Museum in your browser', { icon: 'external' });
  });

  function setTab(id, focus) {
    if (TABS.map(function (t) { return t[0]; }).indexOf(id) < 0) return;
    S.tab = id; S.query = ''; S.searching = false; S.loading = false; clearTimeout(searchTimer);
    renderDrawer();
    if (focus) refocus('#cls-tab-' + id);
  }
  function syncClear() {
    var c = root && $('.search-field__clear', root);
    if (c) c.hidden = !S.query;
  }

  /* drawer listeners that cannot be delegated clicks: search typing, tab arrow keys, file input */
  function bindDrawer() {
    var d = $('#cls-drawer', root);
    var input = $('#cls-search', d);
    input.addEventListener('input', function () {
      S.query = input.value; syncClear();
      clearTimeout(searchTimer);
      if (S.tab === 'browse' && !S.offline && S.query.trim()) {        // museum search: debounce 300 ms, then a round trip
        S.searching = true; renderList();
        /* PROTO: simulated delay */ searchTimer = setTimeout(function () { S.searching = false; renderList(); }, 300 + 350);
      } else { S.searching = false; renderList(); }
    });
    $('#cls-tabs', d).addEventListener('keydown', function (e) {
      var keys = { ArrowRight: 1, ArrowLeft: -1, Home: -9, End: 9 };
      if (!(e.key in keys)) return;
      e.preventDefault();
      var ids = TABS.map(function (t) { return t[0]; }), i = ids.indexOf(S.tab), n = keys[e.key];
      i = n === -9 ? 0 : n === 9 ? ids.length - 1 : (i + n + ids.length) % ids.length;
      setTab(ids[i], true);
    });
    $('#cls-file', d).addEventListener('change', function (e) {
      var f = e.target.files && e.target.files[0];
      if (!f) return;
      if (!/\.wsz$/i.test(f.name)) { App.toast(f.name + ' isn\'t a .wsz skin', { kind: 'error', icon: 'warning' }); return; }
      applyLocal(f);
    });
  }

  /* keyboard: Esc and W leave Classic, Tab stays inside the stage (the app behind is inert) */
  function onKey(e) {
    if (!root) return;
    var overlayOpen = document.querySelector('body > .popover, body > .menu, #overlay-host > .is-open');
    if (overlayOpen) return;                                   // the app's own overlay owns Esc / Tab
    var a = e.target && e.target.nodeType === 1 ? e.target : document.activeElement;
    var typing = a && /^(INPUT|TEXTAREA)$/.test(a.tagName) && a.type !== 'file';
    if (e.key === 'Escape') {
      if (a && a.id === 'cls-search' && a.value) { e.preventDefault(); e.stopPropagation(); clearSearch(); return; }
      if (!isDocked() && drawerOpen() && root.contains(a) && $('#cls-drawer', root).contains(a)) {
        e.preventDefault(); e.stopPropagation(); setDrawer(false, true); return;
      }
      e.preventDefault(); e.stopPropagation(); exit(); return;
    }
    if ((e.key === 'w' || e.key === 'W') && !typing && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault(); e.stopPropagation(); exit(); return;
    }
    if (e.key === 'Tab') {
      var f = $$('a[href], button:not([disabled]), input:not([disabled]):not([hidden]), [tabindex]:not([tabindex="-1"])', root)
        .filter(function (n) { return n.offsetParent !== null && !n.closest('[inert]'); });
      if (!f.length) return;
      var i = f.indexOf(a);
      if (i < 0) { e.preventDefault(); f[0].focus(); return; }
      if (e.shiftKey && i === 0) { e.preventDefault(); f[f.length - 1].focus(); }
      else if (!e.shiftKey && i === f.length - 1) { e.preventDefault(); f[0].focus(); }
    }
  }

  function later(fn, ms) { var t = setTimeout(fn, ms); timers.push(t); return t; }

  /* ---------- states (prototype switcher) ---------- */
  function applyState(id) {
    if (!id || id.indexOf('classic-') !== 0) return;
    S = freshModel();
    S.skins = M.skins.map(function (s) { return Object.assign({}, s); });   // states ignore stored favourites
    S.applied = M.settings.appearance.defaultSkin; S.noticeHidden = false;
    switch (id) {
      case 'classic-drawer-closed': S.drawer = false; break;
      case 'classic-favourites': S.tab = 'favourites'; S.drawer = true; break;
      case 'classic-favourites-empty': S.tab = 'favourites'; S.drawer = true; S.skins.forEach(function (s) { s.favourite = false; }); break;
      case 'classic-downloaded': S.tab = 'downloaded'; S.drawer = true; break;
      case 'classic-search-empty': S.query = 'glitter'; S.drawer = true; break;
      case 'classic-loading': S.loading = true; S.drawer = true; break;
      case 'classic-preview': S.preview = { id: 'b72e01d4', prev: S.applied, left: 12 }; break;
      case 'classic-applying': S.busy = { id: 'd45b7e20', mode: 'apply', phase: 'download', pct: .5, frozen: true }; break;
      case 'classic-failed': S.failed = { id: '1be94d08', kept: S.applied }; delete S.failOnce['1be94d08']; break;
      case 'classic-offline': S.offline = true; S.drawer = true; break;
      case 'classic-nsfw': S.drawer = true; break;
      case 'classic-notice': S.drawer = false; break;
    }
  }

  App.registerStates('classic', [
    { id: 'classic-stage', label: 'Classic: stage with skin drawer', route: '/classic' },
    { id: 'classic-drawer-closed', label: 'Classic: stage, drawer closed', route: '/classic' },
    { id: 'classic-favourites', label: 'Classic: favourite skins', route: '/classic' },
    { id: 'classic-favourites-empty', label: 'Classic: no favourites yet', route: '/classic' },
    { id: 'classic-downloaded', label: 'Classic: downloaded skins', route: '/classic' },
    { id: 'classic-search-empty', label: 'Classic: skin search, no results', route: '/classic' },
    { id: 'classic-loading', label: 'Classic: loading skins', route: '/classic' },
    { id: 'classic-preview', label: 'Classic: preview with Keep / Revert', route: '/classic' },
    { id: 'classic-applying', label: 'Classic: downloading and applying a skin', route: '/classic' },
    { id: 'classic-failed', label: 'Classic: skin failed to load', route: '/classic' },
    { id: 'classic-offline', label: 'Classic: offline, downloaded skins only', route: '/classic' },
    { id: 'classic-nsfw', label: 'Classic: NSFW skins hidden note', route: '/classic' },
    { id: 'classic-notice', label: 'Classic: features-differ notice', route: '/classic' },
    { id: 'classic-drop', label: 'Classic: drop a .wsz file', route: '/classic' }
  ]);

  /* ---------- route ---------- */
  App.route('/classic', {
    title: 'Classic mode',
    render: function (ctx) {
      if (!root && Date.now() - closedAt > 400) restore = { scroll: ctx.el.scrollTop, opener: document.activeElement };
      /* PROTO: demo state */ if (ctx.state) applyState(ctx.state);
      return html`<div class="cls-behind" aria-hidden="true">${behindMarkup(ctx)}</div>`;
    },
    mount: function (ctx) {
      var quick = Date.now() - closedAt < 400;                // re-render (state switch): no second power-on
      if (restore) restore.path = lastPage ? lastPage.path : '/home';
      root = document.createElement('div');
      root.className = 's-sky cls-root' + (quick ? ' is-on is-quick' : '');
      root.setAttribute('role', 'dialog'); root.setAttribute('aria-modal', 'true'); root.setAttribute('aria-labelledby', 'cls-title');
      root.setAttribute('data-drawer', drawerOpen() ? 'open' : 'closed');
      root.innerHTML = String(stageMarkup());
      document.body.appendChild(root);
      document.documentElement.setAttribute('data-classic', 'on');
      $('#app').setAttribute('inert', ''); $('#app').setAttribute('aria-hidden', 'true');
      bindDrawer();
      if (!quick) requestAnimationFrame(function () { requestAnimationFrame(function () { if (root) root.classList.add('is-on'); }); });
      root.setAttribute('tabindex', '-1');                     // the dialog itself takes focus; Tab moves into it
      later(function () {
        var first = /* PROTO: demo state */ ctx.state === 'classic-preview' ? $('[data-act="cls-keep"]', root) : root;
        if (first) first.focus({ preventScroll: true });
      }, quick ? 0 : 60);

      document.addEventListener('keydown', onKey, true);
      var offPlayer = App.on('player', function (kind) { if (kind === 'track' && !S.busy) renderDesk(); });
      var lastDocked = isDocked();
      function onResize() {
        if (isDocked() === lastDocked) return;
        lastDocked = isDocked(); syncDrawer();
      }
      window.addEventListener('resize', onResize);

      if (S.preview) { clearInterval(previewTimer); previewTimer = setInterval(tick, 1000); }
      if (S.busy && !S.busy.frozen) { var b = S.busy; S.busy = null; load(b.id, b.mode); }
      var busyId = S.busy ? S.busy.id : S.failed ? S.failed.id : null;   // bring the card in question into view
      if (busyId) later(function () { var c = $('[data-skin="' + busyId + '"]', root); if (c) c.scrollIntoView({ block: 'nearest' }); }, 30);
      /* PROTO: simulated delay, demo state */ if (!S.listed && !ctx.state) {                          // first visit: the museum list takes a moment
        S.loading = true; renderList();
        later(function () { S.loading = false; renderList(); }, 700);
      }
      S.listed = true;
      /* PROTO: demo state */ if (ctx.state === 'classic-nsfw') later(function () { var n = $('#cls-nsfw', root); if (n) n.scrollIntoView({ block: 'end' }); }, 30);
      var drop = $('#drop-overlay');
      /* PROTO: demo state */ if (ctx.state === 'classic-drop') {
        $('.drop-overlay__label', drop).textContent = 'Drop .wsz to apply skin';
        drop.hidden = false; drop.classList.add('is-on');
      }
      if (pendingFile) { var f = pendingFile; pendingFile = null; applyLocal(f); }

      ctx.onCleanup(function () {
        document.removeEventListener('keydown', onKey, true);
        window.removeEventListener('resize', onResize);
        offPlayer();
        timers.forEach(clearTimeout); timers = [];
        clearInterval(previewTimer); clearTimeout(searchTimer); S.searching = false;
        S.busy = null; S.loading = false;               // a half-done download is dropped, not left spinning
        if (S.preview) endPreview(false, true);         // leaving mid-preview keeps the applied skin
        /* PROTO: demo state */ if (ctx.state === 'classic-drop') { drop.classList.remove('is-on'); drop.hidden = true; }
        App.closePopover();
        root.remove(); root = null; closedAt = Date.now();
        document.documentElement.removeAttribute('data-classic');
        $('#app').removeAttribute('inert'); $('#app').removeAttribute('aria-hidden');
      });
    }
  });
})();
