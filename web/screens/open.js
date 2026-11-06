/* screens/open.js.
   Opening a link (#/open?link=): a song, album, artist or playlist someone shared, from their lull (on any hub) or a
   music service, opens as this hub's own page: GET /api/open (song.link, album.link, Apple Music, Deezer, MusicBrainz,
   Last.fm, and the ids after a lull link's `#lull?`); a playlist link opens as its preview (#/outside). A pasted link
   in search lands here, as do the apps' lull:// links and the share page's. A song opens on its album and plays when a
   click or key brought it here; otherwise a toast offers it.
   `do=queue` (a share page's Add to queue, through the apps or the browser): a song or an album joins next up without
   playing or interrupting anything: of the listener's device playing now when this one isn't (Connect's enqueue),
   else here (with nothing loaded, the first waits paused). lull goes back to the screen it showed, or Home when it
   opened on this link.
   Loaded on the first link (discover.js registers /open with need: App.needScript), so a start that opens none never
   fetches it. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, P = App.player;

  var PLAYLIST_LINK = /[?&]list=|\/playlist\/|\/sets\/|open\.spotify\.com\/album\//;
  function openLink(link, how) {
    if (PLAYLIST_LINK.test(link) && link.indexOf('#lull?') < 0) { App.go('/outside/' + encodeURIComponent(link), { replace: true }); return; }
    M.api('/api/open?link=' + encodeURIComponent(link)).then(function (r) {
      M.absorb(r.entities);
      if (how === 'queue' && r.kind !== 'artist') return enqueueOpened(r);
      if (r.kind === 'artist') return App.go('/artist/' + r.id, { replace: true });
      if (r.kind === 'album') return App.go('/album/' + r.id, { replace: true });
      var t = M.track(r.id);
      if (!t) throw new M.ApiError(0, 'bad_open', 'The hub opened a song it didn\'t describe.');
      App.go(t.album ? '/album/' + t.albumId : '/artist/' + t.artistId, { replace: true });
      var asked = navigator.userActivation ? navigator.userActivation.isActive : false;
      if (asked) P.playTrack(t.id, [t.id]);
      else App.toast(t.title + ', ' + t.artist, { icon: 'link', action: 'Play', onAction: function () { P.playTrack(t.id, [t.id]); } });
    }).catch(function (err) {
      App.go('/search', { replace: true });
      App.toast('Couldn\'t open that link: ' + err.message, { kind: 'error', icon: 'link' });
    });
  }

  function enqueueOpened(r) {
    var ids = r.kind === 'album' ? App.albumTracks(r.id) : Promise.resolve([r.id]);
    return ids.then(function (list) {
      leaveOpen();
      if (!list.length) return App.toast('That album has no songs to add yet', { icon: 'info' });
      var there = !P.state.playing && playingElsewhere();
      if (!there) return P.addToQueue(list, { paused: true });
      enqueue(there, list).then(function () {
        var what = list.length > 1 ? list.length + ' songs' : M.track(list[0]).title;
        App.toast('Added to ' + there.name + '\'s queue: ' + what, { confirms: true, icon: 'connect' });
      }, function () { P.addToQueue(list, { paused: true }); });
    });
  }
  /* the listener's device playing now, other than this one (the newest report that says it plays), or null */
  function playingElsewhere() {
    return App.connect.others().filter(function (d) { return d.state && d.state.playing; })
      .sort(function (a, b) { return b.state.recv - a.state.recv; })[0] || null;
  }
  /* songs into another device's next up (the hub takes up to 500 at once; connect.js plays them there) */
  function enqueue(d, ids) {
    return M.api('/api/devices/' + encodeURIComponent(d.id) + '/command', { method: 'POST', body: { action: 'enqueue', tracks: ids.slice(0, 500) } });
  }
  /* back from #/open to the screen lull showed before, or Home when lull opened on the link (a tab the share page
     opened): nothing else to see here */
  var openedFirst = false;
  function leaveOpen() {
    if (openedFirst) App.go('/home', { replace: true }); else history.back();
  }

  function renderOpen() {
    return html`<div class="view__inner"><section class="card disc-card" aria-busy="true">${ui.labelRule('opening the link')}${ui.skeleton('row', 3)}</section></div>`;
  }
  function mountOpen(ctx) {
    var link = (ctx.query.link || '').trim();
    openedFirst = App.current.first;
    if (link) openLink(link, ctx.query.do); else App.go('/search', { replace: true });
  }
  App.route('/open', { render: renderOpen, mount: mountOpen, nav: 'search', title: 'Opening a link' });
})();
