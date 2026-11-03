/* screens/skins.js: Classic mode's skin and where it is chosen, Settings > Appearance > Classic (design/DESIGN.md
   section 12, docs/context/skins.md). Classic itself (screens/classic.js, desktop/winamp.js) is only Webamp.
   - The choice is per device: App.classicSkin.get() / .choose(skin | null), stored as localStorage 'lull:classicSkin'
     (desktop/webamp-session.js has its shape). A change reaches Classic at once: 'classic-skin' in this page, the
     storage event in other tabs and in the desktop app's Classic window.
   - App.classicSettings() is the section's markup (screens/appearance.js places it): the skin in use, a .wsz file
     from this device, the skin browser, the NSFW switch and what differs in Classic mode.
   - The browser: Browse is GET /api/skins?q=&offset=&nsfw= (the hub's Winamp Skin Museum proxy, 30 a page, search
     debounced); Favourites is /api/skins/favourites (PUT / DELETE one); Downloaded is what the hub has cached (its
     flag) plus the .wsz files kept on this device (LullWebamp.localSkins). Offline, Browse lists favourites and
     downloaded skins from the last answers. Choosing a museum skin downloads it through the hub first, so a skin
     that can't be had says why here, not later in Webamp.
   - .wsz files: "Open a .wsz file" and a .wsz dropped anywhere on lull are kept on this device (IndexedDB) and
     chosen. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$;
  var K = App.settingsKit, row = K.row, group = K.group, switchRow = K.switchRow;
  var L = window.LullWebamp;

  var SEARCH_DEBOUNCE_MS = 300;
  var MAX_FILE = 16 * 1024 * 1024;                      // the hub's cap for a .wsz
  var TABS = [['browse', 'Browse'], ['favourites', 'Favourites'], ['downloaded', 'Downloaded']];

  /* ---------- the choice ---------- */
  migrate();
  /* checked where it is read: a museum md5 or a local id, anything else counts as Webamp's base skin */
  function chosen() {
    var c = App.storage('classicSkin');
    return c && typeof c.md5 === 'string' && /^([0-9a-f]{32}|local-[0-9a-z]+)$/i.test(c.md5) ? c : null;
  }
  function choose(skin) {
    var value = skin ? { md5: skin.md5, name: skin.name, screenshotUrl: skin.screenshotUrl || '' } : null;
    App.storage('classicSkin', value);
    App.emit('classic-skin', value);
    renderChoice();
  }
  App.classicSkin = { get: chosen, choose: choose, keepFile: keepFile };
  /* another tab or window chose: this page's Classic and Settings follow */
  window.addEventListener('storage', function (e) {
    if (e.key !== L.SKIN_KEY) return;
    App.emit('classic-skin', chosen());
    renderChoice();
  });
  /* before Settings held it: 'defaultSkin' (an md5) and the skin last applied in Classic ('classic'.applied), which
     won when both were set */
  function migrate() {
    var old = App.storage('classic'), md5 = App.storage('defaultSkin'), a = old && old.applied;
    if (old === null && md5 === null) return;
    if (a && a.md5) App.storage('classicSkin', { md5: a.md5, name: a.name || '', screenshotUrl: a.screenshotUrl || '' });
    else if (typeof md5 === 'string' && md5) App.storage('classicSkin', { md5: md5, name: '', screenshotUrl: '' });
    try { localStorage.removeItem('lull:classic'); localStorage.removeItem('lull:defaultSkin'); } catch (e) { /* storage blocked */ }
  }
  function nameOf(s) { return s ? s.name || 'Your skin' : 'Webamp\'s base skin'; }
  function inUse(s) { var c = chosen(); return !!c && c.md5 === s.md5; }

  /* ---------- the browser's model (kept across visits) ---------- */
  var S = {
    tab: 'browse', query: '',
    local: [], localRead: false,          // .wsz files kept on this device: { md5: 'local-…', name, file, local }
    browse: { items: [], more: false, loading: false, error: null, key: null, want: null },  // key: the request items answer
    favs: { items: [], loading: false, error: null, read: false },
    offline: false,
    busy: null,                           // { skin }: downloading through the hub before it is chosen
    failed: null                          // { skin, message }
  };
  function showNsfw() { return App.storage('showNsfwSkins') === true; }
  function isDownloaded(s) { return !!s.local || !!s.downloaded; }
  function isFavourite(s) { return !!s.favourite; }
  function sameSkin(a, b) { return !!a && !!b && a.md5 === b.md5; }
  /* every skin object known now (one md5 can appear in several lists), first copy of each */
  function knownSkins() {
    var seen = {};
    return [].concat(S.local, S.favs.items, S.browse.items, S.failed ? S.failed.skin : [])
      .filter(function (s) { return !seen[s.md5] && (seen[s.md5] = true); });
  }
  function findSkin(md5) { return knownSkins().filter(function (s) { return s.md5 === md5; })[0]; }
  /* the hub's flags live on each copy; a change here updates them all */
  function setFlag(md5, key, value) {
    [].concat(S.favs.items, S.browse.items).forEach(function (s) { if (s.md5 === md5) s[key] = value; });
  }
  function skinFileUrl(md5) { return M.url('/api/skins/' + encodeURIComponent(md5) + '.wsz'); }
  function isWsz(file) { return /\.wsz$/i.test(file.name); }

  /* museum skins from the hub, checked at the boundary: an md5 id, and only https links reach an href */
  function museumSkins(list) {
    var out = [];
    (list || []).forEach(function (s) {
      if (!s || !/^[0-9a-f]{32}$/i.test(s.md5)) { console.error('Skipping a skin without an md5 id', s); return; }
      out.push({ md5: s.md5, name: String(s.name || s.md5), screenshotUrl: String(s.screenshotUrl || ''),
        museumUrl: /^https:\/\//.test(s.museumUrl) ? s.museumUrl : '', nsfw: !!s.nsfw,
        favourite: !!s.favourite, downloaded: !!s.downloaded });
    });
    learnName(out);
    return out;
  }
  /* a choice carried over without a name (the old default skin) takes it from the first list that has it */
  function learnName(list) {
    var c = chosen();
    if (!c || c.name) return;
    var s = list.filter(function (x) { return x.md5 === c.md5; })[0];
    if (s) { App.storage('classicSkin', { md5: c.md5, name: s.name, screenshotUrl: s.screenshotUrl }); renderChoice(); }
  }

  /* ---------- markup ---------- */
  App.classicSettings = function () {
    readOnce();
    return html`${group('classic mode')}
      <div class="sys-row sys-row--stack">
        <div class="sys-row__text"><span class="sys-row__label" id="cls-skin-l">Skin</span><span class="sys-row__help">Classic mode opens Webamp in this skin on this device, in the browser and in the app's Winamp window. If Classic is open, a new choice shows there at once.</span></div>
        <div class="sys-row__ctl" id="cls-choice">${choiceMarkup()}</div></div>
      <div class="sys-row sys-row--stack">
        <div class="sys-row__text"><span class="sys-row__label" id="cls-find-l">Find a skin</span><span class="sys-row__help">Browse and search the Winamp Skin Museum, or pick from your favourites and the skins already downloaded. Select a picture to see it larger.</span></div>
        <div class="sys-row__ctl cls-browser" id="cls-browser" role="group" aria-labelledby="cls-find-l">${browserMarkup()}</div></div>
      ${switchRow('device.showNsfwSkins', 'Show NSFW skins', 'The museum marks some skins NSFW. They stay hidden while you browse on this device unless this is on.')}
      ${row('Classic mode', 'Webamp plays your queue. Crossfade, loudness normalisation and gapless playback pause there, and repeat is on or off only. Esc, W or Webamp\'s close button brings you back.', html`<a class="btn btn--sm" href="#/classic">${icon('classic')}Open Classic mode</a>`)}`;
  };

  /* the skin in use, and the two ways to change it that aren't the browser */
  function choiceMarkup() {
    var c = chosen(), busyLocal = S.busy && S.busy.skin.local;
    return html`<div class="cls-now s-card">
        ${shotMarkup(c)}
        <div class="cls-now__text">
          <span class="cls-now__label">In use</span>
          <strong class="cls-now__name ellipsis">${nameOf(c)}</strong>
          <span class="cls-now__src">${!c ? 'Webamp\'s own look' : /^local-/.test(c.md5) ? 'A file kept on this device' : 'From the Winamp Skin Museum'}</span>
        </div>
      </div>
      <div class="cluster">
        <label class="btn btn--sm">${busyLocal ? html`<span class="spin-ring" aria-hidden="true"></span>` : icon('import', null, 'ic-16')}Open a .wsz file<input class="vh" type="file" accept=".wsz" data-skin-file></label>
        ${c ? html`<button class="btn btn--sm btn--quiet" data-act="cls-base">${icon('undo', null, 'ic-16')}Use Webamp's base skin</button>` : ''}
      </div>`;
  }

  function browserMarkup() {
    var i = TABS.map(function (t) { return t[0]; }).indexOf(S.tab);
    var ph = S.tab === 'favourites' ? 'Search favourites'
      : S.tab === 'downloaded' || S.offline ? 'Search downloaded skins' : 'Search the museum';
    return html`<div class="cls-browser__head">
        <div class="seg" role="tablist" aria-label="Skin lists" style="--n:3;--i:${i}" id="cls-tabs">
          <span class="seg__ind" aria-hidden="true"></span>
          ${TABS.map(function (t) {
            var on = t[0] === S.tab;
            return html`<button class="seg__btn" role="tab" id="cls-tab-${t[0]}" aria-controls="cls-list" aria-selected="${on ? 'true' : 'false'}" tabindex="${on ? '0' : '-1'}" data-act="cls-tab" data-id="${t[0]}">${t[1]}</button>`;
          })}
        </div>
        <div class="search-field" role="search">
          ${icon('search')}
          <label class="vh" for="cls-search">${ph === 'Search the museum' ? 'Search the Winamp Skin Museum' : ph}</label>
          <input class="input" id="cls-search" type="search" placeholder="${ph}" autocomplete="off" spellcheck="false" value="${S.query}">
          <span class="search-field__end">
            <button class="btn-icon btn-icon--bare btn-icon--sm search-field__clear" data-act="cls-clear" aria-label="Clear search" data-tip="Clear" ${S.query ? '' : raw('hidden')}>${icon('close', null, 'ic-16')}</button>
          </span>
        </div>
      </div>
      <div class="cls-browser__body" id="cls-list" role="tabpanel" aria-labelledby="cls-tab-${S.tab}" tabindex="-1">${listMarkup()}</div>
      <a class="cls-attrib" href="https://skins.webamp.org/" target="_blank" rel="noopener">Skins from the Winamp Skin Museum</a>`;
  }

  /* Browse is the hub's answer for the query (offline: favourites and downloaded skins); the other tabs filter by
     name here */
  function visibleSkins() {
    var museum = S.tab === 'browse' && !S.offline;
    var list = museum ? S.browse.items : S.tab === 'favourites' ? S.favs.items
      : knownSkins().filter(function (s) { return isDownloaded(s) || (S.tab === 'browse' && isFavourite(s)); });
    var q = museum ? '' : S.query.trim().toLowerCase(), nsfw = showNsfw();
    return list.filter(function (s) {
      if (s.nsfw && !nsfw) return false;
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
      ${museum && !showNsfw() ? html`<p class="cls-nsfw">${icon('lock', null, 'ic-16')}<span>Skins marked NSFW are hidden. The switch below shows them.</span></p>` : ''}`;
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
    if (S.tab === 'favourites') return ui.emptyState({ art: 'star', title: 'No favourite skins yet', text: 'Star a skin to keep it here. Favourites also appear in Webamp\'s own Skins menu.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
    if (S.tab === 'browse' && !S.offline) return ui.emptyState({ art: 'star', title: 'No skins to show', text: 'The Winamp Skin Museum sent an empty list. Try again in a moment.', action: { label: 'Try again', act: 'cls-reload', id: 'browse' } });
    return ui.emptyState({ art: 'slot', title: 'Nothing downloaded yet', text: 'Skins you choose are cached on your hub, and .wsz files you open stay on this device, so they work offline.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
  }
  function shotMarkup(s) {
    if (!s || !s.screenshotUrl) return html`<span class="cls-shot cls-shot--blank" aria-hidden="true">${icon('classic')}</span>`;
    return html`<span class="cls-shot" aria-hidden="true"><img src="${s.screenshotUrl}" width="275" height="348" alt="" loading="lazy" decoding="async"></span>`;
  }
  function cardMarkup(s) {
    var on = inUse(s), busy = S.busy && sameSkin(S.busy.skin, s), failed = S.failed && sameSkin(S.failed.skin, s);
    var status = busy ? html`<span class="spin-ring" aria-hidden="true"></span>Downloading`
      : failed ? html`${icon('warning', null, 'ic-16')}Couldn't download`
      : on ? html`${icon('check', null, 'ic-16')}In use`
      : s.local ? html`${icon('import', null, 'ic-16')}On this device`
      : isDownloaded(s) ? html`${icon('saved', null, 'ic-16')}Downloaded`
      : html`${icon('external', null, 'ic-16')}On the museum`;
    var blocked = busy || (S.offline && !isDownloaded(s));
    return html`<li class="cls-card s-card${on ? ' is-applied' : ''}${failed ? ' is-failed' : ''}" data-skin="${s.md5}">
      <button class="cls-card__shot" data-act="cls-look" data-id="${s.md5}" aria-haspopup="dialog" aria-expanded="false" aria-label="${'See ' + s.name + ' larger'}" data-tip="See it larger">${shotMarkup(s)}</button>
      <div class="cls-card__text">
        <span class="cls-card__name ellipsis">${s.name}</span>
        ${s.local ? html`<span class="cls-card__file ellipsis">${s.file}</span>` : ''}
        <span class="cls-card__status${failed ? ' is-bad' : ''}">${status}</span>
        ${failed ? html`<span class="cls-card__why">${S.failed.message}</span>` : ''}
      </div>
      <div class="cls-card__acts">
        ${useButton(s, on, failed, blocked)}
        <span class="spacer"></span>
        ${s.local ? html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-forget" data-id="${s.md5}" aria-label="${'Remove ' + s.name + ' from this device'}" data-tip="Remove from this device">${icon('trash', null, 'ic-16')}</button>`
          : html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-fav" data-id="${s.md5}" aria-pressed="${isFavourite(s) ? 'true' : 'false'}" aria-label="${'Favourite ' + s.name}" data-tip="${isFavourite(s) ? 'Remove from favourites' : 'Add to favourites'}">${icon(isFavourite(s) ? 'star-fill' : 'star', null, 'ic-16')}</button>`}
        ${s.museumUrl ? html`<a class="btn-icon btn-icon--bare btn-icon--sm" href="${s.museumUrl}" target="_blank" rel="noopener" aria-label="${'View ' + s.name + ' on the Winamp Skin Museum'}" data-tip="View on the Winamp Skin Museum">${icon('external', null, 'ic-16')}</a>` : ''}
      </div>
    </li>`;
  }
  function useButton(s, on, failed, blocked) {
    if (on) return html`<button class="btn btn--sm" aria-disabled="true" data-act="cls-in-use" data-id="${s.md5}" aria-label="${s.name + ' is in use'}">In use</button>`;
    if (failed) return html`<button class="btn btn--sm" data-act="cls-use" data-id="${s.md5}" aria-label="${'Try ' + s.name + ' again'}">Try again</button>`;
    return html`<button class="btn btn--sm" data-act="cls-use" data-id="${s.md5}" aria-label="${'Use ' + s.name}" ${blocked ? raw('aria-disabled="true"') : ''}>Use</button>`;
  }
  /* the screenshot at its own size (pixels doubled where there's room), with what the card offers */
  function lookMarkup(s) {
    var on = inUse(s);
    return html`<div class="popover__head">${icon('classic')}<span class="ellipsis">${s.name}</span></div>
      <div class="popover__body cls-look">${s.screenshotUrl
        ? html`<span class="cls-look__shot"><img src="${s.screenshotUrl}" width="275" height="348" alt="${'Screenshot of ' + s.name + ': the main window, equalizer and playlist'}"></span>`
        : html`<p class="fg-2">A file from this device has no screenshot. Use it to see it in Classic mode.</p>`}</div>
      <div class="popover__actions">
        ${s.museumUrl ? html`<a class="btn btn--quiet btn--sm" href="${s.museumUrl}" target="_blank" rel="noopener">${icon('external', null, 'ic-16')}On the museum</a>` : ''}
        ${on ? html`<button class="btn btn--sm" data-act="cls-look-close">Close</button>`
          : html`<button class="btn btn--sm btn--primary" data-act="cls-use" data-id="${s.md5}">Use this skin</button>`}
      </div>`;
  }

  /* ---------- partial re-renders: patched (web.md), so the tabs' indicator slides and focus stays where it was ---------- */
  function focusKey(scope) {
    var a = document.activeElement;
    if (!a || !scope.contains(a)) return null;
    if (a.id) return '#' + a.id;
    var act = a.getAttribute('data-act');
    return act ? '[data-act="' + act + '"]' + (a.getAttribute('data-id') ? '[data-id="' + a.getAttribute('data-id') + '"]' : '') : null;
  }
  function repaint(id, markup, fallback) {
    var el = $(id);
    if (!el) return;
    var k = focusKey(el), top = el.scrollTop;
    App.patch(el, markup);
    el.scrollTop = top;
    if (!k) return;
    var f = $(k, el) || (fallback && $(fallback));
    if (f && document.activeElement !== f) f.focus({ preventScroll: true });
  }
  function renderList() { repaint('#cls-list', listMarkup(), '#cls-list'); }
  function renderBrowser() { repaint('#cls-browser', browserMarkup()); }
  function renderChoice() { repaint('#cls-choice', choiceMarkup()); renderList(); }

  /* ---------- hub reads ---------- */
  var browseSeq = 0, browseTimer = 0;
  /* the Browse request for the current query and NSFW setting (nsfw=true only when this device shows them) */
  function browseKey() { return 'q=' + encodeURIComponent(S.query.trim()) + (showNsfw() ? '&nsfw=true' : ''); }
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
        if (S.offline !== wasOffline) renderBrowser(); else renderList();
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
    S.favs.loading = true; S.favs.error = null; S.favs.read = true; renderList();
    return M.api('/api/skins/favourites').then(function (res) {
      S.favs.items = museumSkins(res && res.items);
    }, function (err) { S.favs.error = err; }).then(function () { S.favs.loading = false; renderList(); });
  }
  function loadLocal() {
    S.localRead = true;
    return L.localSkins().then(function (all) {
      S.local = all.map(function (e) { return { md5: e.md5, name: e.name, file: e.file, local: true, screenshotUrl: '', museumUrl: '', nsfw: false }; });
      renderList();
    }, function (err) { console.error('Couldn\'t read the skins kept on this device', err); });
  }
  /* each list once per page load (Try again reloads one); Browse whenever its request changed */
  function readOnce() {
    if (!S.favs.read) loadFavourites();
    if (!S.localRead) loadLocal();
    if (S.tab === 'browse' && !S.offline) ensureBrowse();
  }
  App.on('route', function (cur) { if (cur.path === '/settings/appearance') S.failed = null; });
  window.addEventListener('online', function () { if (S.offline && $('#cls-browser')) { S.offline = false; renderBrowser(); ensureBrowse(); } });

  /* NSFW visibility is per device; the museum list is asked for again with it */
  K.custom['device.showNsfwSkins'] = {
    get: showNsfw,
    set: function (v) { App.storage('showNsfwSkins', v); if (S.tab === 'browse' && !S.offline) ensureBrowse(); else renderList(); }
  };

  /* ---------- choosing ---------- */
  /* a museum skin is downloaded through the hub (which checks and caches it) before it becomes the choice */
  function use(s) {
    if (inUse(s)) { App.toast(s.name + ' is already your Classic skin', { icon: 'check' }); return; }
    if (S.busy) { App.toast('Wait for ' + S.busy.skin.name + ' to finish downloading', { icon: 'clock' }); return; }
    if (S.offline && !isDownloaded(s)) { App.toast('You\'re offline. ' + s.name + ' isn\'t downloaded yet.', { icon: 'offline' }); return; }
    if (s.local) { chose(s); return; }
    S.busy = { skin: s }; S.failed = null;
    App.announce('Downloading ' + s.name);
    renderList();
    L.skinFile(skinFileUrl(s.md5)).then(function () {
      S.busy = null;
      s.downloaded = true; setFlag(s.md5, 'downloaded', true);
      chose(s);
    }, function (err) {
      S.busy = null;
      if (err.status === 401) M.onUnauthorized();
      S.failed = { skin: s, message: err.message };
      App.announce('Couldn\'t download ' + s.name + ': ' + err.message);
      renderList();
    });
  }
  function chose(s) {
    choose(s);
    App.toast(s.name + ' is your Classic skin', { icon: 'check', kind: 'ok', action: 'Open Classic', onAction: function () { App.go('/classic'); } });
    App.announce(s.name + ' is your Classic skin');
  }

  /* a .wsz from this device: kept in IndexedDB under an id from its name, size and date, then chosen */
  function wszName(file) {
    return file.replace(/\.wsz$/i, '').replace(/[_-]+/g, ' ').replace(/\s+/g, ' ').trim()
      .replace(/\b\w/g, function (c) { return c.toUpperCase(); }) || 'Local skin';
  }
  function localId(file) {
    var key = file.name + '\n' + file.size + '\n' + file.lastModified, h = 0x811c9dc5;   // FNV-1a
    for (var i = 0; i < key.length; i += 1) { h ^= key.charCodeAt(i); h = Math.imul(h, 0x01000193) >>> 0; }
    return 'local-' + h.toString(16) + file.size.toString(16);
  }
  function keepFile(file) {
    if (!isWsz(file)) { App.toast(file.name + ' isn\'t a .wsz skin', { kind: 'error', icon: 'warning' }); return Promise.resolve(); }
    if (file.size > MAX_FILE) { App.toast(file.name + ' is over 16 MB, which no classic skin is', { kind: 'error', icon: 'warning' }); return Promise.resolve(); }
    var skin = { md5: localId(file), name: wszName(file.name), file: file.name, local: true, screenshotUrl: '', museumUrl: '', nsfw: false };
    S.busy = { skin: skin }; renderChoice();
    return file.slice(0, 2).text().then(function (magic) {
      if (magic !== 'PK') throw new Error(file.name + ' isn\'t a skin: a .wsz is a zip file.');
      return L.keepLocal({ md5: skin.md5, name: skin.name, file: file.name, blob: file, added: Date.now() }).catch(function () {
        throw new Error('this browser didn\'t let lull keep ' + file.name + '.');
      });
    }).then(function () {
      S.busy = null;
      S.local = [skin].concat(S.local.filter(function (s) { return s.md5 !== skin.md5; }));
      chose(skin);
    }, function (err) {
      S.busy = null; renderChoice();
      App.toast('Couldn\'t keep the skin: ' + err.message, { kind: 'error', icon: 'warning' });
    });
  }
  /* .wsz files dropped on the window become the Classic skin; everything else stays the library's (M3U import) */
  var libraryImport = App.hooks.importFile;
  App.hooks.importFile = function (file) { if (isWsz(file)) keepFile(file); else libraryImport(file); };
  document.addEventListener('change', function (e) {
    var f = e.target.closest && e.target.closest('[data-skin-file]');
    if (f && f.files && f.files[0]) { var file = f.files[0]; f.value = ''; keepFile(file); }
  });

  /* ---------- actions ---------- */
  App.action('cls-use', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (!s) return;
    App.closePopover();
    use(s);
  });
  App.action('cls-in-use', function (el) { var s = findSkin(el.getAttribute('data-id')); if (s) App.toast(s.name + ' is already your Classic skin', { icon: 'check' }); });
  App.action('cls-base', function () {
    choose(null);
    App.toast('Classic mode uses Webamp\'s base skin', { icon: 'undo' });
    var f = $('#cls-choice [data-skin-file]'); if (f) f.focus();
  });
  App.action('cls-look', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (s) App.popover(el, lookMarkup(s), { label: s.name, className: 'cls-pop' });
  });
  App.action('cls-look-close', function () { App.closePopover(true); });
  App.action('cls-tab', function (el) { setTab(el.getAttribute('data-id'), true); });
  App.action('cls-clear', clearSearch);
  App.action('cls-more', function () { if (!S.browse.loading) loadBrowse(true, SEARCH_DEBOUNCE_MS); });
  App.action('cls-reload', function (el) {
    if (el.getAttribute('data-id') === 'favourites') loadFavourites(); else loadBrowse(false);
  });
  /* favourites change at once (newest first, like the hub's list) and roll back with its sentence if refused */
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
    if (!$('[data-act="cls-fav"][data-id="' + s.md5 + '"]')) { var l = $('#cls-list'); if (l) l.focus(); }
  });
  App.action('cls-forget', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (!s || !s.local) return;
    L.dropLocal(s.md5).then(function () {
      S.local = S.local.filter(function (x) { return x.md5 !== s.md5; });
      if (inUse(s)) { choose(null); App.toast('Removed ' + s.name + '. Classic mode uses Webamp\'s base skin.', { icon: 'trash' }); }
      else { renderList(); App.toast('Removed ' + s.name + ' from this device', { icon: 'trash' }); }
      var l = $('#cls-list'); if (l) l.focus();
    }, function (err) { App.toast('Couldn\'t remove ' + s.name + ': ' + err.message, { kind: 'error', icon: 'warning' }); });
  });

  function setTab(id, focus) {
    if (TABS.map(function (t) { return t[0]; }).indexOf(id) < 0) return;
    S.tab = id; S.query = '';
    renderBrowser();
    if (id === 'browse' && !S.offline) ensureBrowse();
    if (focus) { var t = $('#cls-tab-' + id); if (t) t.focus(); }
  }
  function syncClear() { var c = $('#cls-browser .search-field__clear'); if (c) c.hidden = !S.query; }
  function clearSearch() {
    S.query = '';
    var i = $('#cls-search'); if (i) { i.value = ''; i.focus(); }
    syncClear();
    if (S.tab === 'browse' && !S.offline) ensureBrowse(); else renderList();
  }
  /* what can't be a delegated click: typing in the search, arrow keys on the tabs, Esc clearing the search (before
     the app's own Esc) */
  document.addEventListener('input', function (e) {
    if (e.target.id !== 'cls-search') return;
    S.query = e.target.value; syncClear();
    if (S.tab === 'browse' && !S.offline) loadBrowse(false, SEARCH_DEBOUNCE_MS); else renderList();
  });
  document.addEventListener('keydown', function (e) {
    if (e.target.id === 'cls-search' && e.key === 'Escape' && e.target.value) { e.preventDefault(); e.stopPropagation(); clearSearch(); return; }
    if (!e.target.closest || !e.target.closest('#cls-tabs')) return;
    var keys = { ArrowRight: 1, ArrowLeft: -1, Home: -9, End: 9 };
    if (!(e.key in keys)) return;
    e.preventDefault();
    var ids = TABS.map(function (t) { return t[0]; }), i = ids.indexOf(S.tab), n = keys[e.key];
    i = n === -9 ? 0 : n === 9 ? ids.length - 1 : (i + n + ids.length) % ids.length;
    setTab(ids[i], true);
  }, true);
})();
