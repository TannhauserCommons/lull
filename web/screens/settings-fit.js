/* screens/settings-fit.js: what keeps Settings whole in a window of any size, loaded by the Settings route on entry
   (system.js), so no other screen pays for it. Its rules come with it, into the `screens` layer after system.css: one
   file, since each file the hub serves costs a page its headers too (650 B, most of it the CSP).
   - The tab list (.sys-nav) sticks beside the section and is never taller than the view below where it sits: the
     view's height rides on #view as --view-h, and how far below the view's top the list still sits (the page's heading
     above it, until the page scrolls past it) on the list as --nav-lift. So on a short window the list scrolls on its
     own, and its last tabs never lie past the view's bottom, under the player bar, until the section has scrolled.
   - A choice (.sys-seg) whose buttons don't fit its row side by side wraps, two to a row or else one (data-cols), so its
     last choice never runs past the edge however wide a theme draws its labels. app.js runs App.fitSeg as it places a
     choice's indicator (placeSeg), which then still sets where it sits across; down, App.fitSeg lines its bottom up
     with the picked button's (--seg-y, --seg-h), from wherever the design rests it (a full pill, a line underneath). */
(function () {
  'use strict';
  var style = document.createElement('style');
  style.textContent = '@layer screens {' + [
    /* beside the section the tab list is never taller than the view below where it sits: on a short window it
       scrolls on its own, so its last tabs are always reachable. Alone (a phone's index), it is the page */
    '@container sys (min-width: 640px) {',
    '  .sys-nav { max-height: calc(var(--view-h, 100dvh) - var(--sp-4) - var(--nav-lift, 0px)); overflow-y: auto; overscroll-behavior: contain; }',
    '}',
    /* a choice is never wider than its row: when its buttons don't fit side by side they wrap, two to a row or one
       (data-cols), and the indicator follows the picked one down as well as across */
    '.sys-seg { max-width: 100%; }',
    '.sys-seg[data-cols] { grid-auto-flow: row; grid-template-columns: repeat(var(--cols), minmax(0, 1fr)); border-radius: var(--r-input); }',
    '.sys-seg[data-cols="2"] { --cols: 2; }',
    '.sys-seg[data-cols="1"] { --cols: 1; }',
    '.sys-seg[data-cols] .seg__btn { white-space: normal; }',
    '.sys-seg[data-cols] .seg__ind { top: 0; bottom: auto; height: var(--seg-h); transform: translate(var(--seg-x), var(--seg-y)); }'
  ].join('\n') + '}';
  document.head.appendChild(style);
  var view = App.$('#view'), last = -1;
  /* the content box's height and the bottom padding the view scrolls under: from the top padding to the bottom edge */
  if (view) new ResizeObserver(function (entries) {
    var h = Math.round(entries[0].contentRect.height + parseFloat(getComputedStyle(view).paddingBottom || 0));
    if (h !== last) { last = h; view.style.setProperty('--view-h', h + 'px'); }
    if (liftNav) liftNav();
  }).observe(view);
  /* how far below where it sticks (the view's top padding, and its own top) the tab list sits: the page's heading's
     height at first, nothing once the page has scrolled past it and the list sticks. Read as the page scrolls (once a frame, and only until the list sticks does it
     change), set on the list alone so nothing else restyles. Settings' mount runs it; it returns its cleanup */
  var liftNav = null;
  App.fitSettingsNav = function () {
    var nav = view && view.querySelector('.sys-nav'), lift = -1, frame = 0;
    if (!nav) return function () {};
    liftNav = function () {
      frame = 0;
      var stuckAt = view.getBoundingClientRect().top + view.clientTop + parseFloat(getComputedStyle(view).paddingTop) + (parseFloat(getComputedStyle(nav).top) || 0);
      var l = Math.max(0, Math.round(nav.getBoundingClientRect().top - stuckAt));
      if (l !== lift) { lift = l; nav.style.setProperty('--nav-lift', l + 'px'); }
    };
    var onScroll = function () { if (!frame) frame = requestAnimationFrame(liftNav); };
    liftNav();
    view.addEventListener('scroll', onScroll, { passive: true });
    return function () { cancelAnimationFrame(frame); view.removeEventListener('scroll', onScroll); liftNav = null; };
  };
  App.fitSeg = function (s) {
    if (!s.classList.contains('sys-seg')) return;
    var n = s.querySelectorAll('.seg__btn').length, fits = function () { return s.scrollWidth <= s.clientWidth + 1; };
    s.removeAttribute('data-cols');
    if (!fits()) {
      if (n > 2 && n % 2 === 0) s.setAttribute('data-cols', '2');
      if (!fits()) s.setAttribute('data-cols', '1');
    }
    var b = s.querySelector('.seg__btn[aria-checked="true"]'), ind = s.querySelector('.seg__ind');
    if (!s.hasAttribute('data-cols') || !b || !ind) return;
    var k = s.offsetWidth / (s.getBoundingClientRect().width || 1), h = b.offsetHeight;
    ind.style.setProperty('--seg-h', h + 'px');              // first, so where it rests (offsetTop ignores the move) is measured at its height
    ind.style.setProperty('--seg-y', ((b.getBoundingClientRect().bottom - s.getBoundingClientRect().top) * k - s.clientTop - ind.offsetTop - ind.offsetHeight).toFixed(2) + 'px');
  };
  App.$$('.sys-seg').forEach(App.fitSeg);
  App.settingsFit = true;
})();
