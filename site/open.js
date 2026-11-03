/* lull's "open" page (docs/adr/0032): a shared lull link is this page with the item's names and public ids after
   `#lull?`, which browsers never send to the server. It opens the item in lull:
   - in the listener's lull in the browser, when they told this page its address (kept in this browser only);
   - else in the lull app, through lull://open?link=… (the desktop app asks "Open lull?"; Android's App Links open the
     app before this page loads);
   - else it offers the item's public page (song.link, album.link, Last.fm) and lull itself.
   No requests leave this page: everything is read from its own address. */
(function () {
  'use strict';
  var HUB_KEY = 'lull.hub';
  var TRIED_KEY = 'lull.tried-app';
  /* the public pages a link may name; anything else is ignored, so a link can't send people somewhere else */
  var PUBLIC_HOSTS = ['song.link', 'album.link', 'www.last.fm', 'last.fm'];
  var NOUN = { song: 'a song', album: 'an album', artist: 'an artist' };

  var $ = function (id) { return document.getElementById(id); };
  var here = location.href;
  var item = read(location.hash);

  function read(hash) {
    if (hash.indexOf('#lull?') !== 0) return null;
    var p = new URLSearchParams(hash.slice('#lull?'.length));
    var kind = p.get('kind'), artist = (p.get('artist') || '').trim(), title = (p.get('title') || '').trim();
    if (!NOUN[kind] || !artist || (kind !== 'artist' && !title)) return null;
    return { kind: kind, artist: artist, title: title, page: publicPage(p.get('page')) };
  }
  function publicPage(url) {
    try {
      var u = new URL(url);
      return u.protocol === 'https:' && PUBLIC_HOSTS.indexOf(u.hostname) >= 0 ? u.href : null;
    } catch (e) { return null; }
  }
  function lastfm(it) {
    var path = '/music/' + encodeURIComponent(it.artist);
    if (it.kind === 'song') path += '/_/' + encodeURIComponent(it.title);
    if (it.kind === 'album') path += '/' + encodeURIComponent(it.title);
    return 'https://www.last.fm' + path;
  }
  function store(key, value) {
    try { if (value === null) localStorage.removeItem(key); else localStorage.setItem(key, value); } catch (e) { /* private window */ }
  }
  function stored(key) { try { return localStorage.getItem(key); } catch (e) { return null; } }
  /* what the listener typed, as an origin: a bare host means https */
  function origin(text) {
    text = text.trim();
    if (!text) return null;
    try {
      var u = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(text) ? text : 'https://' + text);
      return /^https?:$/.test(u.protocol) && u.hostname ? u.origin : null;
    } catch (e) { return null; }
  }
  function openIn(hub) { location.assign(hub + '/#/open?link=' + encodeURIComponent(here)); }

  function describe() {
    if (!item) {
      $('kind').textContent = 'lull';
      $('title').textContent = 'This link names nothing to open';
      $('by').textContent = 'It may have been cut short when it was copied.';
      $('public').hidden = true;
      return;
    }
    $('kind').textContent = NOUN[item.kind] + ' shared from lull';
    $('title').textContent = item.kind === 'artist' ? item.artist : item.title;
    $('by').textContent = item.kind === 'artist' ? '' : item.artist;
    document.title = $('title').textContent + ' · open in lull';
    $('public').href = item.page || lastfm(item);
    $('public').textContent = item.page ? 'Listen on every music app' : 'See it on Last.fm';
  }

  function showChoices(withForm) {
    $('opening').hidden = true;
    $('choices').hidden = false;
    $('hub-form').hidden = !withForm;
    if (withForm) $('hub').focus();
  }

  function start() {
    describe();
    if (!item) return;
    var app = 'lull://open?link=' + encodeURIComponent(here);
    $('app').href = app;
    var hub = origin(stored(HUB_KEY) || '');
    if (hub) {
      $('opening').hidden = false;
      $('opening-text').textContent = 'Opening it in your lull at ' + new URL(hub).host + '…';
      var timer = setTimeout(function () { openIn(hub); }, 1200);
      $('other-lull').addEventListener('click', function () { clearTimeout(timer); store(HUB_KEY, null); showChoices(true); });
      $('stay').addEventListener('click', function () { clearTimeout(timer); showChoices(false); });
      return;
    }
    showChoices(false);
    /* once per visit: coming back to this page (Back from the app's prompt) doesn't ask again */
    var tried = false;
    try { tried = sessionStorage.getItem(TRIED_KEY) === here; sessionStorage.setItem(TRIED_KEY, here); } catch (e) { /* private window */ }
    /* after the page has loaded: a page still loading would wait on the app's prompt */
    if (!tried) window.addEventListener('load', function () { location.href = app; });
  }

  $('browser').addEventListener('click', function () { showChoices(true); });
  $('hub-form').addEventListener('submit', function (e) {
    e.preventDefault();
    var hub = origin($('hub').value);
    if (!hub) {
      $('hub-error').textContent = 'That isn\'t an address. It looks like https://lull.example.ts.net.';
      $('hub').setAttribute('aria-invalid', 'true');
      return;
    }
    store(HUB_KEY, hub);
    openIn(hub);
  });
  /* another link opened in this tab changes only what follows the `#`, which doesn't load the page again */
  window.addEventListener('hashchange', function () { location.reload(); });
  start();
})();
