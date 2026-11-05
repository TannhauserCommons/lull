// A stand-in for the desktop app's Tauri bridge (window.__TAURI__), so desktop.js runs its wrapper code in a plain
// browser: it answers every call, counts what the page sends across by event or command name, and lets a test send
// the page an event as Rust or another window would: __bridge.fire('mini', { action: 'hello' }). A test sets what a
// command answers in __bridge.answers (a value, or a function of the call's arguments), and reads what was sent last
// in __bridge.last.
window.__bridge = { calls: {}, bytes: {}, largest: {}, heard: {}, answers: {}, last: {} };
(function () {
  function note(name, payload) {
    var b = JSON.stringify(payload === undefined ? null : payload).length, x = window.__bridge;
    x.calls[name] = (x.calls[name] || 0) + 1; x.bytes[name] = (x.bytes[name] || 0) + b; x.largest[name] = Math.max(x.largest[name] || 0, b);
    x.last[name] = payload;
    if (name in x.answers) {
      var a = typeof x.answers[name] === 'function' ? x.answers[name](payload) : x.answers[name];
      return Promise.resolve(a == null ? null : JSON.parse(JSON.stringify(a)));
    }
    return Promise.resolve(name === 'desktop_settings' ? {} : null);
  }
  var unlisten = function () { return Promise.resolve(function () {}); };
  // the main window's own calls too (desktop.js with __LULL_DESKTOP__ set, as the app's title bar draws it)
  var win = { minimize: note, toggleMaximize: note, close: note, onFocusChanged: unlisten, onResized: unlisten, listen: unlisten,
    isMaximized: function () { return Promise.resolve(false); }, setSize: note, focus: note };
  window.__bridge.fire = function (name, payload) { (window.__bridge.heard[name] || []).forEach(function (fn) { fn({ payload: payload }); }); };
  window.__TAURI__ = {
    core: { invoke: note },
    event: {
      emit: note, emitTo: function (to, name, p) { return note(name, p); },
      listen: function (name, fn) { (window.__bridge.heard[name] = window.__bridge.heard[name] || []).push(fn); return Promise.resolve(function () {}); }
    },
    window: { getCurrentWindow: function () { return win; } }
  };
})();
