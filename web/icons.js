/* lull icon sprite: the single source of every icon.
   24x24 viewBox, outline, stroke = currentColor at var(--icon-stroke) (set by .ic in base.css).
   Filled exceptions (meaning only): play, pause, heart-fill, saved, downloaded, star-fill, status-ok, status-warn,
   quality-diamond, plus the dot glyphs (more, grip).
   Loading this file injects the sprite once into <body> and defines:
     icon(name, label?, cls?) -> '<svg class="ic ...">' markup string
     ICON_NAMES                -> array of every icon name */
(function () {
  // name -> SVG body. A body starting with "F:" is a filled glyph.
  var ICONS = {
    'home': '<path d="M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z"/>',
    'search': '<circle cx="11" cy="11" r="6.5"/><path d="m16 16 4 4"/>',
    'library': '<path d="M5 4.5v15M9.5 4.5v15M14 5l4.6 14.4"/>',
    'playlists': '<path d="M4 6h11M4 10.5h11M4 15h7M18 15.8V6l2.5 1"/><circle cx="16" cy="16.2" r="2.2"/>',
    'stats': '<path d="M5 20v-8M10 20V5M15 20v-7M20 20V9"/>',
    'settings': '<path d="M4 7h9M17 7h3M4 17h3M11 17h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>',
    'play': 'F:<path d="M8 5.6v12.8a.9.9 0 0 0 1.4.8l10-6.4a.9.9 0 0 0 0-1.6l-10-6.4A.9.9 0 0 0 8 5.6z"/>',
    'pause': 'F:<rect x="6.5" y="5" width="4" height="14" rx="1.2"/><rect x="13.5" y="5" width="4" height="14" rx="1.2"/>',
    'prev': '<path d="M11.5 6.5 5 12l6.5 5.5zM19 6.5 12.5 12l6.5 5.5z"/>',
    'next': '<path d="M12.5 6.5 19 12l-6.5 5.5zM5 6.5 11.5 12 5 17.5z"/>',
    'shuffle': '<path d="M4 7h3.2c2 0 3.2 1 4.3 2.6l2.4 4.8c1.1 1.6 2.3 2.6 4.3 2.6H20M4 17h3.2c1.4 0 2.4-.5 3.2-1.3M13.6 8.3C14.4 7.5 15.4 7 16.8 7H20M17.5 4.5 20 7l-2.5 2.5M17.5 14.5 20 17l-2.5 2.5"/>',
    'repeat': '<path d="m17 3.5 3 3-3 3M4 11.5V10a3.5 3.5 0 0 1 3.5-3.5H20M7 20.5l-3-3 3-3M20 12.5V14a3.5 3.5 0 0 1-3.5 3.5H4"/>',
    'repeat-one': '<path d="m17 3.5 3 3-3 3M4 11.5V10a3.5 3.5 0 0 1 3.5-3.5H20M7 20.5l-3-3 3-3M20 12.5V14a3.5 3.5 0 0 1-3.5 3.5H4M11 10.6l1.5-1v4.9"/>',
    'heart': '<path d="M12 20s-8-4.7-8-10.6A4.4 4.4 0 0 1 12 6.8a4.4 4.4 0 0 1 8 2.6C20 15.3 12 20 12 20z"/>',
    'heart-fill': 'F:<path d="M12 20s-8-4.7-8-10.6A4.4 4.4 0 0 1 12 6.8a4.4 4.4 0 0 1 8 2.6C20 15.3 12 20 12 20z"/>',
    'save': '<path d="M12 4v10M8 10l4 4 4-4M4.5 14.5v3A2.5 2.5 0 0 0 7 20h10a2.5 2.5 0 0 0 2.5-2.5v-3"/>',
    'saved': 'F:<path fill-rule="evenodd" d="M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zm-4.4 9.2 1.4-1.4 2.2 2.2 4.8-4.8 1.4 1.4-6.2 6.2z"/>',
    'soulseek': '<circle cx="12" cy="9" r="1.6"/><path d="M12 10.6V20M9.5 20h5M8.6 5.6a4.8 4.8 0 0 0 0 6.8M15.4 5.6a4.8 4.8 0 0 1 0 6.8M6 3a8.5 8.5 0 0 0 0 12M18 3a8.5 8.5 0 0 1 0 12"/>',
    'quality-diamond': 'F:<path d="M12 4l7 8-7 8-7-8z"/>',
    'queue': '<path d="M4 6.5h16M4 12h16M4 17.5h9M16.5 15l3 2.5-3 2.5"/>',
    'lyrics': '<path d="M5 5h14a1.5 1.5 0 0 1 1.5 1.5v9A1.5 1.5 0 0 1 19 17h-8l-4 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5v-9A1.5 1.5 0 0 1 5 5zM8 9.5h8M8 12.5h5"/>',
    'classic': '<rect x="3.5" y="4.5" width="17" height="15" rx="2.5"/><path d="M3.5 9h17M6.5 6.8h.01M9 6.8h.01M11.5 6.8h.01"/>',
    'activity': '<path d="M8 4v16M4.5 7.5 8 4l3.5 3.5M16 20V4M12.5 16.5 16 20l3.5-3.5"/>',
    'volume-0': '<path d="M4 9.5h3l4.5-4v13l-4.5-4H4z"/>',
    'volume-1': '<path d="M4 9.5h3l4.5-4v13l-4.5-4H4zM15 9.5a3.5 3.5 0 0 1 0 5"/>',
    'volume-2': '<path d="M4 9.5h3l4.5-4v13l-4.5-4H4zM15 9.5a3.5 3.5 0 0 1 0 5M17.8 6.8a7.3 7.3 0 0 1 0 10.4"/>',
    'mute': '<path d="M4 9.5h3l4.5-4v13l-4.5-4H4zM15.5 9.5l5 5M20.5 9.5l-5 5"/>',
    'more': 'F:<circle cx="5.5" cy="12" r="1.7"/><circle cx="12" cy="12" r="1.7"/><circle cx="18.5" cy="12" r="1.7"/>',
    'plus': '<path d="M12 5v14M5 12h14"/>',
    'minus': '<path d="M5 12h14"/>',
    'close': '<path d="M6 6l12 12M18 6 6 18"/>',
    'chevron-down': '<path d="m6 9 6 6 6-6"/>',
    'chevron-up': '<path d="m6 15 6-6 6 6"/>',
    'chevron-left': '<path d="m15 6-6 6 6 6"/>',
    'chevron-right': '<path d="m9 6 6 6-6 6"/>',
    'grip': 'F:<circle cx="9" cy="6" r="1.5"/><circle cx="15" cy="6" r="1.5"/><circle cx="9" cy="12" r="1.5"/><circle cx="15" cy="12" r="1.5"/><circle cx="9" cy="18" r="1.5"/><circle cx="15" cy="18" r="1.5"/>',
    'radio': '<rect x="3.5" y="8.5" width="17" height="11.5" rx="2"/><path d="M7 8.5l9-4.5M7 12.5h4M7 15.5h4"/><circle cx="15.5" cy="14.2" r="2.5"/>',
    'sparkle': '<path d="M12 2.5c.6 5.9 3.6 8.9 9.5 9.5-5.9.6-8.9 3.6-9.5 9.5-.6-5.9-3.6-8.9-9.5-9.5 5.9-.6 8.9-3.6 9.5-9.5z"/>',
    'tag': '<path d="M3.5 12.2V5A1.5 1.5 0 0 1 5 3.5h7.2l8.3 8.3-8.7 8.7z"/><circle cx="8" cy="8" r="1.4"/>',
    'peer': '<circle cx="12" cy="8" r="3.5"/><path d="M5 20c.8-3.6 3.6-5.5 7-5.5s6.2 1.9 7 5.5"/>',
    'speed': '<path d="M4.6 17.5a8 8 0 1 1 14.8 0M12 14l4-4.5"/><circle cx="12" cy="14.5" r="1.3"/>',
    'warning': '<path d="M10.7 4.8a1.5 1.5 0 0 1 2.6 0l7.3 12.9a1.5 1.5 0 0 1-1.3 2.3H4.7a1.5 1.5 0 0 1-1.3-2.3zM12 9.5v4.5M12 17h.01"/>',
    'info': '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.5M12 7.8h.01"/>',
    'offline': '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.3a9.5 9.5 0 0 1 13 0M8.6 15.6a5 5 0 0 1 6.8 0M12 19h.01M4 3.5l16 17"/>',
    'refresh': '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3M19.5 4v4h-4"/>',
    'undo': '<path d="M9 14 4.5 9.5 9 5M4.5 9.5H14a5.5 5.5 0 0 1 0 11h-3"/>',
    'import': '<path d="M14 3.5H7A1.5 1.5 0 0 0 5.5 5v14A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5V8zM14 3.5V8h4.5M12 10.5V17M9.3 14.3 12 17l2.7-2.7"/>',
    'export': '<path d="M14 3.5H7A1.5 1.5 0 0 0 5.5 5v14A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5V8zM14 3.5V8h4.5M12 17v-6.5M9.3 13.2 12 10.5l2.7 2.7"/>',
    'edit': '<path d="M15.5 4.5l4 4L9 19H5v-4zM13.5 6.5l4 4"/>',
    'trash': '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5A1.5 1.5 0 0 0 9 21h6a1.5 1.5 0 0 0 1.5-1.5l1-12.5M10 11v6M14 11v6"/>',
    'star': '<path d="m12 3.5 2.6 5.3 5.9.9-4.2 4.1 1 5.8-5.3-2.8-5.3 2.8 1-5.8-4.2-4.1 5.9-.9z"/>',
    'star-fill': 'F:<path d="m12 3.5 2.6 5.3 5.9.9-4.2 4.1 1 5.8-5.3-2.8-5.3 2.8 1-5.8-4.2-4.1 5.9-.9z"/>',
    'expand': '<path d="M14 4h6v6M20 4l-6.5 6.5M10 20H4v-6M4 20l6.5-6.5"/>',
    'split': '<rect x="3.5" y="5" width="17" height="14" rx="2.5"/><path d="M12 5v14"/>',
    'sidebar': '<rect x="3.5" y="5" width="17" height="14" rx="2.5"/><path d="M14.5 5v14M17 9h1M17 12h1"/>',
    'eye': '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>',
    'eye-off': '<path d="M4 4l16 16M9.9 6A9.6 9.6 0 0 1 12 5.5c6 0 9.5 6.5 9.5 6.5a17 17 0 0 1-2.6 3.4M6.4 7.6A16.7 16.7 0 0 0 2.5 12S6 18.5 12 18.5c1.6 0 3-.4 4.2-1.1M9.9 9.9a3 3 0 0 0 4.2 4.2"/>',
    'external': '<path d="M14 4.5h5.5V10M19.5 4.5 11 13M17 13.5V18a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 18V8.5A1.5 1.5 0 0 1 6 7h4.5"/>',
    'keyboard': '<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6.5 10h.01M10 10h.01M13.5 10h.01M17 10h.01M7.5 14h9"/>',
    'sun': '<circle cx="12" cy="12" r="3.8"/><path d="M12 2.8v2M12 19.2v2M2.8 12h2M19.2 12h2M5.5 5.5l1.4 1.4M17.1 17.1l1.4 1.4M5.5 18.5l1.4-1.4M17.1 6.9l1.4-1.4"/>',
    'moon': '<path d="M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z"/>',
    'check': '<path d="m5 12.5 4.5 4.5L19 7.5"/>',
    'clock': '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
    'sort': '<path d="M7 4v16M4 17l3 3 3-3M14 6h6M14 11h4.5M14 16h3"/>',
    'grid': '<rect x="4" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="1.5"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="1.5"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="1.5"/>',
    'list': '<path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01"/>',
    'follow': '<circle cx="10" cy="8" r="3.5"/><path d="M3.5 20c.7-3.5 3.3-5.5 6.5-5.5 1.4 0 2.7.4 3.7 1.1M18 13v6M15 16h6"/>',
    'equaliser': '<path d="M6 20v-8M12 20V6M18 20v-9"/>',
    'bell': '<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15zM10 20.5a2.2 2.2 0 0 0 4 0"/>',
    'lock': '<rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>',
    'link': '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>',
    'arrow-up': '<path d="M12 19V5M6 11l6-6 6 6"/>',
    'device': '<path d="M8 2.5h8a2 2 0 0 1 2 2V17a4.5 4.5 0 0 1-4.5 4.5H8a2 2 0 0 1-2-2v-15a2 2 0 0 1 2-2z"/><rect x="8.5" y="5" width="7" height="6" rx="1"/><path d="M9 15.5h3M10.5 14v3M15 15.5h.01"/>',
    'connect': '<rect x="3" y="5" width="12" height="9" rx="1.5"/><path d="M6.5 18h5M9 14v4"/><rect x="16.5" y="7.5" width="5" height="12" rx="1.3"/><path d="M19 16.5h.01"/>',
    'globe': '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.4 2.6 3.5 5.4 3.5 8.5s-1.1 5.9-3.5 8.5c-2.4-2.6-3.5-5.4-3.5-8.5s1.1-5.9 3.5-8.5z"/>',
    'download': '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5v8.5M8.6 12.6 12 16l3.4-3.4"/>',
    'downloaded': 'F:<path fill-rule="evenodd" d="M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18zm-1 4.5v6.1l-2.3-2.3-1.4 1.4 4.7 4.7 4.7-4.7-1.4-1.4-2.3 2.3V7.5z"/>',
    'copy': '<rect x="8.5" y="8.5" width="11" height="11" rx="2"/><path d="M15.5 8.5v-2a2 2 0 0 0-2-2h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>',
    'dash': '<path d="M7 12h10"/>',
    'no-source': '<circle cx="12" cy="12" r="7.5"/><path d="m6.7 17.3 10.6-10.6"/>',
    'status-ok': 'F:<circle cx="12" cy="12" r="6"/>',
    'status-warn': 'F:<path d="M12 5.5 18.5 12 12 18.5 5.5 12z"/>',
    'status-down': '<circle cx="12" cy="12" r="6"/><path d="m7.8 16.2 8.4-8.4"/>'
  };

  var names = Object.keys(ICONS);
  var symbols = names.map(function (n) {
    var body = ICONS[n];
    if (body.indexOf('F:') === 0) {
      body = '<g fill="currentColor" stroke="none">' + body.slice(2) + '</g>';
    }
    return '<symbol id="i-' + n + '" viewBox="0 0 24 24">' + body + '</symbol>';
  }).join('');

  function inject() {
    if (document.getElementById('lull-sprite')) return;
    var holder = document.createElement('div');
    holder.innerHTML = '<svg id="lull-sprite" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" ' +
      'style="position:absolute;width:0;height:0;overflow:hidden">' + symbols + '</svg>';
    document.body.insertBefore(holder.firstChild, document.body.firstChild);
  }
  if (document.body) inject();
  else document.addEventListener('DOMContentLoaded', inject);

  /* icon('heart') -> decorative; icon('heart', 'Like') -> role="img" with a name.
     cls adds classes, e.g. 'ic-24'. Unknown names log a console warning. */
  window.icon = function (name, label, cls) {
    if (!ICONS[name]) {
      console.warn('icon: unknown name', name);
    }
    var a11y = label ? ' role="img" aria-label="' + String(label).replace(/"/g, '&quot;') + '"'
      : ' aria-hidden="true"';
    return '<svg class="ic' + (cls ? ' ' + cls : '') + '"' + a11y + ' focusable="false"><use href="#i-' + name + '"/></svg>';
  };
  window.ICON_NAMES = names;
})();
