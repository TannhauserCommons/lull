/* Where a pick can play from: the quiet mark (.pick-avail) Home draws at the end of each pick's reason row, from the
   feed's `avail` (Soulseek, YouTube, or both; docs/context/discovery.md#where-a-pick-plays). Loaded with the first
   availability answer (App.needAvail): Home's first load carries only the [data-avail] attribute and the mark's CSS hook
   (themes.md section 11). YouTube's glyph comes with it rather than with the icon sprite every page loads. */
(function () {
  var html = App.html, raw = App.raw, icon = App.icon;
  var NAMES = { soulseek: 'Soulseek', youtube: 'YouTube' };
  var sprite = document.getElementById('lull-sprite');
  if (sprite && !document.getElementById('i-youtube')) {
    sprite.insertAdjacentHTML('beforeend', '<symbol id="i-youtube" viewBox="0 0 24 24"><rect x="3" y="6" width="18" height="12" rx="3.5"/><path d="M10.5 9.6v4.8l4-2.4z"/></symbol>');
  }
  function glyph(s) {
    return s === 'youtube' ? raw('<svg class="ic" aria-hidden="true" focusable="false"><use href="#i-youtube"/></svg>') : icon(s);
  }
  App.availMark = function (avail) {
    if (!avail || !avail.length) return '';
    var label = 'On ' + avail.map(function (s) { return NAMES[s]; }).join(' and ');
    return html`<span class="pick-avail" role="img" aria-label="${label}" data-tip="${label}">${avail.map(function (s) { return html`<span class="pick-avail__src" data-src="${s}">${glyph(s)}</span>`; })}</span>`;
  };
})();
