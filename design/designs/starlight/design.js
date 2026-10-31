/* starlight design: slot overrides only (classic script, loaded by designs/designs.js).
   brand      = sparkle + "LULL" as outlines baked from Poiret One caps (SIL OFL 1.1, see fonts/OFL-PoiretOne.txt; the font
                itself is not shipped), with a second small sparkle tucked above the last L, as img 01 does with its title.
   coverArt   = the default span plus a per-item variant (mirror, hue shift, crop) seeded from the cover key, so two albums
                that share a scene never look identical side by side. design.css draws the scene from the variables.
   emptyArt   = a small dotted constellation per empty state (a heart for liked songs).
   homeLead   = "your queue as a constellation": the playing song is the 4-point sparkle, the next songs are stars joined by
                a dashed line, drawn on the sky beside the greeting. Decorative (the queue panel lists the same songs);
                each star carries its song in a hover title.
   nowHero    = the img 01 top card: label + rule, giant caps title, caps artist + time, seek line with a dot, outlined
                transport row, art panel on the right. A failed play swaps the seek line and transport for a burgundy
                failure line and its fixes (shown by design.css from .pbar[data-status]). Live parts keep the now.js hooks
                (data-now="c-screen" / "c-art", .now-head__from, data-bind="play|shuffle|repeat|like|pos|dur").
   miniPlayer = the desktop mini window as a small portrait version of that card (device style); the strip keeps its
                markup and is restyled in design.css. Uses only ctx, window.icon and Designs. */
