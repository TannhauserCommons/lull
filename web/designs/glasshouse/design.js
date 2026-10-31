/* glasshouse design slots (classic script, slot overrides only).
   brand     "lull" in chrome serif with a typewriter caret and an aero bead.
   homeLead  Home's first object: an AIMP-style media window (img 03): LCD deck on the left, the current song + up next
             as a numbered list with mono format lines on the right, a header strip (Tracks / Duration / Size) and a
             scrolling ticker on the bottom edge.
   emptyArt  a tiny aero window whose LCD says what is missing ("0 liked songs_"), instead of a line drawing.
   nowHero   the default device re-dressed as the media window: DARK / LIGHT skin tabs on its title edge and a ticker
             footer (title, format, key hints). Every data-now / data-act / data-bind hook is the default markup's own
             or the shared live hooks, so now.js and app.js keep driving it. Compact variant = default. */
(function () {
  'use strict';

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }
  function icon(name) { return window.icon ? window.icon(name) : ''; }

  /* skin tabs: the current skin is plain text, the other one a button on the shared "theme" action. CSS shows the
     right pair for :root[data-theme], so the tabs stay correct without a re-render. */
  var SKIN = '<span class="gh-skin" role="group" aria-label="Theme">' +
    '<span class="gh-skin__tab gh-skin__cur gh-skin--dark">dark<span class="vh"> (current theme)</span></span>' +
    '<button class="gh-skin__tab gh-skin__btn gh-skin--dark" data-act="theme" data-tip="Dark theme">dark<span class="vh"> theme</span></button>' +
    '<span class="gh-skin__tab gh-skin__cur gh-skin--light">light<span class="vh"> (current theme)</span></span>' +
    '<button class="gh-skin__tab gh-skin__btn gh-skin--light" data-act="theme" data-tip="Light theme">light<span class="vh"> theme</span></button>' +
    '</span>';

  /* ticker: two identical runs so a translateX(-50%) loop is seamless (design.css; static under reduced motion).
     Decorative (aria-hidden): every fact in it is also on screen as real text. */
  function ticker(cls, inner) {
    var run = '<span class="gh-ticker__run">.:: ' + inner + ' ::.</span>';
    return '<div class="gh-ticker ' + cls + '" aria-hidden="true"><div class="gh-ticker__track">' + run + run + '</div></div>';
  }
  var NOW_LINE = '<span data-now="artist"></span> - <span data-now="title"></span> <i>::</i> <span data-now="format"></span>';

  function brand() {
    return '<span class="wordmark brand gh-brand" aria-hidden="true"><span class="brand__name">' + esc(Designs.product) +
      '</span><span class="gh-brand__caret"></span><span class="brand__orn gh-brand__bead"></span></span>';
  }

  function homeLead(ctx) {
    var t = ctx.track, F = window.App && App.fmt;
    if (!t || !F) return '';
    var p = ctx.parts, list = [t].concat(ctx.queue || []).slice(0, 8);
    var secs = 0, mb = 0;
    list.forEach(function (x) { secs += x.duration || 0; mb += x.sizeMB || 0; });
    var line = esc(t.artist) + ' - ' + esc(t.title) + ' <i>::</i> ' + esc(p.formatLine(t));
    return '<section class="card gh-aimp" aria-labelledby="gh-aimp-h">' +
      '<div class="card-head gh-aimp__bar"><h2 class="card-head__title" id="gh-aimp-h">' + esc(Designs.product) + ' :: now spinning</h2>' + SKIN + '</div>' +
      '<div class="gh-aimp__deck">' +
        '<div class="gh-aimp__lcd">' + p.cover(t, 152) +
          '<div class="gh-aimp__meta"><p class="gh-aimp__title">' + esc(t.title) + '</p>' +
          '<p class="gh-aimp__artist">' + esc(t.artist) + '</p>' +
          '<p class="gh-aimp__fmt">' + esc(p.formatLine(t)) + '</p></div>' +
          '<p class="gh-aimp__time"><b data-bind="pos">0:00</b> / <span data-bind="dur">0:00</span></p>' +
        '</div>' +
        '<div class="gh-aimp__keys">' +
          '<button class="btn-icon" data-act="prev" aria-label="Previous" data-tip="Previous (P)">' + icon('prev') + '</button>' +
          p.playButton(56) +
          '<button class="btn-icon" data-act="next" aria-label="Next" data-tip="Next (N)">' + icon('next') + '</button>' +
          '<span class="gh-aimp__like">' + p.likeButton(t) + '</span>' +
        '</div>' +
      '</div>' +
      '<div class="gh-aimp__list">' +
        '<p class="gh-aimp__strip"><span>Tracks: ' + list.length + '</span><span>Duration: ' + F.clock(secs) + '</span>' +
          '<span>Size: ' + F.size(mb) + '</span><a class="gh-aimp__open" href="#/now">open player</a></p>' +
        p.trackList(list, { label: 'Now spinning and up next', showSources: false }) +
      '</div>' +
      ticker('gh-aimp__ticker', '<b data-bind="pos">0:00</b> / <span data-bind="dur">0:00</span> <i>::</i> ' + line +
        ' <i>::</i> next: ' + esc(list[1] ? list[1].title : 'end of queue')) +
    '</section>';
  }

  /* empty / error LCD: "0 liked songs" from "No liked songs yet"; a fixed word per art otherwise */
  var EMPTY_WORD = { moon: 'all quiet', slot: 'empty slot', star: 'nothing yet', wave: 'silence' };
  function emptyArt(ctx) {
    var title = String(ctx.title || ''), m = /^no ([a-z ]+?)(?: yet| here| right now)?$/i.exec(title);
    var err = /couldn|unavailable|^that |snag|error/i.test(title);
    var text = err ? 'no signal' : m ? '0 ' + m[1].toLowerCase() : EMPTY_WORD[ctx.art] || EMPTY_WORD.moon;
    return '<span class="gh-empty' + (err ? ' gh-empty--err' : '') + '"><span class="gh-empty__bar"></span>' +
      '<span class="gh-empty__lcd"><span class="gh-empty__txt">' + esc(text) + '</span><i class="gh-caret"></i></span></span>';
  }

  var HINT = '<b>P</b>/<b>N</b> skip <i>::</i> <b>&uarr;</b><b>&darr;</b> vol <i>::</i> <b>space</b> play';
  function nowHero(ctx, fallback) {
    var markup = String(fallback(ctx));
    if (ctx.variant !== 'full') return markup;
    var host = document.createElement('div');
    host.innerHTML = markup;
    var dev = host.querySelector('.device');
    if (!dev) return markup;
    dev.classList.add('gh-np');
    var led = dev.querySelector('.device__led');
    if (led) led.insertAdjacentHTML('afterend', SKIN);
    var foot = ticker('gh-np__ticker', NOW_LINE + ' <i>::</i> ' + HINT);
    var grill = dev.querySelector('.device__grill');
    if (grill) grill.outerHTML = foot; else dev.insertAdjacentHTML('beforeend', foot);
    return host.innerHTML;
  }

  Designs.define('glasshouse', { brand: brand, homeLead: homeLead, emptyArt: emptyArt, nowHero: nowHero });
})();
