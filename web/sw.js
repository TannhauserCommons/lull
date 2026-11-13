/* lull service worker: keeps the app shell so the installed app starts offline.
   Network first for the UI's own files (they are served with ETags and no-cache, so online is always current),
   the cache only when the network fails. /api and /health are never cached: they are live data. Covers of
   downloaded songs (offline.js keeps them in the 'lull-downloads' cache) answer from there when the hub can't.
   One cache per server version (sw.js?v=<version>); the page lists what it loaded so the cache holds exactly
   the files the chosen design needs, and the page installs a new version's worker when the listener accepts the update toast (app.js registerWorker). */
'use strict';
var CACHE = 'lull-shell-' + (new URL(self.location.href).searchParams.get('v') || '0');

self.addEventListener('install', function (e) {
  e.waitUntil(caches.open(CACHE).then(function (c) { return c.addAll(['./']); }));
});

self.addEventListener('activate', function (e) {
  e.waitUntil(caches.keys().then(function (keys) {
    return Promise.all(keys.filter(function (k) { return k.indexOf('lull-shell-') === 0 && k !== CACHE; })
      .map(function (k) { return caches.delete(k); }));
  }).then(function () { return self.clients.claim(); }));
});

self.addEventListener('message', function (e) {
  var m = e.data || {};
  if (m.type === 'skip-waiting') self.skipWaiting();
  if (m.type === 'cache' && Array.isArray(m.urls)) {
    e.waitUntil(caches.open(CACHE).then(function (c) {
      return Promise.all(m.urls.filter(shell).map(function (u) { return c.add(u).catch(function () {}); }));
    }));
  }
});

/* a downloaded song's cover without the hub: the size asked when kept, else the nearest kept size (downloads keep 96,
   256 and 512; the page may ask for any size up to 2048, web.md "Covers at the size they are drawn") */
function keptCover(url) {
  url = url.replace(/&retry=1$/, '');   // the page's second try of a cover
  var asked = +((/[?&]size=(\d+)/.exec(url) || [])[1] || 0);
  var sizes = [asked, 512, 256, 96].filter(function (s, i, all) { return s && all.indexOf(s) === i; });
  return caches.open('lull-downloads').then(function (c) {
    return sizes.reduce(function (found, s) {
      return found.then(function (hit) { return hit || c.match(url.replace(/([?&]size=)\d+/, '$1' + s)); });
    }, Promise.resolve(null));
  }).then(function (hit) { return hit || Response.error(); });
}

/* the UI's own files: same origin, not the API, not health */
function shell(url) {
  var u = new URL(url, self.location.href);
  if (u.origin !== self.location.origin) return false;
  var path = u.pathname.slice(new URL('./', self.location.href).pathname.length);
  return !/^(api\/|api$|health$)/.test(path);
}

self.addEventListener('fetch', function (e) {
  var req = e.request;
  if (req.method === 'GET' && /\/api\/art\//.test(req.url)) {
    e.respondWith(fetch(req).catch(function () { return keptCover(req.url); }));
    return;
  }
  if (req.method !== 'GET' || !shell(req.url)) return;
  e.respondWith(fetch(req).then(function (res) {
    if (res.ok && res.type === 'basic') {
      var copy = res.clone();
      caches.open(CACHE).then(function (c) { c.put(req, copy); });
    }
    return res;
  }, function () {
    return caches.match(req).then(function (hit) {
      return hit || (req.mode === 'navigate' ? caches.match('./') : Response.error());
    });
  }));
});