(function () {
  'use strict';
  /* "LULL", Poiret One caps, tracking .32em; viewBox 0 -16 2637 782 */
  var WORD = 'M108 0V721H340V750H75V0ZM984 764H974Q882 764 818 700Q755 636 755 539V0H788V538Q788 623 843 679Q898 735 979 735Q1060 735 1114 680Q1169 624 1169 538V0H1202V537Q1202 635 1138 700Q1075 764 984 764ZM1705 0V721H1937V750H1672V0ZM2385 0V721H2617V750H2352V0Z';
  var SPARK = 'M12 0C12.9 8.4 15.6 11.1 24 12C15.6 12.9 12.9 15.6 12 24C11.1 15.6 8.4 12.9 0 12C8.4 11.1 11.1 8.4 12 0Z';

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; });
  }
  function ic(name, cls) { return window.icon ? String(window.icon(name, null, cls)) : ''; }
  function btn(act, label, iconName, extra) {
    return '<button class="btn-icon btn-icon--bare" data-act="' + act + '" aria-label="' + label + '" data-tip="' + label + '"' +
      (extra || '') + '>' + ic(iconName) + '</button>';
  }
  function hash(s) {                      /* FNV-1a, stable per key */
    var h = 2166136261;
    for (var i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
    return h >>> 0;
  }

  function brand() {
    return '<span class="wordmark sl-mark" aria-hidden="true">' +
      '<svg class="sl-mark__spark" viewBox="0 0 24 24"><path d="' + SPARK + '"/></svg>' +
      '<svg class="sl-mark__word" viewBox="0 -16 2637 782"><path d="' + WORD + '"/>' +
      '<path class="sl-mark__tiny" transform="translate(2575 -96) scale(9)" d="' + SPARK + '"/></svg></span>';
  }

  /* cover variant: mirror (1 bit), hue shift (5 steps), crop zoom (3 steps) and where the crop sits (3 x 3) */
  function coverArt(ctx, fallback) {
    var h = hash(String(ctx.key)), z = [1, 1.25, 1.45][(h >>> 4) % 3];
    function shift(bits) { return Math.round((((h >>> bits) % 3) - 1) * (z - 1) * 50) + '%'; }
    var vars = '--sl-flip:' + ((h & 1) ? -1 : 1) + ';--sl-hue:' + (((h >>> 1) % 5) - 2) * 14 + 'deg;--sl-zoom:' + z +
      ';--sl-dx:' + shift(6) + ';--sl-dy:' + shift(8);
    var m = String(fallback(ctx));       /* mosaic cells have no style attribute */
    return m.indexOf('style="') > 0 ? m.replace('style="', 'style="' + vars + ';') : m.replace('<span ', '<span style="' + vars + '" ');
  }

  /* empty states: a dotted constellation; points in a 120 x 80 box, the sparkle on one of them */
  var SHAPES = {
    heart: [[60, 72], [30, 46], [24, 24], [42, 12], [60, 26], [78, 12], [96, 24], [90, 46], [60, 72]],
    moon:  [[22, 60], [34, 34], [56, 18], [82, 16], [100, 30]],
    slot:  [[18, 26], [40, 18], [58, 30], [74, 26], [84, 50], [106, 56], [100, 30], [84, 50]],
    wave:  [[12, 50], [32, 30], [52, 50], [72, 30], [92, 50], [108, 34]],
    star:  [[14, 26], [36, 52], [60, 34], [84, 56], [106, 22]]
  };
  function emptyArt(ctx) {
    var kind = /liked/i.test(ctx.title || '') ? 'heart' : SHAPES[ctx.art] ? ctx.art : 'moon', pts = SHAPES[kind];
    var line = pts.map(function (p) { return p.join(','); }).join(' '), spark = pts[kind === 'heart' ? 4 : pts.length >> 1];
    var dots = pts.filter(function (p, i) { return p !== spark && (kind !== 'heart' || i < pts.length - 1); })
      .map(function (p, i) { return '<circle cx="' + p[0] + '" cy="' + p[1] + '" r="' + (i % 3 ? 2.2 : 3) + '"/>'; }).join('');
    return '<svg class="sl-empty" viewBox="0 0 120 80" aria-hidden="true">' +
      '<polyline class="sl-empty__line" points="' + line + '"/><g class="sl-empty__stars">' + dots + '</g>' +
      '<path class="sl-empty__spark" transform="translate(' + (spark[0] - 10) + ' ' + (spark[1] - 10) + ') scale(.84)" d="' + SPARK + '"/></svg>';
  }

  /* your queue as a constellation: x spread evenly, y (in %) on a seeded zigzag; the sparkle is the song playing now */
  function homeLead(ctx) {
    var t = ctx.track;
    if (!t) return '';
    var songs = [t].concat(ctx.queue || []).slice(0, 7), n = songs.length, pts = [];
    songs.forEach(function (s, i) {
      pts.push({ s: s, x: n > 1 ? 3 + i * 94 / (n - 1) : 50, y: 18 + (i % 2 ? 40 : 0) + hash(String(s.id)) % 24 });
    });
    var stars = pts.map(function (p, i) {
      var tip = (i ? 'up next: ' : 'playing: ') + p.s.title + ' by ' + p.s.artist;
      return '<span class="sl-sky__star' + (i ? '' : ' sl-sky__star--now') + '" style="left:' + p.x + '%;top:' + p.y + '%;--i:' + i + '" title="' + esc(tip) + '"></span>';
    }).join('');
    return '<div class="sl-sky" aria-hidden="true"><div class="sl-sky__map">' +
      '<svg class="sl-sky__line" viewBox="0 0 100 100" preserveAspectRatio="none"><polyline points="' +
        pts.map(function (p) { return p.x + ',' + p.y; }).join(' ') + '" vector-effect="non-scaling-stroke"/></svg>' + stars +
      '</div><span class="sl-sky__label">your queue</span></div>';
  }

  /* the same markup now.js writes into data-now="c-screen" / "c-art" on a track change */
  function song(ctx) {
    var t = ctx.track;
    return '<div class="now-slide">' + (t
      ? '<p class="device__title">' + esc(t.title) + '</p><p class="device__artist">' + esc(t.artist) + '</p>'
      : '<p class="device__title">Nothing playing</p><p class="device__artist">Pick something from Home</p>') +
      String(ctx.parts.wave(26)) + '</div>';
  }
  function art(ctx) {
    return ctx.track ? String(window.App.ui.cover(ctx.track, 56)) : '<span class="now-dev__slot" aria-hidden="true"></span>';
  }
  function from(ctx) {
    var p = ctx.player || {};
    if (p.radio) return ic('radio', 'ic-16') + '<span>playing from <b>' + esc(p.radio.name) + '</b> radio</span>';
    return ctx.track ? ic('queue', 'ic-16') + '<span>playing from your queue</span>' : ic('sparkle', 'ic-16') + '<span>nothing queued</span>';
  }
  function like(ctx) {
    var t = ctx.track, on = t && window.App.isLiked(t.id);
    return '<button class="btn-icon btn-icon--bare sl-like' + (on ? ' is-liked' : '') + '" data-act="like" data-bind="like"' +
      (t ? ' data-id="' + esc(t.id) + '" data-like="' + esc(t.id) + '"' : ' disabled') +
      ' aria-pressed="' + !!on + '" aria-label="Like (L)" data-tip="' + (on ? 'Liked' : 'Like') + '">' + ic(on ? 'heart-fill' : 'heart') + '</button>';
  }
  /* failure line + fixes, hidden unless the player bar says failed / jellyfin-error (design.css) */
  function trouble(ctx) {
    var p = ctx.player || {};
    return '<div class="sl-hero__trouble sl-hero__trouble--failed">' +
        '<p class="sl-hero__err">' + ic('warning') + '<span>' + esc(p.status === 'failed' && p.error ? p.error : 'Couldn\'t play this song') + '</span></p>' +
        '<div class="sl-hero__fix"><button class="btn btn--primary btn--sm" data-act="retry-search">' + ic('refresh') + 'Retry search</button>' +
        '<button class="btn btn--sm" data-act="pick-source">Pick another source</button>' + btn('next', 'Skip (N)', 'next') + '</div></div>' +
      '<div class="sl-hero__trouble sl-hero__trouble--jellyfin">' +
        '<p class="sl-hero__err">' + ic('warning') + '<span>Jellyfin isn\'t answering</span></p>' +
        '<div class="sl-hero__fix"><button class="btn btn--primary btn--sm" data-act="try-soulseek">' + ic('soulseek') + 'Try Soulseek instead</button></div></div>';
  }

  function hero(ctx) {
    var p = ctx.player || {};
    if (ctx.variant === 'compact') {
      return '<div class="sl-hero sl-hero--compact now-devc" role="group" aria-label="Player">' +
        '<span class="sl-hero__thumb" data-now="c-art">' + art(ctx) + '</span>' +
        '<div class="sl-hero__song" data-now="c-screen">' + song(ctx) + '</div>' +
        '<div class="sl-hero__keys">' + btn('prev', 'Previous (P)', 'prev') +
        '<span data-now="ca">' + String(ctx.parts.playButton(52)) + '</span>' + btn('next', 'Next (N)', 'next') + '</div></div>';
    }
    var rep = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[p.repeat] || 'Repeat off';
    return '<div class="sl-hero now-dev" role="group" aria-label="Now playing"><span class="sparkle sl-hero__spark" aria-hidden="true"></span>' +
      '<div class="sl-hero__info">' +
        '<p class="sl-hero__from now-head__from">' + from(ctx) + '</p>' +
        '<div class="sl-hero__song" data-now="c-screen">' + song(ctx) + '</div>' +
        '<p class="sl-hero__time"><span data-bind="pos">0:00</span><span class="sl-hero__of" aria-hidden="true">/</span><span data-bind="dur">0:00</span></p>' +
        (ctx.track ? '<div class="sl-hero__facts"><span class="sl-hero__state" data-now="state">' + esc(ctx.parts.state()) + '</span>' +
          '<span class="device__facts" data-now="facts">' + ctx.parts.facts() + '</span></div>' : '') +
        String(window.App.ui.seekBar({ bind: 'player', label: 'Seek', cls: 'sl-hero__seek' })) +
        '<div class="sl-hero__ctl" role="group" aria-label="Transport">' +
          btn('shuffle', 'Shuffle (S)', 'shuffle', ' data-bind="shuffle" aria-pressed="' + !!p.shuffle + '"') +
          btn('prev', 'Previous (P)', 'prev') + String(ctx.parts.playButton(64)) + btn('next', 'Next (N)', 'next') +
          btn('repeat', rep + ' (R)', p.repeat === 'one' ? 'repeat-one' : 'repeat', ' data-bind="repeat" aria-pressed="' + (p.repeat && p.repeat !== 'off') + '"') +
          like(ctx) +
        '</div>' + trouble(ctx) +
      '</div>' +
      '<div class="sl-hero__art" data-now="c-art">' + art(ctx) + '</div>' +
    '</div>';
  }

  function mini(ctx, fallback) {
    if (ctx.style !== 'device') return fallback(ctx);
    var t = ctx.track || {}, on = !!ctx.liked;
    function k(dk, label, iconName, extra) {
      return '<button class="btn-icon btn-icon--bare" data-dk="' + dk + '" aria-label="' + label + '" data-tip="' + label + '"' + (extra || '') + '>' + ic(iconName) + '</button>';
    }
    return '<div class="dk-mini sl-mini' + (ctx.opaque ? ' dk-mini--opaque' : '') + '" data-dk-player="device">' +
      '<section class="sl-mini__card" aria-label="' + esc(Designs.product) + ' mini player" data-tauri-drag-region>' +
        '<div class="sl-mini__head" data-tauri-drag-region><span class="sl-mini__label" data-tauri-drag-region>now playing</span>' +
          '<span class="sl-mini__rule" data-tauri-drag-region></span>' +
          '<button class="dk-mini__win" data-dk="full" aria-label="Open full window" data-tip="Open full window">' + ic('expand') + '</button>' +
          '<button class="dk-mini__win" data-dk="mini-close" aria-label="Close mini player" data-tip="Close mini player">' + ic('close') + '</button></div>' +
        '<div class="sl-mini__now" data-dk-now aria-live="polite"><div class="dk-mini__now">' + String(ctx.parts.cover(84)) +
          '<div><div class="device__title">' + esc(t.title) + '</div><div class="device__artist">' + esc(t.artist) + '</div>' +
          '<div class="device__facts"><span class="q-chip ' + (t.lossless ? 'q-chip--ll' : 'q-chip--ly') + ' q-chip--solid">' + esc(t.format) + '</span></div></div></div></div>' +
        '<p class="sl-mini__times"><b data-dk-pos>0:00</b><span data-dk-dur>0:00</span></p>' +
        String(ctx.parts.wave()) +
        '<div class="sl-mini__keys">' + k('shuffle', 'Shuffle', 'shuffle', ' aria-pressed="false"') + k('prev', 'Previous', 'prev') +
          '<button class="btn-gloss btn-gloss--52" data-dk="play" aria-label="' + (ctx.playing ? 'Pause' : 'Play') + '" data-tip="' + (ctx.playing ? 'Pause' : 'Play') + '">' +
            ic(ctx.playing ? 'pause' : 'play', ctx.playing ? '' : 'ic-play') + '</button>' +
          k('next', 'Next', 'next') + k('repeat', 'Repeat', 'repeat', ' aria-pressed="true"') + '</div>' +
        '<div class="sl-mini__foot">' +
          '<button class="btn-icon btn-icon--bare' + (on ? ' is-liked' : '') + '" data-dk="like" aria-pressed="' + on + '" aria-label="Like" data-tip="' + (on ? 'Liked' : 'Like') + '">' + ic(on ? 'heart-fill' : 'heart') + '</button>' +
          '<span class="sl-mini__vol">' + k('vol-down', 'Volume down', 'minus') + '<span data-dk-vol>vol 70%</span>' + k('vol-up', 'Volume up', 'plus') + '</span></div>' +
        '<div class="dk-mini__note" aria-hidden="true"></div>' +
      '</section></div>';
  }

  Designs.define('starlight', { brand: brand, coverArt: coverArt, emptyArt: emptyArt, homeLead: homeLead, nowHero: hero, miniPlayer: mini });
})();
