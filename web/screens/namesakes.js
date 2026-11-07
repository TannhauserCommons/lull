/* screens/namesakes.js: artists who share a name (ADR-0035, the `namesakes` switch), loaded on first use so no other
   screen pays for it (library.js registers the route and the album menu's item).
   - #/artists/named/<name>: "Artists called Nirvana": every artist of the name, each told apart by its own line
     (MusicBrainz's comment, else type, country and years) and an album lull knows; the listener's own first. Below,
     the listener's songs of the name that nothing places, each with "Whose is this?".
   - The listener's word, from an album's menu ("Not by Nirvana?") or an unplaced song: "This is by …" one of the
     name's artists, or "This is another Nirvana". Nothing is picked for them; their word is kept and nothing else moves it.
   Built from the library's own parts (lib-page, lib-card, lib-flist, the release picker's rows), so every design and
   theme dresses it as it dresses those. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, $ = App.$;
  var style = document.createElement('style');
  style.textContent = '@layer screens {' + [
    '.ns-pick .disc-rel__row { grid-template-columns: 24px minmax(0, 1fr); }',
    '.ns-pick .disc-rel__title small { white-space: normal; }'
  ].join('\n') + '}';
  document.head.appendChild(style);

  function path(name) { return '/api/artists/named?name=' + encodeURIComponent(name); }
  function plural(n, one) { return n + ' ' + one + (n === 1 ? '' : 's'); }
  function rule(text, id) { return html`<div class="label-rule lib-rule"><h2 id="${id}">${text}</h2></div>`; }
  /* what tells an artist apart: its description, and an album lull knows */
  function line(d, id) {
    var al = d.albums && d.albums[id] && M.album(d.albums[id]);
    return [d.yours.indexOf(id) >= 0 ? 'Yours' : '', d.about[id] || '', al ? al.title + (al.year ? ' (' + al.year + ')' : '') : '']
      .filter(Boolean).join(' · ') || 'No description yet';
  }

  function render(ctx) {
    var name = decodeURIComponent(ctx.params.name), r = M.get(path(name));
    if (r.error) return html`<div class="view__inner lib">${App.errorCard(r.error, { path: path(name) })}</div>`;
    if (r.loading) return html`<div class="view__inner lib"><section class="card lib-page" aria-label="Loading">${ui.skeleton('text', 2)}${ui.skeleton('row', 4)}</section></div>`;
    var d = r.data, artists = d.artists.map(M.artist).filter(Boolean);
    /* the listener's own first, then oldest first as the hub lists them */
    artists.sort(function (a, b) { return (d.yours.indexOf(a.id) < 0) - (d.yours.indexOf(b.id) < 0); });
    var songs = (d.unplaced || []).map(M.track).filter(Boolean);
    return html`<div class="view__inner lib lib-named">
      <section class="card lib-page" aria-labelledby="ns-title">
        <p class="lib-kicker">name</p>
        <h1 class="t-display lib-title" id="ns-title">Artists called ${d.name}</h1>
        <p class="lib-ahero__facts"><span>${plural(artists.length, 'artist')} share this name. Open the one you mean.</span></p>
        ${d.errors && d.errors.musicbrainz ? html`<p class="t-micro fg-2">MusicBrainz didn't answer, so others called ${d.name} may be missing.</p>` : ''}
      </section>
      ${d.askFollow ? html`<section class="card lib-card" aria-labelledby="ns-follow-h">
        ${rule('which one you follow', 'ns-follow-h')}
        <p class="t-micro fg-2">You followed ${d.name} before lull knew of more than one. Until you say which, the follow counts for none of them.</p>
        <ul class="lib-flist">${artists.map(function (ar) {
          return html`<li class="lib-frow"><div class="lib-frow__main"><span class="lib-frow__name">${ar.name}</span><span class="lib-frow__meta">${line(d, ar.id)}</span></div>
            <button class="btn btn--sm" data-act="ns-follow-this" data-name="${d.name}" data-id="${ar.id}">This one</button></li>`;
        })}</ul>
      </section>` : ''}
      <section class="card lib-card" aria-labelledby="ns-list-h">
        ${rule('who they are', 'ns-list-h')}
        <ul class="lib-flist">${artists.map(function (ar) {
          return html`<li class="lib-frow">${ui.cover(ar, 48, { round: true })}
            <div class="lib-frow__main"><a class="lib-frow__name" href="#/artist/${ar.id}">${ar.name}</a><span class="lib-frow__meta">${line(d, ar.id)}</span></div>
            ${ui.iconButton('radio', 'Start radio of this ' + ar.name, { 'data-act': 'start-radio', 'data-kind': 'artist', 'data-id': ar.id, cls: 'btn-icon--bare' })}</li>`;
        })}</ul>
      </section>
      ${songs.length ? html`<section class="card lib-card" aria-labelledby="ns-songs-h">
        ${rule('songs lull can\'t place yet', 'ns-songs-h')}
        <p class="t-micro fg-2">Yours, credited to ${d.name}, with nothing yet to say which one. They count for none of them until you say.</p>
        <ul class="lib-flist">${songs.map(function (t) {
          return html`<li class="lib-frow">${ui.cover(t, 48)}
            <div class="lib-frow__main"><span class="lib-frow__name">${t.title}</span><span class="lib-frow__meta">${t.album || 'No album'}</span></div>
            <button class="btn btn--sm" data-act="ns-whose" data-name="${d.name}" data-track="${t.id}">Whose is this?</button></li>`;
        })}</ul>
      </section>` : ''}
    </div>`;
  }

  /* "Whose is this?" / "Not by Nirvana?": one of the name's artists, or another of that name */
  function ask(name, title, word) {
    M.api(path(name)).then(function (d) {
      M.absorb(d.entities || {});
      var artists = d.artists.filter(function (id) { return id !== word.from; });
      var dlg = App.openDialog({ title: title, className: 'ns-pick',
        body: html`<fieldset class="disc-rel"><legend class="vh">Whose it is</legend>
          ${artists.map(function (id) {
            var ar = M.artist(id) || { name: name };
            return html`<label class="disc-rel__row"><input type="radio" name="ns-pick" value="${id}">
              <span class="disc-rel__title">This is by ${ar.name}<small>${line(d, id)}</small></span></label>`;
          })}
          <label class="disc-rel__row"><input type="radio" name="ns-pick" value="">
            <span class="disc-rel__title">This is another ${name}<small>One lull doesn't know yet: it gets a page of its own.</small></span></label>
        </fieldset>`,
        foot: html`<button class="btn" data-close>Cancel</button><button class="btn btn--primary" data-act="ns-say">${icon('check')}Save</button>` });
      pending = { name: name, word: word, el: dlg.el };
    }, function (err) { App.toast('Couldn\'t read the artists called ' + name + ': ' + err.message, { kind: 'error' }); });
  }
  var pending = null;
  App.action('ns-say', function () {
    var p = pending, picked = p && $('input[name="ns-pick"]:checked', p.el);
    if (!picked) { App.toast('Pick whose it is first', { icon: 'info' }); return; }
    var body = { albums: p.word.albums || [], tracks: p.word.tracks || [] };
    var to = picked.value ? '/api/artists/place' : '/api/artists/named/split';
    if (picked.value) body.artist = picked.value; else body.name = p.name;
    App.closeTop();
    M.api(to, { method: 'POST', body: body }).then(function () {
      M.invalidate(['/api/artists']);
      App.toast(picked.value ? 'Kept: that\'s whose it is' : 'Kept: another ' + p.name, { confirms: true, icon: 'check', kind: 'ok' });
      App.refresh();
    }, function (err) { App.toast('Couldn\'t keep that: ' + err.message, { kind: 'error' }); });
  });
  App.action('ns-follow-this', function (el) {
    var name = el.getAttribute('data-name'), id = el.getAttribute('data-id');
    M.api('/api/artists/named/choose', { method: 'POST', body: { name: name, artist: id } }).then(function () {
      M.invalidate(['/api/artists', '/api/follows']);
      M.api('/api/follows').then(function (r) {
        var f = {}; (r.artists || []).concat(r.tags || []).forEach(function (x) { f[x] = true; }); App.store.set('followed', f);
      });
      App.toast('Kept: that\'s the ' + name + ' you follow', { confirms: true, icon: 'check', kind: 'ok' });
      App.refresh();
    }, function (err) { App.toast('Couldn\'t keep that: ' + err.message, { kind: 'error' }); });
  });
  App.action('ns-whose', function (el) {
    var t = M.track(el.getAttribute('data-track'));
    ask(el.getAttribute('data-name'), 'Whose is ' + (t ? t.title : 'this song') + '?', { tracks: [el.getAttribute('data-track')] });
  });

  App.namesakes = {
    render: render,
    /* an album's menu: "Not by Nirvana?" */
    album: function (a) { ask(a.artist, 'Whose is ' + a.title + '?', { albums: [a.id], from: a.artistId }); }
  };
}());
