/* petal: slot overrides only (classic script).
   brand      "lull" in light Nunito Sans with a pixel heart.
   coverArt   mirrors about half of the pixel covers (the scenes are CSS backgrounds, art/cN.svg) and gives round
              covers (artists, radio starters) one of six pixel characters (art/p1-p6.svg).
   emptyArt   pixel scenes instead of line drawings (art/empty-*.svg).
   nowHero    full = the player window (img 03): gloss title bar with a pixel bunny that perks up while music plays,
              page tabs, the live screen, an LCD strip and one transport row. design.css joins the lyrics and queue
              cards to it as panes. compact = a one-row window (phone Lyrics / Queue / Details tabs).
   miniPlayer the desktop mini window as a tiny petal player (device style); the strip keeps its markup.
   Every live hook of the default slots (data-act, data-bind, data-now, data-dk-*) is kept; miniPlayer only uses ctx, window.icon and Designs. */
(function () {
  'use strict';
  function ic(name, cls) { return window.icon ? String(window.icon(name, null, cls)) : ''; }
  function esc(s) { return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) { return '&#' + c.charCodeAt(0) + ';'; }); }
  function hash(s) { var h = 7; for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0; return Math.abs(h); }

  /* tiny pixel sprites: rows of palette keys, '.' = empty (decorative, aria-hidden) */
  var INK = { g: 'var(--pt-px-line)', W: 'var(--pt-px-fill)', c: 'var(--pt-px-ear)', d: 'var(--pt-px-cheek)', j: 'var(--pt-px-eye)', e: 'currentColor' };
  function px(rows, cls) {
    var out = '';
    rows.forEach(function (row, y) {
      for (var x = 0; x < row.length; x++) {
        var k = row[x]; if (k === '.') continue;
        var w = 1; while (row[x + w] === k) w++;
        out += '<rect x="' + x + '" y="' + y + '" width="' + w + '" height="1" style="fill:' + INK[k] + '"/>';
        x += w - 1;
      }
    });
    return '<svg class="' + cls + '" viewBox="0 0 ' + rows[0].length + ' ' + rows.length + '" shape-rendering="crispEdges" aria-hidden="true" focusable="false">' + out + '</svg>';
  }
  var HEART = px(['.ee...ee.', 'eeee.eeee', 'eeeeeeeee', 'eeeeeeeee', '.eeeeeee.', '..eeeee..', '...eee...'], 'pt-px');
  var BUNNY_UP = px(['..gg....gg..', '.gWcg..gcWg.', '.gWcg..gcWg.', '.gWWggggWWg.', 'gWWWWWWWWWWg', 'gWjWWWWWWjWg', 'gWdWWggWWdWg',
    '.gWWWWWWWWg.', '..gWWWWWWg..', '.gWWWWWWWWg.', 'gWWgWWWWgWWg', 'gWWWWWWWWWWg', '.gggggggggg.'], 'pt-bun pt-bun--up');
  var BUNNY_DOWN = px(['............', '............', '............', '...gggggg...', '.ggWWWWWWgg.', 'gWcgWWWWgcWg', 'gWcjjWWjjcWg',
    'gWcgdggdgcWg', '.gggWWWWggg.', '.gWWWWWWWWg.', 'gWWgWWWWgWWg', 'gWWWWWWWWWWg', '.gggggggggg.'], 'pt-bun pt-bun--down');

  function key(act, label, iconName, extra) {
    return '<button class="btn-icon btn-icon--bare pt-win__key" data-act="' + act + '" aria-label="' + label + '" data-tip="' +
      label.replace(/ \(.*\)$/, '') + '"' + (extra || '') + '>' + ic(iconName) + '</button>';
  }
  /* gloss title bar: orbs, the title, the mascot. No fake window buttons: the orbs already say "window" */
  function bar(title) {
    return '<div class="pt-win__bar"><span class="pt-win__dots" aria-hidden="true"></span>' +
      '<span class="pt-win__title" aria-hidden="true">' + title + HEART + '</span>' +
      '<span class="pt-win__mascot" aria-hidden="true"><span class="pt-win__zz">z z</span>' + BUNNY_UP + BUNNY_DOWN + '</span></div>';
  }
  /* first paint of the live status label (now.js refreshes data-now="head" on every player event) */
  function headText(t, ps) {
    if (!t) return 'nothing playing';
    return { finding: 'finding a source', queued: 'queued at peer', buffering: 'buffering', stalled: 'peer stalled',
      failed: 'couldn\'t play', 'jellyfin-error': 'unavailable', ended: 'end of queue' }[ps.status] ||
      (ps.playing ? 'now playing' : 'paused');
  }

  /* first paint of the LCD state word (same words as now.js ledText, which refreshes data-now="led") */
  var LED = { idle: 'off', finding: 'search', queued: 'queued', buffering: 'buffer', streaming: 'stream', stalled: 'stalled',
    failed: 'error', library: 'library', ended: 'end', 'jellyfin-error': 'error' };
  function ledText(t, ps) {
    if (!t) return 'off';
    if (ps.crossfade) return 'xfade';
    if (!ps.playing && (ps.status === 'streaming' || ps.status === 'library')) return 'paused';
    return LED[ps.status] || 'stream';
  }

  function heroFull(ctx) {
    var p = ctx.parts, ps = ctx.player || {}, t = ctx.track;
    var rep = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[ps.repeat] || 'Repeat off';
    return '<div class="pt-win now-dev" role="group" aria-label="Player window">' + bar('lull player') +
      '<div class="pt-win__tabs">' +
        '<button class="pt-win__pages" data-act="now-page" aria-label="Change screen page (now, source, save)" data-tip="Next screen page">' +
          '<span class="pt-win__page" data-p="now">now</span><span class="pt-win__page" data-p="source">source</span>' +
          '<span class="pt-win__page" data-p="save">save</span></button>' +
        '<span class="pt-win__head" data-now="head" aria-hidden="true">' + headText(t, ps) + '</span>' +
      '</div>' +
      '<div class="device__screen s-screen pt-win__screen">' +
        '<div class="now-dev__host" data-now="screen">' + p.screen() + '</div>' +
        '<div class="now-osd" data-now="osd" aria-hidden="true"></div>' +
      '</div>' +
      '<div class="pt-win__lcd" aria-hidden="true"><span class="pt-win__lcdtext">.:: <b data-bind="pos">0:00</b>/<span data-bind="dur">0:00</span>' +
        ' :: <span data-now="led">' + ledText(t, ps) + '</span> :: rx <span data-bind="received">0%</span> ::.</span>' +
        '<span class="pt-win__kbd"><kbd class="kbd">P</kbd><kbd class="kbd">N</kbd><kbd class="kbd">Space</kbd></span></div>' +
      '<div class="pt-win__deck" role="group" aria-label="Transport">' +
        '<span class="pt-win__like" data-now="cb">' + p.likeButton(40) + '</span>' +
        '<span class="pt-win__keys">' +
          key('shuffle', 'Shuffle (S)', 'shuffle', ' data-bind="shuffle" aria-pressed="' + String(!!ps.shuffle) + '"') +
          key('prev', 'Previous (P)', 'prev') +
          '<span class="pt-win__play" data-now="a">' + p.playButton(64) + '</span>' +
          key('next', 'Next (N)', 'next') +
          key('repeat', rep + ' (R)', ps.repeat === 'one' ? 'repeat-one' : 'repeat', ' data-bind="repeat" aria-pressed="' + String(ps.repeat !== 'off') + '"') +
        '</span>' +
        '<span class="pt-win__vol" role="group" aria-label="Volume">' +
          key('now-vol', 'Volume down (Down arrow)', 'minus', ' data-dir="-1"') + key('now-vol', 'Volume up (Up arrow)', 'plus', ' data-dir="1"') +
        '</span>' +
      '</div>' +
    '</div>';
  }

  function heroCompact(ctx) {
    var p = ctx.parts, t = ctx.track;
    return '<div class="pt-win pt-win--compact now-devc" role="group" aria-label="Player">' + bar('lull player') +
      '<div class="pt-win__row">' +
        '<span class="now-devc__art" data-now="c-art">' + (t ? window.App.ui.cover(t, 56) : '<span class="now-dev__slot" aria-hidden="true"></span>') + '</span>' +
        '<div class="device__screen s-screen pt-win__screen" data-now="c-screen"><div class="now-slide">' +
          (t ? '<p class="device__title">' + esc(t.title) + '</p><p class="device__artist">' + esc(t.artist) + '</p>'
             : '<p class="device__title">Nothing playing</p><p class="device__artist">Pick something from Home</p>') +
          p.wave(26) + '</div></div>' +
        '<span class="pt-win__keys">' + key('prev', 'Previous (P)', 'prev') + key('next', 'Next (N)', 'next') +
          '<span data-now="cb">' + p.likeButton(40) + '</span><span data-now="ca">' + p.playButton(52) + '</span></span>' +
      '</div></div>';
  }

  /* desktop mini window: a tiny petal player. desktop.js rewrites [data-dk-now] (cover + title + facts) on track change. */
  function mini(ctx, fallback) {
    if (ctx.style !== 'device') return fallback(ctx);
    var playing = ctx.playing, liked = ctx.liked, t = ctx.track;
    function k(act, label, iconName, extra) {
      return '<button class="btn-icon btn-icon--bare pt-win__key" data-dk="' + act + '" aria-label="' + label + '" data-tip="' + label + '"' + (extra || '') + '>' + ic(iconName) + '</button>';
    }
    return '<div class="dk-mini pt-mini' + (ctx.opaque ? ' dk-mini--opaque' : '') + '" data-dk-player="device">' +
      '<section class="pt-win" aria-label="Mini player" data-tauri-drag-region>' +
        '<div class="pt-win__bar" data-tauri-drag-region><span class="pt-win__dots" aria-hidden="true"></span>' +
          '<span class="pt-win__title" aria-hidden="true">lull' + HEART + '</span>' +
          '<span class="pt-win__mascot" aria-hidden="true">' + (playing ? BUNNY_UP : BUNNY_DOWN) + '</span>' +
          '<span class="pt-mini__win">' + k('full', 'Open full window', 'expand') + k('mini-close', 'Close mini player', 'close') + '</span></div>' +
        '<div class="pt-mini__screen" data-tauri-drag-region>' +
          '<div class="pt-mini__head">Now playing...</div>' +
          '<div data-dk-now aria-live="polite"><div class="dk-mini__now">' + ctx.parts.cover(84) + '<div><div class="device__title">' + esc(t.title) + '</div>' +
            '<div class="device__artist">' + esc(t.artist) + '</div><div class="device__facts"><span class="q-chip q-chip--' + (t.lossless ? 'll' : 'ly') +
            ' q-chip--solid">' + esc(t.format) + '</span></div></div></div></div>' +
          ctx.parts.wave() +
        '</div>' +
        '<div class="pt-win__lcd" aria-hidden="true"><span class="pt-win__lcdtext">.:: <b data-dk-pos>1:24</b>/<span data-dk-dur>~3:52</span> :: <span data-dk-vol>vol 70%</span> ::.</span></div>' +
        '<div class="pt-win__deck">' +
          k('shuffle', 'Shuffle', 'shuffle', ' aria-pressed="false"') + k('previous', 'Previous', 'prev') +
          '<button class="btn-gloss btn-gloss--56" data-dk="play-pause" aria-label="' + (playing ? 'Pause' : 'Play') + '" data-tip="' + (playing ? 'Pause' : 'Play') + '">' + ic(playing ? 'pause' : 'play', playing ? '' : 'ic-play') + '</button>' +
          k('next', 'Next', 'next') + k('repeat', 'Repeat', 'repeat', ' aria-pressed="true"') +
        '</div>' +
        '<div class="pt-mini__foot">' +
          '<button class="btn-gloss btn-gloss--40' + (liked ? ' is-liked' : '') + '" data-dk="like" aria-pressed="' + liked + '" aria-label="Like" data-tip="Like">' + ic(liked ? 'heart-fill' : 'heart') + '</button>' +
          '<span class="pt-win__vol">' + k('vol-down', 'Volume down', 'minus') + '<span class="pt-mini__vol" aria-hidden="true">vol</span>' + k('vol-up', 'Volume up', 'plus') + '</span>' +
        '</div>' +
      '</section></div>';
  }

  var EMPTY = { moon: 1, slot: 1, star: 1, wave: 1 };

  Designs.define('petal', {
    brand: function () { return '<span class="wordmark pt-mark">lull' + HEART + '</span>'; },
    coverArt: function (ctx, fallback) {
      var out = String(fallback(ctx));
      if (ctx.round) return out.replace('class="cover ', 'class="cover pt-face-' + ((hash(ctx.key) >> 5) % 6 + 1) + ' ');
      return (hash(ctx.key) >> 3) & 1 ? out.replace('class="cover ', 'class="cover pv-flip ') : out;
    },
    /* errors get the tangled cassette whatever art the screen asked for, so "it broke" never looks like "nothing yet" */
    emptyArt: function (ctx) {
      var art = /couldn|could not|failed|error|went wrong|unreachable/i.test(ctx.title || '') ? 'slot' : EMPTY[ctx.art] ? ctx.art : 'moon';
      return '<span class="pt-empty pt-empty--' + art + '"></span>';
    },
    nowHero: function (ctx) { return ctx.variant === 'compact' ? heroCompact(ctx) : heroFull(ctx); },
    miniPlayer: mini
  });
})();
