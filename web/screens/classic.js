/* screens/classic.js: Classic mode (/classic). Webamp plays the queue; nothing of the app shows behind it.
   - In the desktop app (LullDesktop.openClassic), Webamp gets its own borderless window, like Winamp itself, and the
     app's window hides (app/src/desktop/classic.rs, docs/context/desktop.md). This page hands over the queue, leaves
     /classic at once, records what the window reports hearing, and takes the queue back when Webamp closes.
   - In a browser, Webamp sits on a full-viewport stage on <body> over the inert, hidden app: glossy title pill with
     three orbs (Exit / Skins / Help), the Webamp slot, a skin drawer (Browse / Favourites / Downloaded) and a footer
     notice. design/DESIGN.md section 12 is the design; docs/context/web.md the data layer and the player engine.

   Webamp 2.3.1 (vendor/webamp/webamp.bundle.min.js, copied by scripts/vendor-web.sh) loads with a <script> tag the
   first time the stage opens, never on boot. One instance renders into #webamp-stage; later visits close() and
   reopen() it (dispose() still leaks), so the stage element itself is kept and moved into each new stage.
   - Hand-off (desktop/webamp-session.js, shared with the desktop window): on enter the app engine pauses and Webamp
     gets the queue (URLs from App.player.streamUrl) with title, artist, album and duration, plus the index, position,
     shuffle, repeat and volume. On exit (Webamp's close button, Esc, W or any route change) the playlist, current
     song, position, shuffle and repeat come back into App.player. While open, App.hooks.media points media keys at
     Webamp and songs heard there are recorded with App.recordPlay.
   - Skins (docs/context/skins.md): GET /api/skins?q=&offset=&nsfw= (the server's Winamp Skin Museum proxy, with
     favourite and downloaded flags), /api/skins/favourites, files from /api/skins/{md5}.wsz (validated, cached, same
     origin, immutable). A skin file is fetched here first, so a failure shows the server's sentence instead of
     Webamp's alert(); Webamp then parses the blob. A preview reverts after 15 s unless kept; a skin Webamp hasn't
     finished after 10 s counts as failed and the previous one comes back.
   - Downloaded = skins the server has cached (its flag), plus .wsz files opened this session; offline, the drawer
     lists favourites and downloaded skins from the last answers.
   - NSFW visibility and the default skin are per-device settings (Settings > Appearance writes them to
     App.storage 'showNsfwSkins' and 'defaultSkin'), read each time the stage opens. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, P = App.player, icon = App.icon, $ = App.$, $$ = App.$$;

  var BUNDLE = 'vendor/webamp/webamp.bundle.min.js';
  var SESSION = 'desktop/webamp-session.js';
  var PREVIEW_SECONDS = 15;
  var APPLY_TIMEOUT_MS = 10000;
  var SEARCH_DEBOUNCE_MS = 300;

  /* ---------- model (kept across visits, so leaving and coming back finds things as they were) ---------- */
  var saved = App.storage('classic') || {};
  var S = {
    tab: 'browse', query: '',
    drawer: null,                       // null = by width (docked open on desktop, closed below)
    noticeHidden: !!saved.noticeHidden,
    applied: saved.applied || null,     // the skin chosen here; else Settings > Appearance's default; else Webamp's base
    defaultSkin: null,
    local: [],                          // .wsz files opened this session: { md5: 'local:...', name, file, url, local }
    browse: { items: [], more: false, loading: false, error: null, key: null, want: null },  // key: the request items answer
    favs: { items: [], loading: false, error: null },
    showNsfw: false,
    offline: false,
    player: { state: 'idle', message: '' },  // Webamp: idle | loading | ready | failed (message says why)
    preview: null,                      // { skin, prev, left }
    busy: null,                         // { skin, mode: apply | preview | restore, phase: download | apply }
    failed: null                        // { skin, kept, message }
  };
  function persist() {
    App.storage('classic', { applied: S.applied && !S.applied.local ? S.applied : null, noticeHidden: S.noticeHidden });
  }
  function appliedSkin() { return S.applied || S.defaultSkin; }
  function nameOf(s) { return s ? s.name : 'Webamp\'s base skin'; }
  function sameSkin(a, b) { return !!a && !!b && a.md5 === b.md5; }
  function isDownloaded(s) { return !!s.local || !!s.downloaded; }
  function isFavourite(s) { return !!s.favourite; }
  /* every skin object known now (one md5 can appear in several lists), first copy of each */
  function knownSkins() {
    var seen = {};
    return [].concat(S.local, S.favs.items, S.browse.items, S.failed ? S.failed.skin : [], appliedSkin() || [])
      .filter(function (s) { return !seen[s.md5] && (seen[s.md5] = true); });
  }
  function findSkin(md5) { return knownSkins().filter(function (s) { return s.md5 === md5; })[0]; }
  /* the server's flags live on each copy; a change here updates them all */
  function setFlag(md5, key, value) {
    [].concat(S.favs.items, S.browse.items, S.applied || [], S.defaultSkin || [])
      .forEach(function (s) { if (s.md5 === md5) s[key] = value; });
  }
  function skinFileUrl(md5) { return M.url('/api/skins/' + encodeURIComponent(md5) + '.wsz'); }
  function isWsz(file) { return /\.wsz$/i.test(file.name); }
  function isDocked() { return window.innerWidth >= 1100; }
  function drawerOpen() { return S.drawer === null ? isDocked() : S.drawer; }
  function onClassic() { return /^#\/classic(?:[/?]|$)/.test(location.hash); }

  /* museum skins from the server, checked at the boundary: an md5 id, and only https links reach an href */
  function museumSkins(list) {
    var out = [];
    (list || []).forEach(function (s) {
      if (!s || !/^[0-9a-f]{32}$/i.test(s.md5)) { console.error('Skipping a skin without an md5 id', s); return; }
      out.push({ md5: s.md5, name: String(s.name || s.md5), screenshotUrl: String(s.screenshotUrl || ''),
        museumUrl: /^https:\/\//.test(s.museumUrl) ? s.museumUrl : '', nsfw: !!s.nsfw,
        favourite: !!s.favourite, downloaded: !!s.downloaded });
    });
    return out;
  }

  /* ---------- the stage element (lives on <body>, above the inert app) ---------- */
  var root = null, stage = null, lastPage = null, restore = null, timers = [], visit = 0;

  /* remember the page Classic was opened from: Exit returns to it */
  App.on('route', function (cur) {
    if (cur.path === '/classic') return;
    lastPage = { path: cur.path, params: cur.params, query: cur.query, route: cur.route };
    if (restore && restore.path === cur.path) {         // exit: scroll position and focus come back too
      var r = restore; restore = null;
      $('#view').scrollTop = r.scroll;
      if (r.opener && document.contains(r.opener) && r.opener.offsetParent !== null) r.opener.focus({ preventScroll: true });
    } else restore = null;
  });

  function lastHref() {
    if (!lastPage) return '/home';
    var q = Object.keys(lastPage.query)
      .map(function (k) { return encodeURIComponent(k) + '=' + encodeURIComponent(lastPage.query[k]); }).join('&');
    return lastPage.path + (q ? '?' + q : '');
  }

  /* #webamp-stage: Webamp's renderInto host, owned by React once rendered, so nothing else goes inside it */
  function makeStage() {
    var el = document.createElement('div');
    el.className = 'cls-slot'; el.id = 'webamp-stage';
    el.setAttribute('data-webamp', ''); el.setAttribute('role', 'region'); el.setAttribute('aria-label', 'Webamp');
    return el;
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
        <div class="cls-desk" id="cls-desk"><div class="cls-desk__status" id="cls-status">${statusMarkup()}</div></div>
        <footer class="cls-foot" id="cls-foot">${footMarkup()}</footer>
        <aside class="cls-drawer win s-panel" id="cls-drawer" aria-label="Skins" ${drawerOpen() ? '' : raw('inert')}>${drawerMarkup()}</aside>
      </div>
      <i class="cls-crt-line" aria-hidden="true"></i>`;
  }

  /* what sits over the stage: Webamp starting or failed, a skin loading or failed, the preview bar */
  function statusMarkup() {
    if (S.player.state === 'loading') return busyBox('Starting Webamp', 'Loading the player, only the first time classic mode opens', true);
    if (S.player.state === 'failed') {
      return html`<div class="cls-failed banner banner--down s-card" role="alert">
        ${icon('warning')}<span class="banner__msg">Webamp didn't start: ${S.player.message}</span>
        <button class="btn btn--sm" data-act="cls-start">Try again</button>
      </div>`;
    }
    return html`${S.busy ? busyMarkup() : ''}${S.failed ? failedMarkup() : ''}${S.preview ? previewMarkup() : ''}`;
  }
  function busyBox(title, text, xfer) {
    return html`<div class="cls-busy s-card" role="status">
      <span class="spin-ring" aria-hidden="true"></span>
      <div class="cls-busy__text">
        <strong>${title}</strong>
        <span class="fg-2">${text}</span>
        <span class="progress${xfer ? ' progress--xfer' : ''}" aria-hidden="true"><i style="--p:1"></i></span>
      </div>
    </div>`;
  }
  function busyMarkup() {
    var s = S.busy.skin, dl = S.busy.phase === 'download';
    return busyBox((dl ? 'Downloading ' : 'Applying ') + s.name,
      dl ? 'Fetching the skin from the museum and caching it' : 'Waiting for Webamp to load the skin', dl);
  }
  function failedMarkup() {
    var f = S.failed;
    return html`<div class="cls-failed banner banner--down s-card" role="alert">
      ${icon('warning')}<span class="banner__msg">Couldn't load ${f.skin.name}: ${f.message} ${nameOf(f.kept)} stays on.</span>
      <button class="btn btn--sm" data-act="cls-retry" data-id="${f.skin.md5}">Try again</button>
      <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-dismiss-failed" aria-label="Dismiss" data-tip="Dismiss">${icon('close', null, 'ic-16')}</button>
    </div>`;
  }
  function previewMarkup() {
    var left = S.preview.left;
    return html`<div class="cls-preview s-card" role="group" aria-label="Skin preview">
      <div class="cls-preview__text">
        <strong>Previewing ${S.preview.skin.name}</strong>
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
    var ph = S.tab === 'favourites' ? 'Search favourites'
      : S.tab === 'downloaded' || S.offline ? 'Search downloaded skins' : 'Search the Winamp Skin Museum';
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

  /* Browse is the server's answer for the query (offline: favourites and downloaded skins); the other tabs filter
     by name here */
  function visibleSkins() {
    var museum = S.tab === 'browse' && !S.offline;
    var list = museum ? S.browse.items : S.tab === 'favourites' ? S.favs.items
      : knownSkins().filter(function (s) { return isDownloaded(s) || (S.tab === 'browse' && isFavourite(s)); });
    var q = museum ? '' : S.query.trim().toLowerCase();
    return list.filter(function (s) {
      if (s.nsfw && !S.showNsfw) return false;
      return !q || s.name.toLowerCase().indexOf(q) >= 0;
    });
  }
  function listMarkup() {
    var notes = S.offline ? html`<div class="banner banner--warn s-card cls-note">${icon('offline')}<span class="banner__msg">You're offline, so only favourite and downloaded skins are listed.</span></div>` : '';
    var museum = S.tab === 'browse' && !S.offline, b = S.browse, src = museum ? b : S.tab === 'favourites' ? S.favs : null;
    if (src && src.error && !src.items.length) {
      return html`${notes}${ui.emptyState({ art: 'star', title: museum ? 'Couldn\'t load skins' : 'Couldn\'t load your favourites',
        text: src.error.message, action: { label: 'Try again', act: 'cls-reload', id: S.tab } })}`;
    }
    if (src && src.loading && !src.items.length) {
      if (museum && S.query.trim()) return html`${notes}<p class="cls-meta" role="status"><span class="spin-ring" aria-hidden="true"></span>Searching the museum</p>`;
      return html`${notes}<p class="vh" role="status">Loading skins</p><ul class="cls-grid" aria-hidden="true">${[0, 1, 2, 3].map(function () {
        return raw('<li class="cls-card cls-card--skel"><span class="skel cls-skel-shot"></span><span class="skel skel--title" style="width:70%"></span><span class="skel skel--text" style="width:45%"></span></li>');
      })}</ul>`;
    }
    var list = visibleSkins();
    if (!list.length) return html`${notes}${emptyMarkup()}`;
    var n = list.length;
    var meta = !museum ? n + (n === 1 ? ' skin' : ' skins')
      : S.query.trim() ? n + (b.more ? '+' : '') + (n === 1 && !b.more ? ' skin matches' : ' skins match') + ' "' + S.query.trim() + '"'
      : 'Most loved first, ' + n + ' shown';
    return html`${notes}<p class="cls-meta" role="status">${meta}</p>
      <ul class="cls-grid">${list.map(cardMarkup)}</ul>
      ${museum ? moreMarkup() : ''}
      ${museum && !S.showNsfw ? html`<p class="cls-nsfw" id="cls-nsfw">${icon('lock', null, 'ic-16')}<span>Skins marked NSFW are hidden. <a href="#/settings/appearance">Show them in Settings</a></span></p>` : ''}`;
  }
  /* the museum list pages 30 at a time: the next page on request, or why it didn't come */
  function moreMarkup() {
    var b = S.browse;
    if (b.error) {
      return html`<div class="banner banner--down s-card cls-note">${icon('warning')}<span class="banner__msg">Couldn't load more skins: ${b.error.message}</span>
        <button class="btn btn--sm" data-act="cls-more">Try again</button></div>`;
    }
    if (!b.more) return '';
    return html`<div class="cls-more"><button class="btn btn--sm" data-act="cls-more" ${b.loading ? raw('aria-disabled="true"') : ''}>${b.loading ? html`<span class="spin-ring" aria-hidden="true"></span>Loading more` : 'Show more skins'}</button></div>`;
  }
  function emptyMarkup() {
    var q = S.query.trim();
    if (q) return ui.emptyState({ art: 'star', title: 'No skins match "' + q + '"', text: 'Try a shorter word, or a colour such as purple or mint.', action: { label: 'Clear search', act: 'cls-clear' } });
    if (S.tab === 'favourites') return ui.emptyState({ art: 'star', title: 'No favourite skins yet', text: 'Star a skin to keep it here. Favourites also appear in Webamp\'s own skins menu.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
    if (S.tab === 'browse' && !S.offline) return ui.emptyState({ art: 'star', title: 'No skins to show', text: 'The Winamp Skin Museum sent an empty list. Try again in a moment.', action: { label: 'Try again', act: 'cls-reload', id: 'browse' } });
    return ui.emptyState({ art: 'slot', title: 'Nothing downloaded yet', text: 'Skins you preview or apply are cached here, so they work offline.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
  }
  function shotMarkup(s) {
    if (!s.screenshotUrl) return html`<span class="cls-shot cls-shot--blank" aria-hidden="true">${icon('classic')}</span>`;
    return html`<span class="cls-shot" aria-hidden="true"><img src="${s.screenshotUrl}" width="275" height="348" alt="" loading="lazy" decoding="async"></span>`;
  }
  function cardMarkup(s) {
    var applied = sameSkin(appliedSkin(), s) && !S.preview, previewing = S.preview && sameSkin(S.preview.skin, s);
    var busy = S.busy && sameSkin(S.busy.skin, s), failed = S.failed && sameSkin(S.failed.skin, s);
    var status = busy ? html`<span class="spin-ring" aria-hidden="true"></span>${S.busy.phase === 'download' ? 'Downloading' : 'Applying'}`
      : failed ? html`${icon('warning', null, 'ic-16')}Couldn't load`
      : previewing ? html`${icon('sparkle', null, 'ic-16')}Previewing`
      : applied ? html`${icon('check', null, 'ic-16')}Applied`
      : s.local ? html`${icon('import', null, 'ic-16')}From this device`
      : isDownloaded(s) ? html`${icon('saved', null, 'ic-16')}Downloaded`
      : html`${icon('external', null, 'ic-16')}On the museum`;
    var blocked = busy || !webamp || (S.offline && !isDownloaded(s));
    return html`<li class="cls-card s-card${applied ? ' is-applied' : ''}${previewing ? ' is-previewing' : ''}${failed ? ' is-failed' : ''}" data-skin="${s.md5}">
      <button class="cls-card__shot" data-act="cls-preview" data-id="${s.md5}" data-tip="Preview" aria-label="${'Preview ' + s.name}" ${blocked ? raw('aria-disabled="true"') : ''}>${shotMarkup(s)}</button>
      <div class="cls-card__text">
        <span class="cls-card__name ellipsis">${s.name}</span>
        ${s.local ? html`<span class="cls-card__file ellipsis">${s.file}</span>` : ''}
        <span class="cls-card__status${failed ? ' is-bad' : ''}">${status}</span>
      </div>
      <div class="cls-card__acts">
        ${applied ? html`<button class="btn btn--sm" aria-disabled="true" data-act="cls-applied" data-id="${s.md5}">Applied</button>`
          : failed ? html`<button class="btn btn--sm" data-act="cls-retry" data-id="${s.md5}">Retry</button>`
          : html`<button class="btn btn--sm" data-act="cls-apply" data-id="${s.md5}" ${blocked ? raw('aria-disabled="true"') : ''}>Apply</button>`}
        <span class="spacer"></span>
        ${s.local ? '' : html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-fav" data-id="${s.md5}" aria-pressed="${isFavourite(s) ? 'true' : 'false'}" aria-label="${'Favourite ' + s.name}" data-tip="${isFavourite(s) ? 'Remove from favourites' : 'Add to favourites'}">${icon(isFavourite(s) ? 'star-fill' : 'star', null, 'ic-16')}</button>`}
        ${s.museumUrl ? html`<a class="btn-icon btn-icon--bare btn-icon--sm" href="${s.museumUrl}" target="_blank" rel="noopener" aria-label="${'View ' + s.name + ' on the Winamp Skin Museum'}" data-tip="View on the Winamp Skin Museum">${icon('external', null, 'ic-16')}</a>` : ''}
      </div>
    </li>`;
  }

  /* ---------- partial re-renders (focus stays where it was) ---------- */
  function focusKey() {
    var a = document.activeElement;
    if (!root || !a || !root.contains(a) || stage.contains(a)) return null;
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
  function renderStatus() {
    if (!root) return;
    var k = focusKey();
    $('#cls-status', root).innerHTML = String(statusMarkup());
    if (k) refocus(k);
  }
  function renderDrawer() {
    if (!root) return;
    var k = focusKey();
    $('#cls-drawer', root).innerHTML = String(drawerMarkup());
    bindDrawer();
    if (k) refocus(k);
  }
  function renderFoot() { if (root) $('#cls-foot', root).innerHTML = String(footMarkup()); }
  function renderAll() { renderStatus(); renderList(); }

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

  /* ---------- server reads ---------- */
  var browseSeq = 0, browseTimer = 0;
  /* the Browse request for the current query and NSFW setting (nsfw=true only when this device shows them) */
  function browseKey() { return 'q=' + encodeURIComponent(S.query.trim()) + (S.showNsfw ? '&nsfw=true' : ''); }
  /* the Browse list for the current request; more appends the next page. delay debounces typing and paging */
  function loadBrowse(more, delay) {
    clearTimeout(browseTimer);
    var b = S.browse, q = browseKey(), seq = ++browseSeq;
    if (!more && b.key !== q) { b.items = []; b.more = false; }
    b.loading = true; b.error = null; b.want = q;
    renderList();
    browseTimer = setTimeout(function () {
      M.api('/api/skins?' + q + '&offset=' + (more ? b.items.length : 0)).then(function (res) {
        if (seq !== browseSeq) return;
        var page = museumSkins(res && res.items);
        b.items = more ? b.items.concat(page) : page; b.more = !!(res && res.more); b.key = q;
        b.loading = false; S.offline = false;
        renderList();
      }, function (err) {
        if (seq !== browseSeq) return;
        b.loading = false; b.error = err;
        var wasOffline = S.offline;
        S.offline = err.status === 0 || !navigator.onLine;
        if (S.offline !== wasOffline) renderDrawer(); else renderList();
      });
    }, delay || 0);
  }
  /* load Browse when what it shows doesn't answer the current request (and isn't on its way) */
  function ensureBrowse() {
    var b = S.browse, q = browseKey();
    if ((b.key === q && !b.error) || (b.loading && b.want === q)) return;
    loadBrowse(false);
  }
  function loadFavourites() {
    S.favs.loading = true; S.favs.error = null; renderList();
    return M.api('/api/skins/favourites').then(function (res) {
      S.favs.items = museumSkins(res && res.items);
    }, function (err) { S.favs.error = err; }).then(function () { S.favs.loading = false; renderList(); });
  }
  /* NSFW visibility and the default skin: per device, written by Settings > Appearance */
  function readDevicePrefs() {
    var md5 = App.storage('defaultSkin');
    S.showNsfw = App.storage('showNsfwSkins') === true;
    S.defaultSkin = typeof md5 === 'string' && md5
      ? findSkin(md5) || { md5: md5, name: 'your default skin', screenshotUrl: '', museumUrl: '', nsfw: false, favourite: false, downloaded: false }
      : null;
  }
  /* favourites once per session (retry from the drawer); settles either way, its error shows in the drawer */
  var favsRead = null;
  function readOnce() {
    if (!favsRead) favsRead = loadFavourites();
    return favsRead;
  }

  /* ---------- Webamp: load, create once, reopen ---------- */
  var webamp = null, session = null, webampClosed = false, closingByUs = false, prepared = null, creating = null;
  function loadScript(src, global, missing) {
    if (window[global]) return Promise.resolve(window[global]);
    return new Promise(function (resolve, reject) {
      var s = document.createElement('script');
      s.src = src;
      s.onload = function () {
        if (window[global]) resolve(window[global]);
        else reject(new Error(src + ' loaded but defined no ' + global + '.'));
      };
      s.onerror = function () { s.remove(); reject(new Error(missing)); };
      document.head.appendChild(s);
    });
  }
  /* the bundle, the shared hand-off, and what the constructor needs (favourites for its skins menu); a failed load
     can be retried */
  function prepare() {
    if (!prepared) {
      prepared = Promise.all([
        loadScript(BUNDLE, 'Webamp', 'the player file (' + BUNDLE + ') didn\'t load. Check your connection; a server built from source needs `mise run setup`.'),
        loadScript(SESSION, 'LullWebamp', SESSION + ' didn\'t load. Check your connection.'),
        readOnce()
      ]).then(function (r) { return r[0]; }, function (err) { prepared = null; throw err; });
    }
    return prepared;
  }
  function createWebamp(Webamp) {
    if (!Webamp.browserIsSupported()) throw new Error('this browser lacks Web Audio or canvas, which Webamp needs.');
    var phone = window.innerWidth < 640;                       // the equalizer closes so 275 px fits a phone
    var w = new Webamp({
      availableSkins: S.favs.items.map(function (s) { return { url: skinFileUrl(s.md5), name: s.name }; }),
      windowLayout: {
        main: { position: { top: 0, left: 0 } },
        equalizer: { position: { top: 116, left: 0 }, closed: phone },
        playlist: { position: { top: phone ? 116 : 232, left: 0 }, size: { extraWidth: 0, extraHeight: 0 } }
      },
      enableHotkeys: false,                                     // lull owns the shortcuts ([data-webamp] silences them)
      enableMediaSession: false,                                // and Media Session
      /* the stage is a stacking context, so this only lifts what Webamp puts on <body> (its context menu) over it */
      zIndex: parseInt(getComputedStyle(document.documentElement).getPropertyValue('--z-classic'), 10) + 1,
      handleTrackDropEvent: onWebampDrop,
      handleSaveListEvent: exportM3u,
      handleLoadListEvent: importM3u
    });
    w.onWillClose(function (cancel) { if (closingByUs) return; cancel(); exit(); });   // its close button = leave Classic
    var s = LullWebamp.session(w, { listen: App.recordPlay });
    return w.renderInto(stage).then(function () { webamp = w; session = s; }, function (err) {
      w.dispose();
      var fresh = makeStage();                                  // React may own the old host: start clean next time
      if (stage.parentNode) stage.parentNode.replaceChild(fresh, stage);
      stage = fresh;
      throw err;
    });
  }
  /* every visit: make sure Webamp is up, then hand it the queue. A visit that ended meanwhile does nothing */
  function startWebamp() {
    var mine = visit;
    function current() { return mine === visit && root; }
    if (!webamp) { S.player = { state: 'loading', message: '' }; renderStatus(); }
    prepare().then(function (Webamp) {
      if (webamp || !current()) return null;
      if (!creating) {
        creating = createWebamp(Webamp).then(function () { creating = null; }, function (err) { creating = null; throw err; });
      }
      return creating;
    }).then(function () {
      if (current() && webamp) webampReady();
    }, function (err) {
      if (!current()) return;
      S.player = { state: 'failed', message: err && err.message || String(err) };
      renderAll();
    });
  }
  var skinRestored = false, pendingFile = null;
  function webampReady() {
    S.player = { state: 'ready', message: '' };
    if (webampClosed) { webamp.reopen(); webampClosed = false; }
    handOver();
    App.hooks.media = webampControls;                          // media keys and the lock screen drive Webamp now
    renderAll();
    if (!skinRestored && appliedSkin()) { skinRestored = true; load(appliedSkin(), 'restore'); }
    if (pendingFile) { var f = pendingFile; pendingFile = null; applyLocal(f); }
  }

  /* ---------- hand-off between the app engine and Webamp (desktop/webamp-session.js has its shape) ---------- */
  var handedOver = false;
  function absolute(url) { return new URL(url, location.href).href; }
  function sameOrigin(url) { return new URL(url).origin === location.origin; }
  /* a queue entry as a hand-off track; duration spares Webamp fetching each file's head, art feeds the OS overlay */
  function handoffTrack(trackId, source) {
    var t = M.track(trackId), url = t && P.streamUrl(t.id, source);
    if (!url || !sameOrigin(absolute(url))) return null;
    var art = t.albumId ? absolute(App.artUrl(t.albumId, 256)) : null;
    return { id: t.id, url: absolute(url), title: t.title || '', artist: t.artist || '', album: t.album || '',
      duration: t.duration || null, art: art && sameOrigin(art) ? art : null };
  }
  function handoff() {
    var ps = P.state, index = 0, tracks = [];
    ps.queue.forEach(function (q, i) {
      var t = handoffTrack(q.trackId, q.source);
      if (!t) return;
      if (i === ps.index) index = tracks.length;
      tracks.push(t);
    });
    return { tracks: tracks, index: index, pos: ps.pos || 0, playing: !!ps.playing, shuffle: !!ps.shuffle,
      repeat: ps.repeat !== 'off', volume: ps.muted ? 0 : ps.volume };
  }
  function handOver() {
    var h = handoff();
    if (h.playing) P.pause();
    session.handIn(h);
    handedOver = true;
  }
  /* Webamp's list, song, position and modes back into the app engine (b: session.handBack()). Songs Webamp played
     from local files have no place in a lull queue and are left out */
  function takeBack(b) {
    syncModes(b.shuffle, b.repeat);
    if (b.local) App.toast(b.local === 1 ? 'The local file you added in Webamp isn\'t in your queue' : 'The ' + b.local + ' local files you added in Webamp aren\'t in your queue', { icon: 'info' });
    if (!b.ids.length) return;                                  // nothing of lull's left in Webamp: the queue stays
    var ps = P.state, same = b.ids.length === ps.queue.length && b.ids.every(function (id, i) { return ps.queue[i].trackId === id; });
    if (!same) P.playList(b.ids, b.index, { at: b.pos, autoplay: b.playing });
    else if (b.index !== ps.index) P.jumpTo(b.index, { at: b.pos, autoplay: b.playing });   // keeps radio and queue groups
    else {
      if (Math.abs(b.pos - ps.pos) > 1) P.seek(b.pos);
      if (b.playing) P.play();
    }
  }
  /* App.hooks.media while Classic is open */
  var webampControls = {
    play: function () { webamp.play(); }, pause: function () { webamp.pause(); },
    prev: function () { webamp.previousTrack(); }, next: function () { webamp.nextTrack(); },
    seek: function (sec) { webamp.seekToTime(sec); },
    seekBy: function (sec) { if (sec < 0) webamp.seekBackward(-sec); else webamp.seekForward(sec); }
  };

  /* Webamp's repeat is on or off: on keeps the app's all / one, off turns it off */
  function syncModes(shuffle, repeat) {
    var ps = P.state;
    if (shuffle !== ps.shuffle) P.setShuffle(shuffle);
    if (repeat && ps.repeat === 'off') P.cycleRepeat();
    while (!repeat && ps.repeat !== 'off') P.cycleRepeat();
  }

  /* files dropped on Webamp's windows: a .wsz applies here; lull songs dragged from another lull window join the list
     (and the queue on exit); anything else (audio files) Webamp plays by itself */
  function onWebampDrop(e) {
    var dt = e.dataTransfer, skinFile = Array.prototype.filter.call(dt.files || [], isWsz)[0];
    if (skinFile) { takeSkinFile(skinFile); return []; }
    var tracks = (dt.getData('text/plain') || '').split(',').map(function (id) { return handoffTrack(id.trim(), null); })
      .filter(Boolean).map(session.add);
    return tracks.length ? tracks : null;
  }
  /* Webamp's playlist menu. Save list: the lull songs in it as an M3U download. Load list: the library's M3U import,
     which matches each line and makes a lull playlist (Webamp's list stays as it is) */
  function exportM3u(tracks) {
    var lines = ['#EXTM3U'];
    tracks.forEach(function (wt) {
      var url = wt.url && absolute(wt.url), id = url && session.idOf(url), t = id && M.track(id);
      if (t) lines.push('#EXTINF:' + Math.round(t.duration || -1) + ',' + t.artist + ' - ' + t.title, url);
    });
    if (lines.length === 1) { App.toast('Nothing to save: the list holds no lull songs', { icon: 'info' }); return null; }
    var a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([lines.join('\n') + '\n'], { type: 'audio/x-mpegurl' }));
    a.download = 'lull-queue.m3u8';
    a.click();
    setTimeout(function () { URL.revokeObjectURL(a.href); }, 1000);   // the download has started by then
    App.toast('Saved the list as lull-queue.m3u8', { icon: 'check', kind: 'ok' });
    return null;
  }
  function importM3u() {
    var input = document.createElement('input');
    input.type = 'file'; input.accept = '.m3u,.m3u8,audio/x-mpegurl'; input.hidden = true;
    input.addEventListener('change', function () { if (input.files[0]) libraryImport(input.files[0]); input.remove(); });
    document.body.appendChild(input); input.click();
    return null;
  }

  /* ---------- skins in Webamp: fetch, apply, preview, restore ---------- */
  /* a URL Webamp can parse: the .wsz fetched from the server (its sentence on failure), or the local file */
  function fetchSkin(skin) {
    if (skin.local) return Promise.resolve(skin.url);
    /* the server serves .wsz as immutable (named by md5), so any cached copy will do, also offline */
    return fetch(skinFileUrl(skin.md5), { credentials: 'same-origin', cache: 'force-cache' }).then(function (res) {
      if (res.ok) return res.blob().then(function (b) { return URL.createObjectURL(b); });
      if (res.status === 401) M.onUnauthorized();
      return res.json().then(function (e) { return e && e.message; }, function () { return null; }).then(function (msg) {
        throw new Error(msg || 'the server answered ' + res.status + '.');
      });
    }, function () { throw new Error('can\'t reach the lull server.'); });
  }
  function applyUrl(url) {
    webamp.setSkinFromUrl(url);
    return new Promise(function (resolve, reject) {
      var t = setTimeout(function () { reject(new Error('Webamp didn\'t finish loading it within 10 seconds.')); }, APPLY_TIMEOUT_MS);
      webamp.skinIsLoaded().then(function () { clearTimeout(t); resolve(); }, function (e) { clearTimeout(t); reject(e); });
    });
  }
  /* show a skin without the busy states (revert, restore after a failure); null = Webamp's base skin */
  function showSkin(skin) {
    /* Webamp has no public call for its base skin: this is the action its own Skins > <Base Skin> menu sends (2.3.1) */
    if (!skin) webamp.store.dispatch({ type: 'LOAD_DEFAULT_SKIN' });
    else webamp.setSkinFromUrl(skin.local ? skin.url : skinFileUrl(skin.md5));
  }
  /* mode: apply | preview | restore (the applied skin on Webamp's first start: no toast) */
  function load(skin, mode) {
    if (!webamp) { App.toast(S.player.state === 'failed' ? 'Webamp didn\'t start, so skins can\'t load' : 'Webamp is still starting', { icon: 'clock' }); return; }
    if (S.busy) { App.toast('Wait for ' + S.busy.skin.name + ' to finish loading', { icon: 'clock' }); return; }
    if (S.offline && !isDownloaded(skin)) { App.toast('You\'re offline. ' + skin.name + ' isn\'t downloaded yet.', { icon: 'offline' }); return; }
    if (S.preview && mode === 'apply' && sameSkin(S.preview.skin, skin)) { endPreview(true); return; }
    var kept = mode === 'restore' ? null : S.preview ? S.preview.prev : appliedSkin();
    var replacesPreview = !!S.preview;
    if (replacesPreview) { clearInterval(previewTimer); S.preview = null; }
    var job = S.busy = { skin: skin, mode: mode, phase: isDownloaded(skin) ? 'apply' : 'download' }, objectUrl = null;
    S.failed = null;
    if (mode !== 'restore') App.announce((job.phase === 'download' ? 'Downloading ' : 'Applying ') + skin.name);
    renderAll();
    fetchSkin(skin).then(function (url) {
      if (!skin.local) objectUrl = url;
      job.phase = 'apply'; renderAll();
      return applyUrl(url);
    }).then(function () {
      S.busy = null;
      if (!skin.local) { skin.downloaded = true; setFlag(skin.md5, 'downloaded', true); }   // the server cached it
      if (mode === 'preview') { if (root) startPreview(skin, kept); else showSkin(kept); }   // left mid-load: no preview
      else if (mode === 'apply') {
        S.applied = skin;
        App.toast(skin.name + ' is your classic skin', { icon: 'check', kind: 'ok' }); App.announce(skin.name + ' applied');
      }
      persist(); renderAll();
    }, function (err) {
      S.busy = null;
      S.failed = { skin: skin, kept: kept, message: err.message };
      if (job.phase === 'apply' || replacesPreview) showSkin(kept);   // Webamp may be half-way into it
      App.announce('Couldn\'t load ' + skin.name + '. ' + nameOf(kept) + ' stays on.');
      renderAll();
    }).then(function () { if (objectUrl) URL.revokeObjectURL(objectUrl); });
  }

  var previewTimer = 0;
  function startPreview(skin, prev) {
    S.preview = { skin: skin, prev: prev, left: PREVIEW_SECONDS };
    App.announce('Previewing ' + skin.name + '. Keep it or it reverts in ' + PREVIEW_SECONDS + ' seconds.');
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
    var p = S.preview, bar = root && $('.cls-preview', root), focusInBar = bar && bar.contains(document.activeElement);
    clearInterval(previewTimer); S.preview = null;
    if (kept) S.applied = p.skin; else showSkin(p.prev);
    persist();
    if (quiet) return;
    App.toast(kept ? p.skin.name + ' is your classic skin' : 'Back to ' + nameOf(p.prev),
      kept ? { icon: 'check', kind: 'ok' } : { icon: 'undo', action: 'Preview again', onAction: function () { load(p.skin, 'preview'); } });
    renderAll();
    if (focusInBar) refocus('[data-act="cls-preview"][data-id="' + p.skin.md5 + '"]', '#cls-search');
  }

  function wszName(file) {
    return file.replace(/\.wsz$/i, '').replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim()
      .replace(/\b\w/g, function (c) { return c.toUpperCase(); }) || 'Local skin';
  }
  /* a .wsz from this device: applied from memory (URL.createObjectURL), listed under Downloaded for the session */
  function applyLocal(file) {
    var key = 'local:' + file.name + ':' + file.size;
    var skin = S.local.filter(function (s) { return s.md5 === key; })[0];
    if (!skin) {
      skin = { md5: key, name: wszName(file.name), file: file.name, url: URL.createObjectURL(file), local: true, screenshotUrl: '', museumUrl: '', nsfw: false };
      S.local.unshift(skin);
    }
    S.tab = 'downloaded'; S.query = '';
    if (root) { renderDrawer(); setDrawer(true); }
    load(skin, 'apply');
  }
  /* applies now, or as soon as Webamp is up */
  function takeSkinFile(file) {
    if (webamp && root) applyLocal(file); else pendingFile = file;
  }

  /* .wsz files dropped on the window open Classic and apply; everything else stays the library's (M3U import) */
  var libraryImport = App.hooks.importFile;
  App.hooks.importFile = function (file) {
    if (!isWsz(file)) { libraryImport(file); return; }
    takeSkinFile(file);
    if (!onClassic()) App.go('/classic');
  };

  /* ---------- actions ---------- */
  function exit() {
    if (!root || root.classList.contains('is-leaving')) return;
    root.classList.add('is-leaving');
    later(function () { App.go(lastHref()); }, App.reducedMotion() ? 130 : 200);
  }
  App.action('cls-exit', exit);
  App.action('cls-drawer', function (el) { setDrawer(!drawerOpen(), true, el); });
  App.action('cls-help', function (el) {
    App.popover(el, html`<div class="popover__head">${icon('classic')}<span>classic mode</span></div>
      <div class="popover__body cls-help">
        <p>Webamp plays your queue here with its own skins. Drag its windows anywhere on the stage, and drop a .wsz file anywhere to apply it.</p>
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
  App.action('cls-start', startWebamp);
  App.action('cls-tab', function (el) { setTab(el.getAttribute('data-id'), true); });
  function clearSearch() {
    S.query = '';
    var i = root && $('#cls-search', root); if (i) { i.value = ''; i.focus(); }
    syncClear();
    if (S.tab === 'browse' && !S.offline) ensureBrowse(); else renderList();
  }
  App.action('cls-clear', clearSearch);
  App.action('cls-open-file', function () { var f = root && $('#cls-file', root); if (f) { f.value = ''; f.click(); } });
  App.action('cls-more', function () { if (!S.browse.loading) loadBrowse(true, SEARCH_DEBOUNCE_MS); });
  App.action('cls-reload', function (el) {
    if (el.getAttribute('data-id') === 'favourites') loadFavourites(); else loadBrowse(false);
  });
  App.action('cls-preview', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (!s) return;
    if (S.preview && sameSkin(S.preview.skin, s)) { App.toast('Already previewing ' + s.name, { icon: 'sparkle' }); return; }
    if (sameSkin(appliedSkin(), s) && !S.preview) { App.toast(s.name + ' is already on', { icon: 'check' }); return; }
    load(s, 'preview');
  });
  App.action('cls-apply', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (s && !(S.busy && sameSkin(S.busy.skin, s))) load(s, 'apply');
  });
  App.action('cls-applied', function (el) { var s = findSkin(el.getAttribute('data-id')); if (s) App.toast(s.name + ' is already your classic skin', { icon: 'check' }); });
  App.action('cls-retry', function (el) { var s = findSkin(el.getAttribute('data-id')); if (s) load(s, 'apply'); });
  App.action('cls-dismiss-failed', function () { S.failed = null; renderAll(); refocus(null, '#cls-search'); });
  App.action('cls-keep', function () { endPreview(true); });
  App.action('cls-revert', function () { endPreview(false); });
  App.action('cls-notice', function () {
    S.noticeHidden = true; persist(); renderFoot();
    refocus('.cls-exit .btn');
    App.toast('Notice hidden. Help keeps it.', { icon: 'info' });
  });
  /* favourites change at once (newest first, like the server's list) and roll back with its sentence if refused */
  function setFavourite(s, on) {
    var before = S.favs.items;
    S.favs.items = (on ? [s] : []).concat(before.filter(function (f) { return f.md5 !== s.md5; }));
    s.favourite = on; setFlag(s.md5, 'favourite', on);
    renderList();
    M.api('/api/skins/favourites/' + encodeURIComponent(s.md5), { method: on ? 'PUT' : 'DELETE' }).catch(function (err) {
      S.favs.items = before; s.favourite = !on; setFlag(s.md5, 'favourite', !on);
      renderList();
      App.toast(err.message, { kind: 'error', icon: 'warning' });
    });
  }
  App.action('cls-fav', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (!s || s.local) return;
    var on = !isFavourite(s);
    setFavourite(s, on);
    if (S.tab === 'favourites' && !on) {
      App.toast('Removed ' + s.name + ' from favourites', { icon: 'star', action: 'Undo', onAction: function () { setFavourite(s, true); } });
    } else App.announce(on ? s.name + ' added to favourites' : s.name + ' removed from favourites');
    if (!$('[data-act="cls-fav"][data-id="' + s.md5 + '"]', root)) refocus(null, '#cls-list');
  });

  function setTab(id, focus) {
    if (TABS.map(function (t) { return t[0]; }).indexOf(id) < 0) return;
    S.tab = id; S.query = '';
    renderDrawer();
    if (id === 'browse' && !S.offline) ensureBrowse();
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
      if (S.tab === 'browse' && !S.offline) loadBrowse(false, SEARCH_DEBOUNCE_MS); else renderList();
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
      if (!isWsz(f)) { App.toast(f.name + ' isn\'t a .wsz skin', { kind: 'error', icon: 'warning' }); return; }
      takeSkinFile(f);
    });
  }

  /* keyboard: Esc and W leave Classic (W not inside Webamp), Tab stays inside the stage (the app behind is inert) */
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
    if ((e.key === 'w' || e.key === 'W') && !typing && !e.ctrlKey && !e.metaKey && !e.altKey && !(a && a.closest && a.closest('[data-webamp]'))) {
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
  var lastDocked = isDocked();
  function onResize() {
    if (isDocked() === lastDocked) return;
    lastDocked = isDocked(); syncDrawer();
  }
  function onOnline() { if (S.offline) { S.offline = false; renderDrawer(); ensureBrowse(); } }

  function later(fn, ms) { var t = setTimeout(fn, ms); timers.push(t); return t; }

  /* ---------- open / close ---------- */
  function openStage() {
    visit += 1;
    if (restore) restore.path = lastPage ? lastPage.path : '/home';
    root = document.createElement('div');
    root.className = 's-sky cls-root';
    root.setAttribute('role', 'dialog'); root.setAttribute('aria-modal', 'true'); root.setAttribute('aria-labelledby', 'cls-title');
    root.setAttribute('data-drawer', drawerOpen() ? 'open' : 'closed');
    root.setAttribute('tabindex', '-1');                       // the dialog itself takes focus; Tab moves into it
    root.innerHTML = String(stageMarkup());
    if (!stage) stage = makeStage();
    $('#cls-desk', root).insertBefore(stage, $('#cls-status', root));
    document.body.appendChild(root);
    document.documentElement.setAttribute('data-classic', 'on');
    $('#app').setAttribute('inert', ''); $('#app').setAttribute('aria-hidden', 'true');
    bindDrawer();
    requestAnimationFrame(function () { requestAnimationFrame(function () { if (root) root.classList.add('is-on'); }); });
    later(function () { if (root) root.focus({ preventScroll: true }); }, 60);
    document.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', onResize);
    window.addEventListener('online', onOnline);
    lastDocked = isDocked();
    readDevicePrefs();
    readOnce();
    ensureBrowse();                                            // offline: finds out whether that is still so
    startWebamp();
  }
  function closeStage() {
    visit += 1;                                                // a Webamp start still in flight won't hand over now
    if (S.preview) endPreview(false, true);                    // leaving mid-preview keeps the applied skin
    if (webamp) {
      App.hooks.media = null;                                  // back to the app engine
      session.stop();
      if (handedOver) { handedOver = false; takeBack(session.handBack()); }
      closingByUs = true; webamp.close(); closingByUs = false;  // close() asks onWillClose too: let it through
      webampClosed = true;
    }
    document.removeEventListener('keydown', onKey, true);
    window.removeEventListener('resize', onResize);
    window.removeEventListener('online', onOnline);
    timers.forEach(clearTimeout); timers = [];
    App.closePopover();
    root.remove(); root = null;
    document.documentElement.removeAttribute('data-classic');
    $('#app').removeAttribute('inert'); $('#app').removeAttribute('aria-hidden');
  }

  /* ---------- the desktop app: Webamp in its own window (app/src/desktop/classic.rs) ---------- */
  /* desktop.js loads after this file: ask when Classic opens */
  function desktopApp() { var D = window.LullDesktop; return D && D.inWrapper && D.openClassic ? D : null; }
  var windowListen = null, hearing = false, windowWasPlaying = false;
  /* the hand-off plus the skin to start in and the favourites for Webamp's Skins menu */
  function openWindow(D) {
    if (!hearing) { hearing = true; D.onClassic(fromWindow); }
    readDevicePrefs();
    var skinOf = function (s) { return { url: absolute(skinFileUrl(s.md5)), name: s.name }; };
    var h = handoff(), skin = appliedSkin();
    return readOnce().then(function () {
      h.skin = skin && !skin.local ? skinOf(skin) : null;
      h.skins = S.favs.items.map(skinOf);
      return D.openClassic(h);
    }).then(function () {                                       // the window starts Webamp where the app was
      windowWasPlaying = h.playing;
      if (h.playing) P.pause();
    });
  }
  /* what Webamp's window reports (desktop.js passes on its 'classic' events) */
  function fromWindow(m) {
    if (m.type === 'listen') windowListen = App.recordPlay(m.id, m.duration);
    else if (m.type === 'listened' && windowListen) {
      windowListen.listened = m.seconds; windowListen.end(m.how); windowListen = null;
    } else if (m.type === 'back') {
      if (m.back) takeBack(m.back);
      else if (windowWasPlaying) P.play();                      // Webamp never started: carry on here
    }
    else if (m.type === 'failed') App.toast('Webamp didn\'t start: ' + m.message, { kind: 'error', icon: 'warning' });
  }

  /* ---------- route ---------- */
  App.route('/classic', {
    title: 'Classic mode',
    render: function (ctx) {
      if (!root) restore = { scroll: ctx.el.scrollTop, opener: document.activeElement };
      return '';
    },
    mount: function (ctx) {
      var D = desktopApp();
      if (D && !root) {
        /* Classic lives in its own window: back to the page it came from. An app without that window (older than
           this page) refuses, and the stage opens here instead */
        openWindow(D).then(function () { if (onClassic()) App.go(lastHref()); }, function (err) {
          console.warn('Classic window unavailable, using the stage', err);
          if (onClassic() && !root) openStage();
        });
      } else if (!root) openStage();
      /* a re-render of /classic keeps the stage; leaving closes it */
      ctx.onCleanup(function () { if (!onClassic() && root) closeStage(); });
    }
  });
})();
