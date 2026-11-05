/* screens/settings-fit.js: what keeps Settings whole in a window of any size, loaded by the Settings route on entry
   (system.js), so no other screen pays for it. Its rules come with it, into the `screens` layer after system.css: one
   file, since each file the hub serves costs a page its headers too (650 B, most of it the CSP).
   - The tab list (.sys-nav) sticks beside the section and is never taller than the view below its top padding: the
     view's height rides on #view as --view-h, so on a short window the list scrolls on its own instead of hiding its
     last tabs until the section has scrolled to its end. */
(function () {
  'use strict';
  var style = document.createElement('style');
  style.textContent = '@layer screens {' + [
    /* beside the section the tab list is never taller than the view below its top padding: on a short window it
       scrolls on its own, so its last tabs are always reachable. Alone (a phone's index), it is the page */
    '@container sys (min-width: 640px) {',
    '  .sys-nav { max-height: calc(var(--view-h, 100dvh) - var(--sp-4)); overflow-y: auto; overscroll-behavior: contain; }',
    '}'
  ].join('\n') + '}';
  document.head.appendChild(style);
  var view = App.$('#view'), last = -1;
  /* the content box's height and the bottom padding the view scrolls under: from the top padding to the bottom edge */
  if (view) new ResizeObserver(function (entries) {
    var h = Math.round(entries[0].contentRect.height + parseFloat(getComputedStyle(view).paddingBottom || 0));
    if (h !== last) { last = h; view.style.setProperty('--view-h', h + 'px'); }
  }).observe(view);
  App.settingsFit = true;
})();
