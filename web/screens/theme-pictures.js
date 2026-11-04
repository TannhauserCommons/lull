/* screens/theme-pictures.js.
   The pictures lull makes of a listener's theme for Settings > Appearance (ADR-0080), loaded the first time one is
   needed (screens/appearance.js, App.themePictures). App.makeThemePictures(theme, modes) queues a theme: one at a
   time, Home in that theme loads in one off-screen frame (1440x900, motion reduced), and each mode is drawn from it
   into a 320x200 WebP of its top left 1152x720, framed as the built-in designs' pictures are, and sent to the hub with
   the rev it shows. The drawing is the page's own DOM and CSS in an SVG foreignObject, every font and image inlined,
   so it looks like the theme on any device. It stays out of the listener's way (calm(), at the end). A device that
   can't encode WebP leaves the picture to another. */
(function () {
  'use strict';
  var M = App.data;
  var W = 1440, H = 900, CROP_W = 1152, CROP_H = 720, OUT_W = 320, OUT_H = 200;

  var LATER_MS = 10000;                     // how long a page that changed a theme takes to draw it, and then some
  var queue = [], running = false;
  App.makeThemePictures = function (theme, modes, opts) {
    queue = queue.filter(function (q) { return q.theme.id !== theme.id; });    // a newer save replaces a pending one
    queue.push({ theme: theme, modes: modes, later: !!(opts && opts.later) });
    if (!running) next();
  };
  function next() {
    var job = queue.shift();
    if (!job) { running = false; return; }
    running = true;
    (job.later ? still(job) : Promise.resolve(job.modes)).then(function (modes) {
      // none left: another page made them, and this one shows them too
      return modes.length ? calm().then(function () { return make(job.theme, modes); }) : true;
    }).then(function (shown) { if (shown) App.emit('themePictures', job.theme.id); },
      function (err) { console.warn('lull: no picture of ' + job.theme.id + ' this time:', err.message); })
      .then(next);
  }

  /* ---------- one theme: the frame, each mode, the hub ---------- */
  function make(theme, modes) {
    var frame = document.createElement('iframe');
    frame.setAttribute('aria-hidden', 'true'); frame.tabIndex = -1; frame.inert = true;
    frame.style.cssText = 'position:fixed;left:-' + (W + 100) + 'px;top:0;width:' + W + 'px;height:' + H + 'px;border:0;pointer-events:none';
    frame.src = 'index.html#/home?design=' + encodeURIComponent(theme.id) + '&theme=' + modes[0] + '&motion=reduce';
    document.body.appendChild(frame);
    var made = 0, sheets = null;
    return loaded(frame).then(function (w) {
      return modes.reduce(function (p, mode, i) {
        return p.then(function () {
          return i ? calm().then(function () {
            w.Designs.applyTheme(mode);                     // the page's own switch, without the cross-fade it starts
            w.document.documentElement.removeAttribute('data-theme-fade');
          }) : null;
        }).then(function () { return settle(w); })
          .then(function () { return sheets || (sheets = css(w)); })    // the same for every mode: media queries don't ask it
          .then(function (text) { return calm().then(function () { return draw(w, text); }); })
          .then(function (blob) { return send(theme, mode, blob); })
          .then(function () { made++; });
      }, Promise.resolve());
    }).then(function () { done(); return made > 0; }, function (err) { done(); throw err; });
    function done() { frame.remove(); data = {}; }
  }
  /* a theme that changed elsewhere: the modes still missing a picture of its rev after the page that changed it has had
     its turn (none when it made them, or when the theme has changed again) */
  function still(job) {
    return new Promise(function (ok) { setTimeout(ok, LATER_MS); }).then(function () { return Designs.refresh(); }).then(function () {
      var m = App.designs[job.theme.id];
      if (!m || m.rev !== job.theme.rev) return [];
      return job.modes.filter(function (x) { return m.pictures.indexOf(x) < 0; });
    });
  }
  function loaded(frame) {
    return new Promise(function (ok, fail) {
      var t = setTimeout(function () { fail(new Error('the theme took too long to load')); }, 30000);
      frame.addEventListener('load', function () {
        var w = frame.contentWindow;
        if (!w || !w.Designs) { clearTimeout(t); fail(new Error('the page did not load')); return; }
        /* a picture is of a still page: no transitions, so changing the mode restyles it once (a page opened in that
           mode has none running either) */
        var still = w.document.createElement('style');
        still.textContent = '*, *::before, *::after { transition: none !important; }';
        w.document.head.appendChild(still);
        w.Designs.ready(function () { clearTimeout(t); ok(w); });
      }, { once: true });
    });
  }
  /* Home has drawn when what #view holds and how many pictures the page has stop changing, as the UI gates read it;
     then its fonts and the pictures on its screen are in. The frame is off the page's screen, where an engine may never
     start a lazy picture (WebKit), so those on the frame's own screen load at once; no wait is longer than a few seconds */
  function settle(w) {
    var doc = w.document, last = '', stable = 0, tries = 0;
    return new Promise(function (ok) {
      (function check() {
        var view = doc.getElementById('view');
        var now = (view ? view.innerHTML.length : 0) + ':' + doc.images.length;
        stable = now === last ? stable + 1 : 0; last = now;
        if (stable >= 4 || ++tries > 80) ok(); else setTimeout(check, 100);
      })();
    }).then(function () { return within(3000, doc.fonts.ready); }).then(function () {
      return within(5000, Promise.all([].filter.call(doc.images, function (im) {
        var r = im.getBoundingClientRect();
        return r.bottom > 0 && r.top < H && r.right > 0 && r.left < W && !im.complete;
      }).map(function (im) { im.loading = 'eager'; return im.decode().catch(function () {}); })));
    }).then(function () { return frames(w, 2); });
  }
  function send(theme, mode, blob) {
    if (!blob || blob.type !== 'image/webp') throw new Error('this browser can\'t write WebP');
    return M.api('/api/theme-pictures/' + encodeURIComponent(theme.id) + '/' + theme.rev + '-' + mode + '.webp', { method: 'PUT', body: blob });
  }

  /* ---------- drawing a page: its DOM and CSS as an SVG image, onto a canvas ---------- */
  /* a file the page uses as a data: URL, fetched once (the browser's cache has it) */
  var data = {};
  function dataUrl(url) {
    if (/^data:/.test(url)) return Promise.resolve(url);
    return data[url] || (data[url] = fetch(url, { credentials: 'same-origin' })
      .then(function (r) { return r.ok ? r.blob() : null; })
      .then(function (b) { return b && new Promise(function (ok) { var fr = new FileReader(); fr.onload = function () { ok(fr.result); }; fr.readAsDataURL(b); }); })
      .catch(function () { return null; }));
  }
  function draw(w, text) {
    return clone(w, w.document, text).then(function (svg) {
      return calm().then(function () {
        var img = new Image();                       // a data: URL: from a blob: URL this picture would lock the canvas
        img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
        return within(10000, img.decode(), 'the picture took too long to draw').then(function () { return img; });
      });
    }).then(function (img) {
      var c = document.createElement('canvas'); c.width = OUT_W; c.height = OUT_H;
      var g = c.getContext('2d'); g.imageSmoothingEnabled = true; g.imageSmoothingQuality = 'high';
      g.drawImage(img, 0, 0, CROP_W, CROP_H, 0, 0, OUT_W, OUT_H);
      return new Promise(function (ok) { c.toBlob(ok, 'image/webp', 0.86); });
    });
  }
  /* every stylesheet as one text, every url() a data: URL. Media queries are decided here, by the page being drawn: an
     SVG image would answer them for itself (as if it had no hover, for one). */
  function css(w) {
    var doc = w.document;
    function inline(text, base) {
      var re = /url\(\s*(['"]?)([^'")]+)\1\s*\)/g, urls = [];
      text.replace(re, function (m, q, u) { if (!/^(data:|#)/.test(u)) urls.push(u); return m; });
      if (!urls.length) return Promise.resolve(text);
      var got = {};
      return Promise.all(urls.map(function (u) { return dataUrl(new URL(u, base).href).then(function (d) { got[u] = d; }); }))
        .then(function () { return text.replace(re, function (m, q, u) { return /^(data:|#)/.test(u) ? m : got[u] ? 'url("' + got[u] + '")' : 'none'; }); });
    }
    var n = 0;
    function rules(list, base) {
      return [].reduce.call(list, function (p, r) {
        return p.then(function (out) { return ++n % 150 ? out : yieldNow().then(function () { return out; }); }).then(function (out) {
          if (r instanceof w.CSSImportRule) {
            if (!r.styleSheet) return out;
            return rules(r.styleSheet.cssRules, r.styleSheet.href || base).then(function (inner) {
              var block = r.layerName == null ? inner : '@layer ' + r.layerName + ' {' + inner + '}';
              return out + (r.media.mediaText && !w.matchMedia(r.media.mediaText).matches ? '' : block + '\n');
            });
          }
          if (r instanceof w.CSSMediaRule) return w.matchMedia(r.media.mediaText).matches ? rules(r.cssRules, base).then(function (t) { return out + t; }) : out;
          if (r instanceof w.CSSLayerBlockRule) return rules(r.cssRules, base).then(function (t) { return out + '@layer ' + r.name + ' {' + t + '}\n'; });
          if (r instanceof w.CSSSupportsRule) return rules(r.cssRules, base).then(function (t) { return out + '@supports ' + r.conditionText + ' {' + t + '}\n'; });
          return inline(r.cssText, base).then(function (t) { return out + t + '\n'; });
        });
      }, Promise.resolve(''));
    }
    return [].reduce.call(doc.styleSheets, function (p, s) {
      return p.then(function (out) {
        if (s.disabled) return out;
        return yieldNow().then(function () { return rules(s.cssRules, s.href || doc.baseURI); }).then(function (t) { return out + t; });
      });
    }, Promise.resolve('')).then(function (text) {
      /* a picture of the theme, not of this moment: no scrollbars, toasts or tooltips */
      return text + '\n* { scrollbar-width: none !important; } #toasts, .tooltip, .popover, .menu, .scrim { display: none !important; }';
    });
  }
  /* the page as XHTML in a foreignObject. Inside it CSS's :root is the <svg>, so the svg carries the page root's
     attributes (data-design, data-theme, ...) for the theme's :root rules to apply. */
  function clone(w, doc, text) {
    var live = doc.documentElement, root = live.cloneNode(true);
    [].forEach.call(root.querySelectorAll('script, link[rel="stylesheet"], style, noscript, iframe'), function (n) { n.remove(); });
    var liveImgs = live.querySelectorAll('img'), imgs = root.querySelectorAll('img');
    var liveCanvases = live.querySelectorAll('canvas');
    [].forEach.call(root.querySelectorAll('canvas'), function (c, i) {
      var im = doc.createElement('img');
      try { im.src = liveCanvases[i].toDataURL(); } catch (e) { /* a canvas it can't read stays empty */ }
      [].forEach.call(c.attributes, function (a) { im.setAttribute(a.name, a.value); });
      c.replaceWith(im);
    });
    var styled = root.querySelectorAll('[style*="url("]');
    return Promise.all([].map.call(imgs, function (im, i) {
      var src = liveImgs[i] && (liveImgs[i].currentSrc || liveImgs[i].src);
      im.removeAttribute('srcset'); im.removeAttribute('loading'); im.removeAttribute('decoding');
      if (!src) return null;
      return dataUrl(src).then(function (d) { if (d) im.setAttribute('src', d); else im.removeAttribute('src'); });
    }).concat([].map.call(styled, function (el) {
      var st = el.getAttribute('style'), re = /url\(\s*(['"]?)([^'")]+)\1\s*\)/g, urls = [];
      st.replace(re, function (m, q, u) { urls.push(u); return m; });
      return Promise.all(urls.map(function (u) { return /^data:/.test(u) ? u : dataUrl(new URL(u, doc.baseURI).href); })).then(function (got) {
        var i = 0;
        el.setAttribute('style', st.replace(re, function () { var d = got[i++]; return d ? 'url("' + d + '")' : 'none'; }));
      });
    }))).then(function () {
      var head = root.querySelector('head') || root, sheet = doc.createElement('style');
      sheet.textContent = text; head.appendChild(sheet);
      root.style.setProperty('width', W + 'px'); root.style.setProperty('height', H + 'px');
      var attrs = [].filter.call(live.attributes, function (a) { return a.name !== 'style' && a.name !== 'xmlns' && a.name !== 'data-theme-fade'; })
        .map(function (a) { return ' ' + a.name + '="' + a.value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;') + '"'; }).join('');
      /* the image is the frame's size: its own viewport is what the page's vh and vw mean inside it (the canvas crops) */
      return '<svg xmlns="http://www.w3.org/2000/svg"' + attrs + ' width="' + W + '" height="' + H + '"><foreignObject width="100%" height="100%">' +
        new XMLSerializer().serializeToString(root) + '</foreignObject></svg>';
    });
  }

  /* p, or after ms: on with what there is, or, with why, given up */
  function within(ms, p, why) {
    return Promise.race([p, new Promise(function (ok, fail) { setTimeout(function () { if (why) fail(new Error(why)); else ok(); }, ms); })]);
  }

  /* ---------- staying out of the way ----------
     The steps no script can split (the frame's own start, its change of mode, the browser drawing the SVG) each wait
     for calm: the page idle and nothing tapped, typed or scrolled for a moment, so a slow phone never makes the
     listener wait on a picture they didn't ask for. */
  var CALM_MS = 1500, lastInput = 0;
  ['pointerdown', 'keydown', 'wheel', 'touchstart'].forEach(function (type) {
    addEventListener(type, function () { lastInput = Date.now(); }, { capture: true, passive: true });
  });
  function calm() {
    return new Promise(function (ok) {
      (function wait() {
        var left = lastInput + CALM_MS - Date.now();
        if (left > 0) { setTimeout(wait, left); return; }
        if (window.requestIdleCallback) requestIdleCallback(function () { if (lastInput + CALM_MS > Date.now()) wait(); else ok(); }, { timeout: 3000 });
        else setTimeout(ok, 50);
      })();
    });
  }
  function yieldNow() {
    return window.scheduler && scheduler.yield ? scheduler.yield() : new Promise(function (ok) { setTimeout(ok, 0); });
  }
  /* n frames of the page, or as long as n frames take at 60 Hz if an off-screen frame's frames are held back */
  function frames(w, n) {
    return new Promise(function (ok) {
      setTimeout(ok, n * 17);
      (function step() { if (n-- <= 0) ok(); else w.requestAnimationFrame(step); })();
    });
  }
})();
