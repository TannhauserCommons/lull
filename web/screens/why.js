/* Why a pick or a radio song was suggested, in words (ui.reasonText): the reason's evidence (what the listener did,
   counted by the hub, docs/context/api.md#discovery), the strongest true one first, and a follow only when there is one.
   Loaded on first use (App.needWhy), fetched as the pointer, a touch or focus reaches a reason chip; Home's first load
   carries only the chips. */
(function () {
  var html = App.html, ui = App.ui;
  function times(n, one, many) { return n === 1 ? one : n + ' ' + many; }
  function artistWhy(e, followed) {
    if (followed || e.followed) return 'You follow them.';
    if (e.liked) return 'You\'ve liked ' + times(e.liked, 'one of their songs', 'of their songs') + '.';
    if (e.plays) return 'You\'ve played them ' + times(e.plays, 'once', 'times') + '.';
    if (e.saved) return 'You\'ve saved ' + times(e.saved, 'one of their songs', 'of their songs') + '.';
    if (e.library) return 'You have ' + times(e.library, 'one of their albums', 'of their albums') + '.';
    if (e.lastfm) return 'They\'re a favourite in your Last.fm history.';
    return 'An artist you like.';
  }
  ui.reasonText = function (r) {
    var e = r.evidence || {}, followed = !!r.ref && App.isFollowed(r.ref);
    if (r.kind === 'like') return html`Similar to <b>${r.name}</b>. ${artistWhy(e, followed)}`;
    if (r.kind === 'listeners') return html`Played a lot by listeners whose taste is like yours.`;
    if (r.kind === 'fresh') return html`New from <b>${r.name}</b>. ${artistWhy(e, followed)}`;
    if (followed || e.followed) return html`It sits in <b>${r.name}</b>, a tag you follow.`;
    if (e.artists) return html`It sits in <b>${r.name}</b>, which runs through ${times(e.artists, 'an artist', 'artists')} you like.`;
    return html`It sits in <b>${r.name}</b>, a tag that runs through the artists you like.`;
  };
  App.emit('why:ready');
})();
