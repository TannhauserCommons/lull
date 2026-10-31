/* lull design slots (classic script).
   brand: the glossy "lull" pill.
   nowHero: "lull pocket", a glossy media-player window built from the default device's own live parts (screen, LED,
     A/B keys, prev/next keys, pills), so every data-now / data-act hook keeps working; only the arrangement and the
     skin change. The D-pad's volume keys stay with the mini player (the dock has the volume). Compact = default.
   emptyArt: a little lull pocket with an empty screen (the glyph says what is missing).
   coverArt: extra motifs (CD sheen, cassette, checker sticker, pink-dusk stripes, crescent night, moon night) for the
     orb / night / stripe / heart covers, picked from the item key so a cover never changes between screens.
   homeLead: desk widgets for wide screens (img 02): a "today" sticker, the pocket mini and "this week"; CSS shows them
     on the wallpaper beside the discover window at >= 1700 px and hides them elsewhere. */
(function () {
  'use strict';

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; });
  }

  function brand() {
    return '<span class="wordmark lull-wm" aria-hidden="true">lull<span class="wordmark__spark"></span></span>';
  }

  function nowHero(ctx, fallback) {
    if (ctx.variant !== 'full') return fallback(ctx);
    var src = document.createElement('div');
    src.innerHTML = String(ctx.parts.device());
    function pick(sel) { var n = src.querySelector(sel); return n ? n.outerHTML : ''; }
    var t = ctx.track;
    var art = t && window.App ? String(App.ui.cover(t, '100%')) : '<span class="now-dev__slot" aria-hidden="true"></span>';
    return '<div class="lull-np" role="group" aria-label="Player">' +
      '<div class="lull-win__bar lull-np__bar"><span aria-hidden="true">now playing</span>' + pick('.device__led') + '</div>' +
      '<div class="lull-np__shell">' +
        '<div class="lull-np__screen device__screen s-screen">' +
          '<span class="lull-np__art" data-now="c-art">' + art + '</span>' +
          pick('.device__page-head') + pick('[data-now="screen"]') + pick('[data-now="osd"]') +
        '</div>' +
        '<div class="lull-np__foot">' +
          '<span class="lull-np__mark" aria-hidden="true">lull <em>pocket</em></span>' +
          '<span class="lull-np__state">' + pick('.device__led') + '</span>' +   /* phone: the title bar's status pill moves here */
          '<span class="lull-np__grill" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></span>' +
        '</div>' +
        '<div class="lull-np__deck">' + pick('.device__pill--shuffle') +
          '<span class="lull-np__transport">' + pick('.dpad__btn--left') + pick('.device__key--a') + pick('.dpad__btn--right') + '</span>' +
          pick('.device__pill--repeat') +
        '</div>' +
      '</div>' +
      /* the like orb sits beside the big title under the pocket, placed there by CSS */
      '<span class="lull-np__like">' + pick('.device__key--b') + '</span>' +
    '</div>';
  }

  /* screen glyphs, dashed: moon (nothing yet), slot (no cartridge), star (a heart: nothing liked), wave (silence) */
  var GLYPH = {
    moon: '<path class="lull-empty__glyph" d="M60 20a9 9 0 1 0 6 15 7 7 0 0 1-6-15z"/>',
    slot: '<rect class="lull-empty__glyph" x="45" y="19" width="22" height="16" rx="2"/>',
    star: '<path class="lull-empty__glyph" d="M56 37s-9-5.5-9-11.5a4.6 4.6 0 0 1 9-1.5 4.6 4.6 0 0 1 9 1.5C65 31.5 56 37 56 37z"/>',
    wave: '<path class="lull-empty__glyph" d="M43 28h5l3-6 4 12 4-9 3 3h7"/>'
  };
  function emptyArt(ctx) {
    return '<svg class="lull-empty" viewBox="0 0 112 88" aria-hidden="true" focusable="false">' +
      '<path class="lull-empty__shell" d="M36 3h40a8 8 0 0 1 8 8v54a20 20 0 0 1-20 20H36a8 8 0 0 1-8-8V11a8 8 0 0 1 8-8z"/>' +
      '<rect class="lull-empty__screen" x="35" y="10" width="42" height="34" rx="5"/>' + (GLYPH[ctx.art] || GLYPH.moon) +
      '<circle class="lull-empty__key" cx="42" cy="61" r="4.5"/><circle class="lull-empty__play" cx="56" cy="61" r="7.5"/><circle class="lull-empty__key" cx="70" cy="61" r="4.5"/>' +
      '<circle class="lull-empty__grill" cx="66" cy="76" r="1.3"/><circle class="lull-empty__grill" cx="71" cy="74" r="1.3"/><circle class="lull-empty__grill" cx="76" cy="71" r="1.3"/>' +
      '</svg><span class="sparkle"></span>';
  }

  /* recipe -> how many looks it has (the plain one plus lull-alt, lull-alt2) */
  var ALT = { c1: 2, c5: 2, c4: 3, c6: 3 };
  function coverArt(ctx, fallback) {
    var out = String(fallback(ctx)), n = ALT[ctx.recipe];
    if (!n || !ctx.key) return out;
    var h = 0, k = String(ctx.key);
    for (var i = 0; i < k.length; i++) h = (h * 31 + k.charCodeAt(i)) | 0;
    var v = ((h % n) + n) % n;
    return v ? out.replace('class="cover ', 'class="cover lull-alt' + (v === 2 ? '2' : '') + ' ') : out;
  }

  var DAYS = ['sunday', 'monday', 'tuesday', 'wednesday', 'thursday', 'friday', 'saturday'];
  var MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december'];
  function homeLead(ctx) {
    var t = ctx.track, p = ctx.parts, d = new Date();
    var today = '<section class="lull-desk__win lull-desk__today" aria-label="Today">' +
      '<div class="lull-win__bar"><span aria-hidden="true">today</span></div>' +
      '<p class="lull-desk__date"><b>' + d.getDate() + '</b><span>' + MONTHS[d.getMonth()] + '<br>' + DAYS[d.getDay()] + '</span></p></section>';
    /* the screen keeps only the codec and rate ("FLAC :: 44.1 kHz"); the full line wraps edge to edge */
    var fmt = t ? String(p.formatLine(t)).split(',')[0] : '';
    var pocket = t ? '<section class="lull-desk__win lull-desk__pocket" aria-label="Pocket player">' +
      '<div class="lull-win__bar"><span aria-hidden="true">lull pocket</span></div>' +
      '<div class="lull-desk__screen">' + p.cover(t, '64px') +
        '<span class="lull-desk__meta"><b>' + esc(t.title) + '</b><span>' + esc(t.artist) + '</span>' + (fmt ? '<small>' + esc(fmt) + '</small>' : '') + '</span></div>' +
      '<div class="lull-desk__keys">' + p.likeButton(t) + p.playButton(48) + '</div></section>' : '';
    return '<div class="lull-desk">' + today + pocket + week() + '</div>';
  }
  /* "this week": minutes and plays plus a 24-hour listening sparkline, linking to Stats. The queue window beside it
     already lists what is up next. Reads the 7-day stats the Stats page uses; skipped when they are not there. */
  function week() {
    var D = window.App && App.data && App.data.stats && App.data.stats.byPeriod && App.data.stats.byPeriod['7d'];
    if (!D || !D.clock || !D.clock.length) return '';
    var max = Math.max.apply(null, D.clock) || 1, peak = D.clock.indexOf(max);
    var bars = D.clock.map(function (v, h) {
      return '<i style="--v:' + (Math.round(v / max * 100) / 100) + '"' + (h === peak ? ' class="is-peak"' : '') + '></i>';
    }).join('');
    var hrs = Math.floor(D.minutes / 60), min = D.minutes % 60;
    return '<section class="lull-desk__win lull-desk__week" aria-label="This week">' +
      '<div class="lull-win__bar"><span aria-hidden="true">this week</span></div>' +
      '<p class="lull-desk__nums"><b>' + hrs + ' h ' + min + ' min</b><span>' + D.plays + ' plays, ' + D.artists + ' artists</span></p>' +
      '<div class="lull-desk__spark" role="img" aria-label="Listening by hour of day, most around ' + peak + ':00">' + bars + '</div>' +
      '<p class="lull-desk__axis" aria-hidden="true"><span>0:00</span><span>12:00</span><span>23:00</span></p>' +
      '<a class="btn btn--sm lull-desk__more" href="#/stats">See stats</a></section>';
  }

  Designs.define('lull', { brand: brand, nowHero: nowHero, emptyArt: emptyArt, coverArt: coverArt, homeLead: homeLead });
})();
