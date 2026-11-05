/* window.LullNative for the web UI inside the Android app (docs/context/web.md: output interface,
   offline downloads). Defined before the page's scripts run; every call goes to the native player
   plugin (app/plugins/player). The hub's token never reaches the page: Kotlin adds it to requests. */
(function () {
  'use strict';
  if (window.LullNative) return;
  var PLUGIN = 'plugin:lull-player|';
  /* Tauri's API object arrives with the IPC script; look it up at call time, not now */
  function core() {
    var t = window.__TAURI__ && window.__TAURI__.core;
    if (!t) throw new Error('The lull app bridge is not ready');
    return t;
  }
  function call(cmd, args) { return command(PLUGIN + cmd, args); }
  function command(cmd, args) {
    return new Promise(function (resolve) { resolve(core().invoke(cmd, args || {})); })
      .catch(function (e) { throw e instanceof Error ? e : new Error(String(e)); });
  }
  /* events: a Channel registered with the plugin; the returned function unsubscribes */
  function on(event, fn) {
    var t = core(), channel = new t.Channel();
    channel.onmessage = fn;
    var registered = t.invoke(PLUGIN + 'registerListener', { event: event, handler: channel });
    return function () {
      registered.then(function () { return t.invoke(PLUGIN + 'remove_listener', { event: event, channelId: channel.id }); })
        .catch(function (e) { console.error('lull: removing a native listener failed', e); });
    };
  }
  /* Kotlin keeps covers as files; the WebView shows them through Tauri's asset protocol */
  function withArt(d) {
    d.artUrl = d.artPath ? core().convertFileSrc(d.artPath) : null;
    delete d.artPath;
    return d;
  }

  window.LullNative = Object.freeze({
    player: Object.freeze({
      load: function (item, opts) { return call('load', { item: item, options: opts || {} }); },
      play: function () { return call('play'); },
      pause: function () { return call('pause'); },
      seek: function (seconds) { return call('seek', { seconds: seconds }); },
      setVolume: function (v) { return call('set_volume', { volume: v }); },
      setNext: function (item) { return call('set_next', { item: item || null }); },
      crossfade: function (seconds) { return call('crossfade', { seconds: seconds }); },
      equalize: function (spec) { return call('equalize', { spec: spec || null }); },
      on: on
    }),
    downloads: Object.freeze({
      add: function (o) { return call('download_add', o); },
      remove: function (trackIds) { return call('download_remove', { trackIds: trackIds }); },
      list: function () { return call('download_list').then(function (r) { return r.items.map(withArt); }); },
      usage: function () { return call('download_usage'); },
      /* progress | done | error, kept apart from the player's own error event */
      on: function (type, fn) {
        return on('download-' + type, type === 'done' ? function (d) { fn(withArt(d)); } : fn);
      }
    }),
    /* the UI sounds (ADR-0081): the phone asks for quiet (ringer on silent or vibrate, touch sounds off). quiet() is a
       Promise of a boolean; onQuiet(fn) hears each change and returns the function that stops it */
    quiet: function () { return call('sound_quiet').then(function (r) { return !!r.quiet; }); },
    onQuiet: function (fn) { return on('sound-quiet', function (d) { fn(!!d.quiet); }); },
    /* this app's own updates (src/mobile/update.rs): a newer release's APK opens in the browser */
    updates: Object.freeze({
      status: function () { return command('app_update'); },
      check: function () { return command('app_update_check'); },
      install: function () { return command('app_update_install'); }
    })
  });
})();
