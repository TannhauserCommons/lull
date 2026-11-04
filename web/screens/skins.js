/* screens/skins.js: Classic mode's skin and size, and where they are chosen, Settings > Appearance > Classic
   (design/DESIGN.md section 12, docs/context/skins.md). Classic itself (screens/classic.js, desktop/winamp.js) is only
   Webamp.
   - The size is per device too: App.classicSize.get() / .set(size) / .double() (Webamp's Double Size: 2x, or 1x from
     2x), stored as localStorage 'lull:classicSize' (LullWebamp.SIZES); 'classic-size' in this page, the storage event
     elsewhere.
   - The choice is per device: App.classicSkin.get() / .choose(skin | null), stored as localStorage 'lull:classicSkin'
     (desktop/webamp-session.js has its shape). A change reaches Classic at once: 'classic-skin' in this page, the
     storage event in other tabs and in the desktop app's Classic window.
   - App.classicSettings() is the section's markup (screens/appearance.js places it): the skin in use, Random skin,
     Add skins, the skin browser, the hub's skins folder, the NSFW and random-on-open switches and what differs in
     Classic mode.
   - The browser: Your skins is the listener's own, GET /api/skins/mine (the skins the hub keeps, `source: hub`, and
     those of the folder LULL_SKINS_DIR it only reads, `source: folder`; read again on `skins.mine` and by Look again:
     POST /api/skins/mine/rescan) plus .wsz files an older lull kept on this device (LullWebamp.localSkins), which it
     offers once to move to the hub; Browse is GET /api/skins?q=&offset=&nsfw= (the hub's Winamp Skin Museum proxy, 30 a
     page, search debounced); Favourites is /api/skins/favourites (PUT / DELETE one); Downloaded is what the hub has
     cached (its flag) plus the files kept on this device. Offline, Browse lists favourites and downloaded skins from
     the last answers. Choosing a skin from the hub downloads it first, so a skin that can't be had says why here,
     not later in Webamp.
   - Random skin (LullWebamp.pickSkin, the rule): one of Your skins, else of the favourites, never the one in use,
     chosen as any other. With "New random skin each time Classic opens" on (per device, localStorage
     'lull:classicRandom'), App.classicSkin.opening() picks one as Classic opens (screens/classic.js asks it before
     Webamp gets a skin, in the browser and for the app's window).
   - .wsz files: "Add skins" and .wsz files dropped anywhere on lull go to the hub (POST /api/skins/mine), so every
     device has them; one is chosen at once, several are listed under Your skins. A skin the hub keeps can be deleted
     (DELETE /api/skins/mine/{md5}); one in the skins folder can't, and its card says why. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, ui = App.ui, M = App.data, icon = App.icon, $ = App.$;
  var K = App.settingsKit, row = K.row, group = K.group, switchRow = K.switchRow, choice = K.choice;
  var L = window.LullWebamp;

  var SEARCH_DEBOUNCE_MS = 300;
  var MAX_FILE = 16 * 1024 * 1024;                      // the hub's cap for a .wsz
  var MINE_PAGE = 60;                                   // Your skins shows this many more at a time
  var TABS = [['mine', 'Your skins'], ['browse', 'Browse'], ['favourites', 'Favourites'], ['downloaded', 'Downloaded']];

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
  App.classicSkin = { get: chosen, choose: choose, keepFiles: keepFiles, opening: opening };
  /* the size: Settings' Size row and Webamp's Double Size both set it */
  function setSize(size) {
    L.writeSize(size);
    App.emit('classic-size', size);
    K.showChoice('device.classicSize', size);
  }
  App.classicSize = {
    get: L.readSize, set: setSize,
    double: function () {
      var size = L.doubled(L.readSize());
      setSize(size);
      App.announce('Classic size ' + size + '\u00d7');
    }
  };
  K.custom['device.classicSize'] = { get: L.readSize, set: function (v) { setSize(+v); } };
  /* another tab or window chose: this page's Classic and Settings follow */
  window.addEventListener('storage', function (e) {
    if (e.key === L.SIZE_KEY) { App.emit('classic-size', L.readSize()); K.showChoice('device.classicSize', L.readSize()); return; }
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
    local: [], localRead: null,           // .wsz files an older lull kept on this device: { md5: 'local-…', name, file, local }
    /* the listener's skins on the hub: { md5, name, file, mine, removable } (removable: the hub keeps it, not the
       folder); folder null when LULL_SKINS_DIR isn't set */
    mine: { items: [], loading: false, error: null, read: null, answered: false, folder: null, problem: null, watching: false, skipped: [], shown: MINE_PAGE },
    adding: 0,                            // skins being added to the hub
    moving: false,                        // this device's skins going to the hub
    picking: false,                       // Random skin, reading the lists
    browse: { items: [], more: false, loading: false, error: null, key: null, want: null },  // key: the request items answer
    favs: { items: [], loading: false, error: null, read: null },
    offline: false,
    busy: null,                           // { skin }: downloading through the hub before it is chosen
    failed: null                          // { skin, message }
  };
  function showNsfw() { return App.storage('showNsfwSkins') === true; }
  function isDownloaded(s) { return !!s.local || !!s.downloaded; }
  function isOwn(s) { return !!s.local || !!s.mine; }
  function isFavourite(s) { return !!s.favourite; }
  function sameSkin(a, b) { return !!a && !!b && a.md5 === b.md5; }
  /* every skin object known now (one md5 can appear in several lists), first copy of each */
  function knownSkins() {
    var seen = {};
    return [].concat(S.local, S.mine.items, S.favs.items, S.browse.items, S.failed ? S.failed.skin : [])
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
  /* the hub's skins folder, checked at the boundary as museum skins are */
  function mineSkins(list) {
    var out = [];
    (list || []).forEach(function (s) {
      if (!s || !/^[0-9a-f]{32}$/i.test(s.md5)) { console.error('Skipping a skin without an md5 id', s); return; }
      out.push({ md5: s.md5, name: String(s.name || s.file || s.md5), file: String(s.path || s.file || ''), mine: true,
        removable: s.source === 'hub', screenshotUrl: '', museumUrl: '', nsfw: false, favourite: false, downloaded: true });
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
        <div class="sys-row__text"><span class="sys-row__label" id="cls-find-l">Find a skin</span><span class="sys-row__help">Your own skins, the Winamp Skin Museum, your favourites and the skins already downloaded. Select a picture to see it larger.</span></div>
        <div class="sys-row__ctl cls-browser" id="cls-browser" role="group" aria-labelledby="cls-find-l">${browserMarkup()}</div></div>
      ${switchRow('device.classicRandom', 'New random skin each time Classic opens', 'Classic opens in one of your skins at random on this device, never the last one. With no skins of your own, it picks from your favourites.')}
      ${K.cfgField('LULL_SKINS_DIR', 'Skins folder', 'Optional: a folder of .wsz files you already keep on your hub. lull reads it, in every subfolder, and never changes it. Its skins show under Your skins beside the ones you add.')}
      ${row('Size', 'How big Webamp is drawn, in the browser and in the app\'s Winamp window, with every skin pixel kept sharp. If a size doesn\'t fit the screen, Classic shows the largest that does. In Classic, Ctrl+D (Webamp\'s Double Size) switches between 1\u00d7 and 2\u00d7.',
        choice('device.classicSize', 'Size', L.SIZES.map(function (z) { return [z, z + '\u00d7']; })))}
      ${switchRow('device.showNsfwSkins', 'Show NSFW skins', 'The museum marks some skins NSFW. They stay hidden while you browse on this device unless this is on.')}
      ${row('Classic mode', 'Webamp plays your queue. Crossfade, loudness normalisation and gapless playback pause there, and repeat is on or off only. Esc, W or Webamp\'s close button brings you back.', html`<a class="btn btn--sm" href="#/classic">${icon('classic')}Open Classic mode</a>`)}`;
  };

  /* the skin in use, and the two ways to change it that aren't the browser */
  function choiceMarkup() {
    var c = chosen();
    return html`<div class="cls-now s-card">
        ${shotMarkup(c)}
        <div class="cls-now__text">
          <span class="cls-now__label">In use</span>
          <strong class="cls-now__name ellipsis">${nameOf(c)}</strong>
          <span class="cls-now__src">${!c ? 'Webamp\'s own look' : /^local-/.test(c.md5) ? 'A file kept only on this device' : sourceOf(c.md5)}</span>
        </div>
      </div>
      <div class="cluster">
        <button class="btn btn--sm" data-act="cls-random" ${S.picking ? raw('aria-disabled="true"') : ''}>${S.picking ? html`<span class="spin-ring" aria-hidden="true"></span>` : icon('shuffle', null, 'ic-16')}Random skin</button>
        <label class="btn btn--sm" data-tip="Your hub keeps them, so every device has them">${S.adding ? html`<span class="spin-ring" aria-hidden="true"></span>` : icon('plus', null, 'ic-16')}${S.adding ? 'Adding skins' : 'Add skins'}<input class="vh" type="file" accept=".wsz" multiple data-skin-file></label>
        ${c ? html`<button class="btn btn--sm btn--quiet" data-act="cls-base">${icon('undo', null, 'ic-16')}Use Webamp's base skin</button>` : ''}
      </div>`;
  }

  function browserMarkup() {
    var i = TABS.map(function (t) { return t[0]; }).indexOf(S.tab);
    var ph = S.tab === 'mine' ? 'Search your skins' : S.tab === 'favourites' ? 'Search favourites'
      : S.tab === 'downloaded' || S.offline ? 'Search downloaded skins' : 'Search the museum';
    return html`<div class="cls-browser__head">
        <div class="seg" role="tablist" aria-label="Skin lists" style="--n:${TABS.length};--i:${i}" id="cls-tabs">
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
    var list = museum ? S.browse.items : S.tab === 'favourites' ? S.favs.items : S.tab === 'mine' ? ownSkins()
      : knownSkins().filter(function (s) { return !s.mine && (isDownloaded(s) || (S.tab === 'browse' && isFavourite(s))); });
    var q = museum ? '' : S.query.trim().toLowerCase(), nsfw = showNsfw();
    return list.filter(function (s) {
      if (s.nsfw && !nsfw) return false;
      return !q || s.name.toLowerCase().indexOf(q) >= 0;
    });
  }
  function listMarkup() {
    var notes = S.offline ? html`<div class="banner banner--warn s-card cls-note">${icon('offline')}<span class="banner__msg">You're offline, so only favourite and downloaded skins are listed.</span></div>` : '';
    var museum = S.tab === 'browse' && !S.offline, b = S.browse, src = museum ? b : S.tab === 'favourites' ? S.favs : null;
    if (S.tab === 'mine') return html`${notes}${mineMarkup()}`;
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
  /* Your skins: the hub's folder (what it is, what it skipped, Look again) and this device's files, MINE_PAGE at a time */
  function mineMarkup() {
    var m = S.mine;
    if (!m.answered && !m.error) return html`<p class="cls-meta" role="status"><span class="spin-ring" aria-hidden="true"></span>Reading your skins</p>`;
    var list = visibleSkins(), shown = list.slice(0, m.shown), q = S.query.trim();
    var body = !list.length ? (q ? emptyMarkup() : mineEmptyMarkup())
      : html`<p class="cls-meta" role="status">${list.length + (list.length === 1 ? ' skin' : ' skins') + (q ? ' match "' + q + '"' : '')}</p>
        <ul class="cls-grid">${shown.map(cardMarkup)}</ul>
        ${list.length > shown.length ? html`<div class="cls-more"><button class="btn btn--sm" data-act="cls-mine-more">Show more skins</button></div>` : ''}`;
    return html`${folderMarkup()}${body}`;
  }
  /* where Your skins come from on the hub, and what to do when that's nothing */
  function folderMarkup() {
    var m = S.mine;
    if (m.error) {
      return html`<div class="banner banner--down s-card cls-note">${icon('warning')}<span class="banner__msg">Couldn't read your skins folder: ${m.error.message}</span>
        <button class="btn btn--sm" data-act="cls-reload" data-id="mine">Try again</button></div>`;
    }
    /* the skins folder is named while it gives skins, or while the hub can't see it change; empty, it says nothing */
    var skipped = m.skipped.length, folder = !!m.folder && (!m.watching || m.items.some(function (s) { return !s.removable; }));
    if (!folder && !skipped && !m.problem) return moveMarkup();
    return html`${moveMarkup()}<div class="cls-folder">
        <p class="cls-folder__text">${icon('link', null, 'ic-16')}<span>${m.problem ? m.problem
          : folder ? html`Also from ${m.folder} on your hub, which lull only reads${m.watching ? '' : '. lull can\'t see changes there by itself, so look again after adding skins'}.`
          : 'Your skins are kept on your hub.'}</span></p>
        <button class="btn btn--sm btn--quiet" data-act="cls-rescan" ${m.loading ? raw('aria-disabled="true"') : ''}>${m.loading ? html`<span class="spin-ring" aria-hidden="true"></span>` : icon('refresh', null, 'ic-16')}Look again</button>
      </div>
      ${skipped ? html`<details class="cls-skipped"><summary>${skipped === 1 ? '1 file on your hub isn\'t a classic skin' : skipped + ' files on your hub aren\'t classic skins'}</summary>
        <ul>${m.skipped.map(function (x) { return html`<li>${x.path}: ${x.why}</li>`; })}</ul></details>` : ''}`;
  }
  function mineEmptyMarkup() {
    return ui.emptyState({ art: 'slot', title: 'No skins of your own yet',
      text: 'Add .wsz files and your hub keeps them, so every device lists them here. You can also drop them anywhere on lull.' + (S.mine.folder ? ' Files you put in your skins folder show here too.' : ''),
      action: { label: 'Add skins', act: 'cls-add' } });
  }
  /* skins an older lull kept on this device: offered once, until moved or declined (localStorage 'lull:skinsMoveOffer') */
  function moveMarkup() {
    var n = S.local.length;
    if (!n || App.storage('skinsMoveOffer') === 'declined') return '';
    return html`<div class="banner s-card cls-note" role="region" aria-label="Skins on this device">${icon('device')}
      <span class="banner__msg">${n === 1 ? '1 skin is' : n + ' skins are'} kept only on this device. Move ${n === 1 ? 'it' : 'them'} to your hub and every device has ${n === 1 ? 'it' : 'them'}.</span>
      <button class="btn btn--sm btn--primary" data-act="cls-move" ${S.moving ? raw('aria-disabled="true"') : ''}>${S.moving ? html`<span class="spin-ring" aria-hidden="true"></span>Moving` : html`${icon('arrow-up', null, 'ic-16')}Move to your hub`}</button>
      ${S.moving ? '' : html`<button class="btn btn--sm btn--quiet" data-act="cls-move-no">Keep ${n === 1 ? 'it' : 'them'} here</button>`}</div>`;
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
    return ui.emptyState({ art: 'slot', title: 'Nothing downloaded yet', text: 'Skins you choose are cached on your hub, so they work offline.', action: { label: 'Browse skins', act: 'cls-tab', id: 'browse' } });
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
      : s.local ? html`${icon('device', null, 'ic-16')}Only on this device`
      : s.mine && s.removable ? html`${icon('downloaded', null, 'ic-16')}On your hub`
      : s.mine ? html`${icon('link', null, 'ic-16')}In your skins folder`
      : isDownloaded(s) ? html`${icon('saved', null, 'ic-16')}Downloaded`
      : html`${icon('external', null, 'ic-16')}On the museum`;
    var blocked = busy || (S.offline && !isDownloaded(s));
    return html`<li class="cls-card s-card${on ? ' is-applied' : ''}${failed ? ' is-failed' : ''}" data-skin="${s.md5}">
      <button class="cls-card__shot" data-act="cls-look" data-id="${s.md5}" aria-haspopup="dialog" aria-expanded="false" aria-label="${'See ' + s.name + ' larger'}" data-tip="See it larger">${shotMarkup(s)}</button>
      <div class="cls-card__text">
        <span class="cls-card__name ellipsis">${s.name}</span>
        ${isOwn(s) ? html`<span class="cls-card__file ellipsis" title="${s.file}">${s.file}</span>` : ''}
        <span class="cls-card__status${failed ? ' is-bad' : ''}">${status}</span>
        ${failed ? html`<span class="cls-card__why">${S.failed.message}</span>` : ''}
      </div>
      <div class="cls-card__acts">
        ${useButton(s, on, failed, blocked)}
        <span class="spacer"></span>
        ${s.mine ? deleteButton(s) : s.local ? html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-forget" data-id="${s.md5}" aria-label="${'Remove ' + s.name + ' from this device'}" data-tip="Remove from this device">${icon('trash', null, 'ic-16')}</button>`
          : html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-fav" data-id="${s.md5}" aria-pressed="${isFavourite(s) ? 'true' : 'false'}" aria-label="${'Favourite ' + s.name}" data-tip="${isFavourite(s) ? 'Remove from favourites' : 'Add to favourites'}">${icon(isFavourite(s) ? 'star-fill' : 'star', null, 'ic-16')}</button>`}
        ${s.museumUrl ? html`<a class="btn-icon btn-icon--bare btn-icon--sm" href="${s.museumUrl}" target="_blank" rel="noopener" aria-label="${'View ' + s.name + ' on the Winamp Skin Museum'}" data-tip="View on the Winamp Skin Museum">${icon('external', null, 'ic-16')}</a>` : ''}
      </div>
    </li>`;
  }
  /* a skin the hub keeps is deleted from it; one in the skins folder says why it can't be */
  function deleteButton(s) {
    if (s.removable) return html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-delete" data-id="${s.md5}" aria-label="${'Delete ' + s.name + ' from your hub'}" data-tip="Delete from your hub">${icon('trash', null, 'ic-16')}</button>`;
    return html`<button class="btn-icon btn-icon--bare btn-icon--sm" data-act="cls-delete" data-id="${s.md5}" aria-disabled="true" aria-label="${s.name + ' is in your skins folder, which lull only reads'}" data-tip="In your skins folder, which lull only reads: remove it there">${icon('lock', null, 'ic-16')}</button>`;
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
        : html`<p class="fg-2">${s.local ? 'A file on this device' : 'One of your skins'} has no screenshot. Use it to see it in Classic mode.</p>`}</div>
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
    S.favs.loading = true; S.favs.error = null; renderList();
    return (S.favs.read = M.api('/api/skins/favourites').then(function (res) {
      S.favs.items = museumSkins(res && res.items);
    }, function (err) { S.favs.error = err; }).then(function () { S.favs.loading = false; renderList(); }));
  }
  function loadLocal() {
    return (S.localRead = L.localSkins().then(function (all) {
      S.local = all.map(function (e) { return { md5: e.md5, name: e.name, file: e.file, local: true, screenshotUrl: '', museumUrl: '', nsfw: false }; });
      renderList();
    }, function (err) { console.error('Couldn\'t read the skins kept on this device', err); }));
  }
  /* the hub's skins folder as it last read it, or (rescan) read again now */
  function loadMine(rescan) {
    var m = S.mine;
    m.loading = true; m.error = null; renderList();
    return (m.read = M.api('/api/skins/mine' + (rescan ? '/rescan' : ''), rescan ? { method: 'POST' } : undefined).then(function (res) {
      m.answered = true;
      m.items = mineSkins(res && res.items); m.folder = res && res.folder || null; m.problem = res && res.problem || null;
      m.watching = !!(res && res.watching);
      m.skipped = ((res && res.skipped) || []).filter(function (x) { return x && typeof x.path === 'string'; })
        .map(function (x) { return { path: x.path, why: String(x.why || '') }; });
    }, function (err) { m.error = err; }).then(function () { m.loading = false; renderChoice(); }));
  }
  /* each list once per page load (Try again reloads one); Browse whenever its request changed */
  function readOnce() {
    if (!S.favs.read) loadFavourites();
    if (!S.localRead) loadLocal();
    if (!S.mine.read) loadMine();
    if (S.tab === 'browse' && !S.offline) ensureBrowse();
  }
  /* the hub read its folder again and found something else */
  App.on('server:skins.mine', function () { if (S.mine.read) loadMine(); });
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
  /* one toast says what the skin is: a quicker choice (Random, again and again) replaces it, never stacks under it */
  var choseToast = null;
  function chose(s) {
    choose(s);
    if (choseToast) choseToast.close();
    choseToast = App.toast(s.name + ' is your Classic skin', { icon: 'check', kind: 'ok', action: 'Open Classic', onAction: function () { App.go('/classic'); } });
    App.announce(s.name + ' is your Classic skin');
  }

  /* ---------- Random skin ---------- */
  function ownSkins() { return [].concat(S.mine.items, S.local); }
  /* where a chosen hub skin comes from, for the skin in use */
  function sourceOf(md5) {
    var m = S.mine.items.filter(function (s) { return s.md5 === md5; })[0];
    return !m ? 'From the Winamp Skin Museum' : m.removable ? 'One of your skins on the hub' : 'From your skins folder';
  }
  /* the skin Random gives now, from the lists as this page has them (each read once, so a pick can't miss one) */
  function pickRandom() {
    return Promise.all([S.mine.read || loadMine(), S.localRead || loadLocal(), S.favs.read || loadFavourites()]).then(function () {
      var c = chosen();
      return L.pickSkin(ownSkins(), S.favs.items, c ? c.md5 : null);
    });
  }
  /* why Random has nothing to give */
  function noOther() {
    var own = ownSkins(), pool = own.length ? own : S.favs.items, c = chosen();
    if (!pool.length) return 'No skins to pick from yet. Add .wsz files or star favourites.';
    return (c ? c.name || 'Your skin' : 'It') + ' is your only ' + (own.length ? 'skin' : 'favourite') + ', and it\'s in use';
  }
  App.action('cls-random', function () {
    if (S.picking) return;
    if (S.busy) { App.toast('Wait for ' + S.busy.skin.name + ' to finish downloading', { icon: 'clock' }); return; }
    S.picking = true; renderChoice();
    pickRandom().then(function (s) {
      S.picking = false; renderChoice();
      if (s) use(s); else App.toast(noOther(), { icon: 'info' });
    });
  });
  /* Classic opening: the skin to start in. With random-on-open on, a new one is chosen first, once its file can be
     had; otherwise (or with nothing else to pick, or when it can't be had) the choice stays */
  function opening() {
    if (App.storage('classicRandom') !== true) return Promise.resolve(chosen());
    return pickRandom().then(function (s) {
      if (!s) return null;
      return L.skinFile(s.local ? L.skinUrl(new URL('./', location.href).href, s) : skinFileUrl(s.md5)).then(function () {
        choose(s);
      });
    }).then(function () { return chosen(); }, function (err) {
      console.warn('Classic keeps its skin: the random one couldn\'t be had', err);
      return chosen();
    });
  }
  K.custom['device.classicRandom'] = {
    get: function () { return App.storage('classicRandom') === true; },
    set: function (v) { App.storage('classicRandom', !!v); }
  };

  /* ---------- adding skins: .wsz files go to the hub, which checks and keeps them for every device ---------- */
  /* one file to the hub: { added, skin }, or a rejection with a sentence that names the file */
  function upload(file) {
    if (!isWsz(file)) return Promise.reject(new Error(file.name + ' isn\'t a .wsz skin'));
    if (file.size > MAX_FILE) return Promise.reject(new Error(file.name + ' is over 16 MB, which no classic skin is'));
    return M.api('/api/skins/mine?name=' + encodeURIComponent(file.name), { method: 'POST', body: file }).then(function (res) {
      var skin = mineSkins([res && res.item])[0];
      if (!skin) throw new Error('the hub didn\'t say what it kept of ' + file.name);
      return { added: !!res.added, skin: skin };
    }, function (err) {
      throw new Error(err.status === 400 ? err.message : file.name + ': ' + err.message);
    });
  }
  /* a skin the hub now keeps, listed before its next answer comes */
  function remember(skin) {
    if (!S.mine.items.some(function (x) { return x.md5 === skin.md5; })) S.mine.items = [skin].concat(S.mine.items);
  }
  /* .wsz files added or dropped, in turn. One becomes the Classic skin; several join Your skins, and the skin in use
     stays */
  function keepFiles(files) {
    if (!files.length) return Promise.resolve();
    S.adding += files.length; renderChoice();
    var added = [], had = [], failed = [];
    return files.reduce(function (p, file) {
      return p.then(function () {
        return upload(file).then(function (r) { (r.added ? added : had).push(r.skin); remember(r.skin); },
          function (err) { failed.push(err.message); });
      });
    }, Promise.resolve()).then(function () {
      S.adding -= files.length;
      var got = added.concat(had);
      if (got.length) loadMine();
      renderChoice();
      if (files.length === 1 && got.length) { use(got[0]); return; }
      if (!got.length) { App.toast('Couldn\'t add ' + (failed.length === 1 ? 'the skin: ' : 'the skins: ') + failed.join('; '), { kind: 'error', icon: 'warning' }); return; }
      var text = added.length ? 'Added ' + plural(added.length, 'skin') + ' to your hub. Every device lists ' + (added.length === 1 ? 'it' : 'them') + ' under Your skins.' : '';
      if (had.length) text += (text ? ' ' : '') + plural(had.length, 'skin') + (had.length === 1 ? ' was' : ' were') + ' there already.';
      if (failed.length) text += ' ' + failed.length + ' couldn\'t be added: ' + failed.join('; ');
      App.toast(text, { icon: 'check', kind: failed.length ? 'error' : 'ok', action: $('#cls-browser') ? 'Show' : null, onAction: function () { setTab('mine', true); } });
    });
  }
  function plural(n, one) { return n + ' ' + one + (n === 1 ? '' : 's'); }
  /* this device's skins to the hub, each dropped here once the hub has it; the skin in use stays in use */
  function moveLocal() {
    var c = chosen(), moved = 0, failed = [];
    S.moving = true; renderList();
    return L.localSkins().then(function (all) {
      return all.reduce(function (p, e) {
        return p.then(function () {
          return upload(new File([e.blob], e.file || e.name + '.wsz')).then(function (r) {
            remember(r.skin); moved += 1;
            if (c && c.md5 === e.md5) choose(r.skin);
            return L.dropLocal(e.md5).then(function () {
              S.local = S.local.filter(function (x) { return x.md5 !== e.md5; });
            });
          }, function (err) { failed.push(err.message); });
        });
      }, Promise.resolve());
    }).catch(function (err) { failed.push('this device\'s skins couldn\'t be read (' + err.message + ')'); }).then(function () {
      S.moving = false;
      if (moved) loadMine(); else renderChoice();
      if (failed.length) {
        App.toast((moved ? 'Moved ' + plural(moved, 'skin') + ' to your hub. ' : '') + failed.length + ' couldn\'t be moved: ' + failed.join('; '), { kind: 'error', icon: 'warning' });
      } else App.toast('Moved ' + plural(moved, 'skin') + ' to your hub. Every device lists ' + (moved === 1 ? 'it' : 'them') + ' under Your skins.', { kind: 'ok', icon: 'check' });
    });
  }
  /* .wsz files dropped on the window or added become Classic skins; anything else stays the library's (M3U import) */
  var libraryImport = App.hooks.importFile;
  App.hooks.importFile = function (file) { if (isWsz(file)) keepFiles([file]); else libraryImport(file); };
  document.addEventListener('change', function (e) {
    var f = e.target.closest && e.target.closest('[data-skin-file]');
    if (f && f.files && f.files.length) { var files = [].slice.call(f.files); f.value = ''; keepFiles(files); }
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
    var f = $('#cls-choice [data-act="cls-random"]'); if (f) f.focus();
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
    var id = el.getAttribute('data-id');
    if (id === 'favourites') loadFavourites(); else if (id === 'mine') loadMine(); else loadBrowse(false);
  });
  App.action('cls-rescan', function () { if (!S.mine.loading) loadMine(true); });
  App.action('cls-add', function () { var f = $('[data-skin-file]'); if (f) f.click(); });
  App.action('cls-move', function () { if (!S.moving && S.local.length) moveLocal(); });
  App.action('cls-move-no', function () {
    App.storage('skinsMoveOffer', 'declined'); renderList();
    App.toast('They stay on this device only. Add them with Add skins whenever you like.', { icon: 'device' });
    var l = $('#cls-list'); if (l) l.focus();
  });
  /* a skin the hub keeps is deleted from it, for every device, once confirmed; the skins folder is only read */
  App.action('cls-delete', function (el) {
    var s = findSkin(el.getAttribute('data-id'));
    if (!s || !s.mine) return;
    if (!s.removable) {
      App.toast(s.name + ' is in your skins folder' + (S.mine.folder ? ', ' + S.mine.folder + ',' : '') + ' which lull only reads. Remove it there.', { icon: 'lock' });
      return;
    }
    App.confirm({ title: 'Delete ' + s.name + '?', danger: true, confirm: 'Delete',
      text: 'Your hub deletes the file, so it goes from Your skins on every device.' + (inUse(s) ? ' Classic mode here goes back to Webamp\'s base skin.' : '') }, function () {
      M.api('/api/skins/mine/' + encodeURIComponent(s.md5), { method: 'DELETE' }).then(function () {
        S.mine.items = S.mine.items.filter(function (x) { return x.md5 !== s.md5; });
        if (inUse(s)) { choose(null); App.toast('Deleted ' + s.name + '. Classic mode uses Webamp\'s base skin.', { icon: 'trash' }); }
        else { renderList(); App.toast('Deleted ' + s.name + ' from your hub', { icon: 'trash' }); }
        var l = $('#cls-list'); if (l) l.focus();
      }, function (err) { App.toast('Couldn\'t delete ' + s.name + ': ' + err.message, { kind: 'error', icon: 'warning' }); });
    });
  });
  App.action('cls-mine-more', function () { S.mine.shown += MINE_PAGE; renderList(); });
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
    if (!s || isOwn(s)) return;
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
    S.tab = id; S.query = ''; S.mine.shown = MINE_PAGE;
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
