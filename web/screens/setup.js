/* screens/setup.js.
   Getting a device into the hub (ADR-0002) and the hub ready for music:
   - the pairing gate: opens while the hub answers 401 (App.server.authorized false). GET /api/setup says which:
     an unclaimed hub asks for its setup code (POST /api/setup/claim, the first device), a claimed one for a pair
     code from a paired device (POST /api/pair); #/pair/<code> (the QR code's link) fills the code in;
   - first-run setup (/setup): the library (folder or Jellyfin), the Soulseek account, Last.fm.
     GET /api/setup { claimed, steps: { library, soulseek, lastfm } } says which steps are done.
   Parts come from screens/hub.js (App.hubKit). */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var SETUP = '/api/setup', LASTFM = '/api/lastfm';

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
    M.api(SETUP).then(function (s) { if (gate) gateForm(s.claimed); }, function (err) {
      if (!gate) return;
      $('#setup-gate').innerHTML = String(html`<div class="sys-gate__form"><div class="sys-gate__mark">${ui.wordmark('desktop')}</div>
        <h2 class="sys-gate__title" id="${gate.titleId}">can't reach your hub</h2>
        <p class="sys-gate__text">${err.message}</p>
        <div class="sys-gate__foot"><button class="btn btn--primary btn--block" data-act="setup-gate-retry">Try again</button></div></div>`);
    });
  }
  App.action('setup-gate-retry', function () { if (gate) { gate.close(); gate = null; } openGate(); });
  /* claimed: a pair code (6 characters) from a paired device; not claimed: the hub's setup code (its log) */
  function gateForm(claimed) {
    var code = claimed ? pairCodeFromRoute() : '';
    $('#setup-gate').innerHTML = String(html`<form class="sys-gate__form" novalidate>
      <div class="sys-gate__mark">${ui.wordmark('desktop')}</div>
      <h2 class="sys-gate__title" id="${gate.titleId}">${claimed ? 'pair this device' : 'set up your hub'}</h2>
      <p class="sys-gate__text">${claimed
        ? code ? 'Name this device, then pair it with your hub.' : 'On a device that is already paired, open Settings, Devices, Add a device. Scan its code with this device\'s camera, or type the code here.'
        : 'This hub has no devices yet. Its setup code is in the hub\'s log (or in LULL_SETUP_CODE). Enter it to make this the first device.'}</p>
      <div class="field"><label class="field__label" for="setup-code">${claimed ? 'Pair code' : 'Setup code'}</label>
        <input class="input setup-code" id="setup-code" value="${code}" autocomplete="one-time-code" autocapitalize="characters" spellcheck="false" ${claimed ? raw('maxlength="6"') : ''} ${code ? '' : raw('autofocus')} aria-describedby="setup-err"></div>
      <div class="field"><label class="field__label" for="setup-name">This device's name</label>
        <input class="input" id="setup-name" value="${App.hubKit.deviceName()}" maxlength="60" ${code ? raw('autofocus') : ''}><span class="field__help">You'll see it in Settings, Devices and when you switch where music plays.</span></div>
      <span class="field__error" id="setup-err" hidden>${icon('warning', null, 'ic-16')}<span data-msg></span></span>
      ${App.hubKit.httpsNote()}
      <div class="sys-gate__foot"><button type="submit" class="btn btn--primary btn--block">${claimed ? 'Pair' : 'Set up'}</button></div></form>`);
    var form = $('#setup-gate form'), codeEl = $('#setup-code'), nameEl = $('#setup-name'), err = $('#setup-err'), submit = $('[type="submit"]', form);
    (code ? nameEl : codeEl).focus();
    function showErr(m) { $('[data-msg]', err).textContent = m; err.hidden = !m; codeEl.setAttribute('aria-invalid', m ? 'true' : 'false'); }
    form.addEventListener('submit', function (e) {
      e.preventDefault();
      var c = codeEl.value.trim(), name = nameEl.value.trim() || App.hubKit.deviceName();
      if (!c) { showErr(claimed ? 'Enter the pair code' : 'Enter the setup code'); codeEl.focus(); return; }
      showErr(''); submit.disabled = true;
      var call = claimed ? M.api('/api/pair', { method: 'POST', body: { code: c.toUpperCase(), name: name, kind: 'browser' } })
        : M.api('/api/setup/claim', { method: 'POST', body: { code: c, name: name } });
      call.then(function () {
        gate.close();
        App.start();
        if (claimed) { App.toast('Paired with your hub', { kind: 'ok', icon: 'connect' }); if (pairCodeFromRoute()) App.go('/home', { replace: true }); }
        else App.go('/setup', { replace: true });
      }, function (e2) {
        submit.disabled = false;
        showErr(e2.status === 401 ? (claimed ? 'That code is wrong, used or expired. Make a new one on the paired device.' : 'That isn\'t this hub\'s setup code. Check the hub\'s log.') : e2.message);
        codeEl.select();
      });
    });
  }
  App.on('server', function (srv) { if (srv.authorized === false) openGate(); else if (gate) gate.close(); });

  /* #/pair/<code>: the QR code's link. Unpaired, the gate fills the code in; paired already, say so. */
  App.route('/pair/:code', { title: 'Pair', nav: null, render: function () {
    if (!App.server.checked) return ui.skeleton('card', 1);
    return html`<div class="card" style="max-width:640px">${App.server.authorized
      ? ui.emptyState({ art: 'star', title: 'This device is already paired', text: 'Its pairing with your hub still works, so the code wasn\'t needed. Codes work once; this one is still unused.', action: { label: 'Go home', href: '#/home' } })
      : ui.emptyState({ art: 'slot', title: 'Pairing this device', text: 'Name it in the window above to finish.' })}</div>`;
  } });
  App.on('server', function () { if (/^\/pair\//.test(App.current.path)) App.refresh(); });

  /* =====================================================================
     2. First-run setup (/setup): three steps, each done when GET /api/setup says so
     ===================================================================== */
  function steps() {
    var s = M.peek(SETUP), l = M.peek(LASTFM);
    var st = s && s.steps || {};
    return [!!st.library, !!st.soulseek, !!(st.lastfm || l && l.connected)];
  }
  function lastfmStep() {
    var r = M.get(LASTFM);
    if (r.loading) return ui.skeleton('chip', 1);
    if (r.error) return html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.error.message}</p><button class="btn btn--sm" data-act="reload-read" data-id="${LASTFM}">Try again</button>`;
    if (r.data.connected) return html`<p class="sys-step__done">${icon('check', null, 'ic-16')}Connected as <b>${r.data.user}</b></p>`;
    if (!r.data.configured) return html`<p class="sys-row__help">Last.fm needs an API key and secret first: add them in Settings, Last.fm. You can skip this step.</p>`;
    return html`${r.data.error ? html`<p class="field__error">${icon('warning', null, 'ic-16')}${r.data.error}</p>` : ''}<button class="btn btn--sm" data-act="setup-lastfm">${icon('external')}${r.data.error ? 'Try again' : 'Connect Last.fm'}</button>`;
  }
  /* Last.fm sends the browser back to #/settings/lastfm; during setup that returns here */
  App.action('setup-lastfm', function () { App.storage('setupReturn', true); location.href = M.url('/api/lastfm/connect'); });
  App.on('route', function (r) {
    if (r.path === '/settings/lastfm' && App.storage('setupReturn')) { App.storage('setupReturn', false); App.go('/setup', { replace: true }); }
  });
  function stepItem(i, done, title, text, body) {
    return html`<li class="sys-step ${done ? 'is-done' : ''}"><span class="sys-step__n">${done ? html`${icon('check', null, 'ic-16')}<span class="vh">Done:</span>` : i + 1}</span><div class="sys-step__body">
      <h3 class="sys-step__title">${title}</h3><p class="sys-step__text">${text}</p><div class="sys-step__act setup-step__act">${body}</div></div></li>`;
  }
  function renderSetup() {
    var r = M.get(SETUP);
    if (r.loading) return ui.skeleton('card', 1);
    if (r.error) return App.errorCard(r.error, { path: SETUP, title: 'Couldn\'t read what setup still needs' });
    var done = steps(), n = done.filter(Boolean).length;
    return html`<div class="sys-page sys-welcome setup">
      ${ui.sparkle('--s:22px; right:6%; top:18px')}
      <header class="sys-welcome__head"><h1 class="t-display">welcome to lull</h1>
        <p class="sys-lede">Songs stream from Soulseek while they download, and what you keep lands in your own library. Three steps get your hub ready.</p></header>
      <section class="card sys-welcome__card" aria-labelledby="setup-h">
        ${ui.labelRule('get started', { id: 'setup-h' })}
        <ol class="sys-steps">
          ${stepItem(0, done[0], 'Choose your library', 'A music folder on the hub, or your Jellyfin server.', App.hubKit.librarySource({ withFolder: true }))}
          ${stepItem(1, done[1], 'Sign in to Soulseek', 'The account lull searches and downloads with.', App.hubKit.soulseekForm())}
          ${stepItem(2, done[2], 'Connect Last.fm', 'Scrobble what you play, and let your history shape recommendations. Optional.', html`<div id="setup-lastfm">${lastfmStep()}</div>`)}
        </ol>
        <footer class="sys-welcome__foot"><span class="fg-2" aria-live="polite">${n} of 3 done</span><span class="spacer"></span>
          ${n < 3 ? html`<a class="btn btn--quiet" href="#/home">Skip for now</a>` : ''}<a class="btn btn--primary" href="#/home">Start listening</a></footer>
      </section></div>`;
  }
  /* a step finishes on the hub (a restart, a switch, a sign-in): ask again what setup needs */
  function refreshSteps() { if (App.current.path === '/setup') M.reload(SETUP); }
  App.route('/setup', { render: renderSetup, title: 'Setup', nav: 'home', mount: function (ctx) {
    ctx.onCleanup(App.on('server:hub.restarting', refreshSteps));
    ctx.onCleanup(App.on('server:service', function (e) { if (e.name === 'soulseek') refreshSteps(); }));
  } });
})();
