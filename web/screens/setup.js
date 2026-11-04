/* screens/setup.js.
   Getting a device into the hub (ADR-0002) and the hub ready for music:
   - the pairing gate: opens while the hub answers 401 (App.server.authorized false). GET /api/setup says which:
     an unclaimed hub asks for its setup code (POST /api/setup/claim, the first device), a claimed one for a pair
     code from a paired device (POST /api/pair); #/pair/<code> (the QR code's link) fills the code in;
   - first-run setup (/setup): the library (folder or Jellyfin), the Soulseek account, Last.fm.
     GET /api/setup { claimed, steps: { library, soulseek, lastfm } } says which steps are done.
   The gate's form and the setup page come from screens/hub.js (App.hubKit.gateForm, .setupPage), which loads on use
   (App.needHub): the gate asks for it as it opens, alongside its own read, and /setup on entry, so a paired device that
   is set up never fetches them. What stays here, from the start: opening the gate, the routes, library.source events. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, $ = App.$;
  var SETUP = '/api/setup';

  /* =====================================================================
     1. Pairing gate
     ===================================================================== */
  var gate = null;
  function pairCodeFromRoute() { var m = /^\/pair\/([^/?]+)/.exec(App.current.path); return m ? decodeURIComponent(m[1]).toUpperCase() : ''; }
  function openGate() {
    if (gate) return;
    var titleId = App.uid('setup-gate-t');
    gate = App.openDialog({ className: 'sys-gate', closable: false, dismissible: false, label: 'Pair with your hub',
      onClose: function () { gate = null; },
      body: html`<div id="setup-gate">${ui.skeleton('text', 3)}</div>`,
      onMount: function (el) { el.setAttribute('aria-labelledby', titleId); el.removeAttribute('aria-label'); } });
    gate.titleId = titleId;
    Promise.all([M.api(SETUP), App.needHub()]).then(function (r) { if (gate) App.hubKit.gateForm(gate, r[0].claimed, pairCodeFromRoute()); }, function (err) {
      if (!gate) return;
      $('#setup-gate').innerHTML = String(html`<div class="sys-gate__form"><div class="sys-gate__mark">${ui.wordmark('desktop')}</div>
        <h2 class="sys-gate__title" id="${gate.titleId}">can't reach your hub</h2>
        <p class="sys-gate__text">${err.message}</p>
        <div class="sys-gate__foot"><button class="btn btn--primary btn--block" data-act="setup-gate-retry">Try again</button></div></div>`);
    });
  }
  App.action('setup-gate-retry', function () { if (gate) { gate.close(); gate = null; } openGate(); });
  App.on('server', function (srv) { if (srv.authorized === false) openGate(); else if (gate) gate.close(); });

  /* #/pair/<code>: the QR code's link. Unpaired, the gate fills the code in; paired already, say so. */
  App.route('/pair/:code', { title: 'Pair', nav: null, render: function () {
    if (!App.server.checked) return ui.skeleton('card', 1);
    return html`<div class="card" style="max-width:640px">${App.server.authorized
      ? ui.emptyState({ art: 'star', title: 'This device is already paired', text: 'Its pairing with your hub still works, so the code wasn\'t needed. Codes work once; this one is still unused.', action: { label: 'Go home', href: '#/home' } })
      : ui.emptyState({ art: 'slot', title: 'Pairing this device', text: 'Name it in the window above to finish.' })}</div>`;
  } });
  App.on('server', function () { if (/^\/pair\//.test(App.current.path)) App.refresh(); });

  /* the library switching source (folder or Jellyfin): its reads are fetched again once it has finished, whether or not
     screens/hub.js is here yet; hub.js shows the switch's progress, from the latest event when it loads mid-switch */
  App.librarySource = null;
  App.on('server:library.source', function (e) {
    App.librarySource = e;
    if (e.state !== 'switching') { M.reload('/api/library/source'); M.reload(SETUP); }
  });

  /* =====================================================================
     2. First-run setup (/setup): three steps, each done when GET /api/setup says so
     ===================================================================== */
  /* Last.fm sends the browser back to #/settings/lastfm; during setup that returns here */
  App.action('setup-lastfm', function () { App.storage('setupReturn', true); location.href = M.url('/api/lastfm/connect'); });
  App.on('route', function (r) {
    if (r.path === '/settings/lastfm' && App.storage('setupReturn')) { App.storage('setupReturn', false); App.go('/setup', { replace: true }); }
  });
  /* a step finishes on the hub (a restart, a switch, a sign-in): ask again what setup needs */
  function refreshSteps() { if (App.current.path === '/setup') M.reload(SETUP); }
  App.route('/setup', { need: function () { return App.hubKit ? null : App.needHub(); }, render: function () { return App.hubKit.setupPage(); }, title: 'Setup', nav: 'home', mount: function (ctx) {
    App.storage('setupOffered', true);
    ctx.onCleanup(App.on('server:hub.restarting', refreshSteps));
    ctx.onCleanup(App.on('server:service', function (e) { if (e.name === 'soulseek') refreshSteps(); }));
  } });
  /* A hub without a library or a Soulseek account opens setup once per device, on its first Home: the claim above
     goes there itself, but the desktop app claims the hub it starts on this computer and opens Home directly. */
  var offering = false;
  App.on('server', function (srv) {
    if (srv.authorized !== true || offering || App.storage('setupOffered') || App.current.path !== '/home') return;
    offering = true;
    M.api(SETUP).then(function (s) {
      App.storage('setupOffered', true);
      if ((!s.steps.library || !s.steps.soulseek) && App.current.path === '/home') App.go('/setup', { replace: true });
    }, function () { offering = false; });
  });
})();
