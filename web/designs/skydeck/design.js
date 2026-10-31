/* skydeck design slots (classic script; slot overrides only).
   nowHero (full): the default handheld plus a cartridge seated in its top slot. The cartridge label is built from the
   live compact hooks (data-now="c-art" / "c-screen"), so now.js swaps its cover and title on every track change; the
   cartridge is decorative (aria-hidden). The compact variant stays the default.
   emptyArt: a little handheld whose LCD asks for a cartridge (or shows static when something failed). The empty
   state's title and text say what happened; the art is decorative (its wrapper is aria-hidden). */
(function () {
  'use strict';

  function nowHero(ctx, fallback) {
    var device = String(fallback(ctx));
    if (ctx.variant !== 'full') return device;
    var src = document.createElement('div');
    src.innerHTML = String(ctx.parts.compact());
    var art = src.querySelector('[data-now="c-art"]');
    var title = src.querySelector('[data-now="c-screen"]');
    return '<div class="sd-cart" aria-hidden="true"><span class="sd-cart__grip"></span>' +
      '<span class="sd-cart__label">' +
        '<span class="sd-cart__art" data-now="c-art">' + (art ? art.innerHTML : '') + '</span>' +
        '<span class="sd-cart__title" data-now="c-screen">' + (title ? title.innerHTML : '') + '</span>' +
      '</span></div>' + device;
  }

  /* LCD line per empty state: errors get "no signal", the rest ask for a cartridge in the console's voice */
  function lcdLine(c) {
    var t = String(c.title || '').toLowerCase();
    if (/couldn't|snag|unavailable|isn't here|nothing lives|offline|down/.test(t)) return { text: 'no signal', fail: true };
    if (/drag songs|drop to add|no songs yet/.test(t)) return { text: 'no songs on this cart' };
    if (/playing|played|listening|queued|instrumental/.test(t)) return { text: 'press play' };
    if (c.art === 'slot') return { text: 'insert cart' };
    if (c.art === 'moon') return { text: 'all quiet' };
    return { text: 'empty slot' };
  }

  function emptyArt(c) {
    var l = lcdLine(c);
    return '<span class="sd-empty' + (l.fail ? ' sd-empty--nosignal' : '') + '">' +
      '<span class="sd-empty__bezel"><span class="sd-empty__lcd"><span>' + l.text +
        (l.fail ? '' : '<i class="sd-empty__cursor"></i>') + '</span></span></span>' +
      '<span class="sd-empty__keys"><span class="sd-empty__dpad"></span><span class="sd-empty__ab"><i></i><i></i></span></span>' +
      '</span><span class="sparkle"></span>';
  }

  Designs.define('skydeck', { nowHero: nowHero, emptyArt: emptyArt });
})();
