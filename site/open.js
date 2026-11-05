/* lull's "open" page (docs/adr/0032, 0034). Two hosts serve it:
   - GitHub Pages, for links copied before ADR-0034: the item's names and public ids follow `#lull?`, which browsers
     never send to the server;
   - lull's share Worker (share/), for links copied since: the item is in the address, and the Worker hands it to
     this script as JSON (#lull-item), with its cover and public page.
   Either way it opens the item in lull, or adds a song or an album to the listener's queue there (`do=queue`, handed
   to lull beside the link, never in it; lull adds it without playing or leaving the screen it shows):
   - in the listener's lull in the browser, when they told this page its address (kept in this browser only);
   - else in the lull app. Android: the app's buttons are intent URLs, which Chrome opens on one tap (it blocks lull://
     unless a tap asks for it), coming back here with ?app=none when the app isn't installed; App Links, once
     verified, open the app before this page loads. Elsewhere: lull://open?link=…; the desktop app asks "Open lull?";
   - else it offers the item's public page (song.link, album.link, Last.fm) and lull itself.
   For a song or an album the listener picks Open or Add to queue; for an artist (nothing to queue) the page opens it
   on its own, as before: the listener's lull after a moment, or on a computer the app, once per visit.
   No requests leave this page for anywhere else: everything is read from its own address (the cover, on the share
   host, is that host's own). */
(function (root) {
  'use strict';
  var PACKAGE = 'io.github.tannhausercommons.lull';
  /* the link a lull opens: this page's address (its ids and #lull? part included), without the app=none it came
     back with */
  function linkOf(href) { var u = new URL(href); u.searchParams.delete('app'); if (!u.searchParams.toString()) u.search = ''; return u.href; }
  /* where Chrome comes back to when the app isn't installed: this page, saying so */
  function fallbackOf(link) { var u = new URL(link); u.searchParams.set('app', 'none'); return u.href; }
  function isAndroid(ua) { return /\bAndroid\b/i.test(ua || ''); }
  /* what lull is asked to do beside opening (`do=`): only adding to the queue */
  function doPart(action) { return action === 'queue' ? '&do=queue' : ''; }
  /* the lull app's link for `link`: on Android an intent URL (the app's lull://open, its package, and the page to come
     back to), elsewhere lull://open. Each value is encoded whole: the link's own # and ; stay out of the intent's */
  function appUrl(link, android, action) {
    var data = 'open?link=' + encodeURIComponent(link) + doPart(action);
    if (!android) return 'lull://' + data;
    return 'intent://' + data + '#Intent;scheme=lull;package=' + PACKAGE + ';S.browser_fallback_url=' + encodeURIComponent(fallbackOf(link)) + ';end';
  }
  /* the same in the listener's lull in the browser, at `hub` */
  function hubUrl(hub, link, action) { return hub + '/#/open?link=' + encodeURIComponent(link) + doPart(action); }
  var api = { appUrl: appUrl, hubUrl: hubUrl, linkOf: linkOf, fallbackOf: fallbackOf, isAndroid: isAndroid };
  if (typeof module === 'object' && module.exports) module.exports = api;     // tests/web/open-page.test.mjs
  if (typeof document === 'undefined') return;
  root.LullOpen = api;

  var HUB_KEY = 'lull.hub';
  var TRIED_KEY = 'lull.tried-app';
  /* the public pages a link may name; anything else is ignored, so a link can't send people somewhere else */
  var PUBLIC_HOSTS = ['song.link', 'album.link', 'www.last.fm', 'last.fm'];
  var NOUN = { song: 'a song', album: 'an album', artist: 'an artist' };

  var $ = function (id) { return document.getElementById(id); };
  var here = linkOf(location.href);
  var android = isAndroid(navigator.userAgent);
  var item = read(location.hash, $('lull-item'));

  /* the item the share Worker handed over, else the one after #lull? */
  function read(hash, handed) {
    var p;
    if (handed) {
      try { p = JSON.parse(handed.textContent); } catch (e) { return null; }
    } else if (hash.indexOf('#lull?') === 0) {
      var q = new URLSearchParams(hash.slice('#lull?'.length));
      p = { kind: q.get('kind'), artist: q.get('artist'), title: q.get('title'), album: q.get('album'), page: q.get('page') };
    } else return null;
    var kind = p.kind, artist = (p.artist || '').trim(), title = (p.title || '').trim();
    if (!NOUN[kind] || !artist || (kind !== 'artist' && !title)) return null;
    return { kind: kind, artist: artist, title: title, album: (p.album || '').trim(), page: publicPage(p.page), cover: ownPath(p.cover) };
  }
  /* the cover the share host serves itself: a path on this host, never another site's address */
  function ownPath(path) { return typeof path === 'string' && /^\/[^/\\]/.test(path) ? path : null; }
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
  function openIn(hub, action) { location.assign(hubUrl(hub, here, action)); }

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
    $('by').textContent = item.kind === 'artist' ? '' : [item.artist, item.album].filter(Boolean).join(' · ');
    if (item.cover) {
      $('cover').src = item.cover;
      $('cover').alt = item.kind === 'artist' ? item.artist : 'Cover of ' + (item.album || item.title);
      $('cover').hidden = false;
      $('icon').hidden = true;
    }
    document.title = $('title').textContent + ' · open in lull';
    var page = item.page || lastfm(item);
    $('public').href = page;
    /* song.link and album.link list every music app; Last.fm is one page */
    $('public').textContent = /(^|\.)last\.fm$/.test(new URL(page).hostname) ? 'See it on Last.fm' : 'Listen on every music app';
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
    var queueable = item.kind !== 'artist';
    var app = appUrl(here, android);
    $('app').href = app;
    $('app-queue').href = appUrl(here, android, 'queue');
    $('app-queue').hidden = $('queue-submit').hidden = !queueable;
    var hub = origin(stored(HUB_KEY) || '');
    if (hub) {
      $('opening').hidden = false;
      var timer = null;
      if (queueable) {
        $('opening-text').textContent = 'Your lull at ' + new URL(hub).host;
        $('open-there').hidden = $('queue-there').hidden = false;
        $('stay').hidden = true;
        $('open-there').addEventListener('click', function () { openIn(hub); });
        $('queue-there').addEventListener('click', function () { openIn(hub, 'queue'); });
      } else {
        $('opening-text').textContent = 'Opening it in your lull at ' + new URL(hub).host + '…';
        timer = setTimeout(function () { openIn(hub); }, 1200);
      }
      $('other-lull').addEventListener('click', function () { clearTimeout(timer); store(HUB_KEY, null); showChoices(true); });
      $('stay').addEventListener('click', function () { clearTimeout(timer); showChoices(false); });
      return;
    }
    showChoices(false);
    /* Android: one tap on the app's button opens it (Chrome opens no app on its own); back here, it isn't installed */
    if (android) {
      if (new URLSearchParams(location.search).get('app') === 'none') {
        $('no-app').hidden = false;
        $('app').classList.remove('primary');
        $('get').classList.replace('quiet', 'primary');
      }
      return;
    }
    if (queueable) return;
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
    openIn(hub, e.submitter && e.submitter.value);
  });
  /* another link opened in this tab changes only what follows the `#`, which doesn't load the page again */
  window.addEventListener('hashchange', function () { location.reload(); });
  start();
})(typeof self !== 'undefined' ? self : this);
