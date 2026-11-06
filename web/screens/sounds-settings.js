/* Settings > Sounds (ADR-0081): this device's sounds, the packs they come from, and the hidden surprises. Loaded as the
   section opens (system.js). Everything here is per device, as the other display settings are, except the packs
   themselves, which the hub keeps for every device. */
(function () {
  'use strict';
  var html = App.html, raw = App.raw, icon = App.icon, M = App.data;
  var K = App.settingsKit, row = K.row, group = K.group, custom = K.custom;
  var SND = App.sounds, S = window.LullSoundRules;

  /* its look, in the screens layer as if it were system.css's, which loads at start on every route: the packs are rows
     of a plain list; the surprises sit behind a spoiler */
  if (!document.getElementById('sounds-settings-css')) {
    var css = document.createElement('style'); css.id = 'sounds-settings-css';
    css.textContent = '@layer screens {' +
      '.sys-packs { list-style: none; margin: 0; padding: 0; }' +
      '.sys-surprises > summary { margin: var(--sp-2) 0; list-style: none; }' +
      '.sys-surprises > summary::-webkit-details-marker { display: none; } }';
    document.head.appendChild(css);
  }

  /* the words for each sound: what it is, and when it plays */
  var WORDS = {
    boot: ['Start-up', 'Once, when lull starts'],
    play: ['Play', 'When you press play'],
    pause: ['Pause', 'When you press pause'],
    skip: ['Next and previous', 'When you move to another song'],
    seek: ['Seeking', 'When you let go of the seek bar'],
    buffering: ['Waiting', 'A soft loop while a song is found and loads. Never over music'],
    found: ['Found a source', 'When lull finds a copy of a song it was looking for'],
    stalled: ['Stalled', 'When a transfer stops moving'],
    saved: ['Saved', 'When a song you saved is in your library'],
    liked: ['Liked', 'When you like a song'],
    queued: ['Added to the queue', 'Play next, Add to queue'],
    error: ['Something went wrong', 'A song that won\'t play, or an action that failed'],
    design: ['Design changed', 'When you switch design or theme'],
    classic: ['Classic mode', 'When Classic mode opens']
  };

  function st() { return SND.settings(); }
  function set(patch) { SND.set(patch); }
  function one(key, k, v) { var o = Object.assign({}, st()[key]); o[k] = v; var p = {}; p[key] = o; return p; }

  custom['sounds.on'] = { get: function () { return st().on; }, set: function (v) { set({ on: !!v }); App.refresh(); } };
  custom['sounds.volume'] = { get: function () { return st().volume; }, set: function (v) { set({ volume: +v }); SND.hear('play'); } };
  K.fmtRange['sounds.volume'] = function (v) { return v + '%'; };
  custom['sounds.onlySilent'] = { get: function () { return st().onlySilent; }, set: function (v) { set({ onlySilent: !!v }); } };
  custom['sounds.followQuiet'] = { get: function () { return st().followQuiet; }, set: function (v) { set({ followQuiet: !!v }); } };
  custom['sounds.surprises'] = { get: function () { return st().surprises; }, set: function (v) { set({ surprises: !!v }); App.refresh(); } };
  function perSound(prefix, key, getter) {
    (SND.vocab() ? SND.vocab().events : Object.keys(WORDS)).forEach(function (e) {
      custom[prefix + e] = { get: function () { return getter(e); }, set: function (v) { set(one(key, e, !!v)); } };
    });
  }
  perSound('sounds.sound.', 'sounds', function (e) { return S.enabled(e, st(), SND.vocab()); });
  perSound('sounds.background.', 'background', function (e) { return !!st().background[e]; });

  App.settingsSections.sounds = function () {
    var vocab = SND.vocab();
    if (!vocab || !SND.allRead()) {
      SND.load(true).then(function () { if (/^\/settings\/sounds/.test(App.current.path)) App.refresh(); });
      return App.ui.skeleton('text', 6);
    }
    var on = st().on;
    return html`
      ${K.switchRow('sounds.on', 'Sounds', 'Small sounds for what happens in lull, in your design\'s style. On this device only.')}
      <div ${on ? '' : raw('hidden')}>
        ${row('Volume', 'A share of this device\'s volume', K.range('sounds.volume', 'Sound volume', 0, 100, 5))}
        ${K.switchRow('sounds.onlySilent', 'Only when no music is playing', 'While a song plays, only play and pause make a sound. The waiting loop never plays over music.')}
        ${K.switchRow('sounds.followQuiet', 'Follow the system\'s quiet', 'No sounds while this device asks for reduced motion, or is set to silent.')}
        ${group('each sound')}
        <div class="sys-sounds">${vocab.events.map(soundRow)}</div>
        ${group('in the background')}
        <p class="sys-lede">Sounds play only while lull is the window in front. These may also play while it isn't.</p>
        ${vocab.background.map(function (e) { return K.switchRow('sounds.background.' + e, WORDS[e][0], WORDS[e][1]); })}
      </div>
      ${group('sound packs')}
      ${packList()}
      ${group('hidden surprises')}
      ${K.switchRow('sounds.surprises', 'Hidden surprises', 'Now and then lull, your design or a pack does something small and unexpected. Rare, and never in your way.')}
      ${st().surprises ? surprises() : ''}`;
  };

  /* ---------- each sound ---------- */
  function soundRow(e) {
    var w = WORDS[e] || [e, ''], s = S.soundFor(e, SND.stack(), st(), SND.known()), from = s ? SND.known()[s.pack] : null;
    var replaced = st().replace[e];
    var help = w[1] + (from ? '. From ' + from.name + (replaced ? ' (your choice)' : '') : '. Silent in these packs');
    return row(w[0], help, html`<span class="cluster">
        <button type="button" class="btn-icon btn-icon--sm" data-act="sound-hear" data-name="${e}" aria-label="Hear ${w[0]}" data-tip="Hear it" ${s ? '' : raw('disabled')}>${icon('play', null, 'ic-16')}</button>
        <button type="button" class="btn btn--quiet btn--sm" data-act="sound-replace" data-name="${e}" aria-haspopup="menu">Replace</button>
        <button type="button" class="switch" role="switch" aria-checked="${S.enabled(e, st(), SND.vocab()) ? 'true' : 'false'}" aria-label="${w[0]}" data-switch="sounds.sound.${e}"></button>
      </span>`);
  }
  App.action('sound-hear', function (el) { SND.hear(el.getAttribute('data-name'), el.getAttribute('data-pack') || null, el.hasAttribute('data-egg')); });
  /* Replace: this sound from any pack this device knows, the packs' own again, or a file of the listener's */
  App.action('sound-replace', function (el) {
    var e = el.getAttribute('data-name'), known = SND.known();
    var items = Object.keys(known).filter(function (id) { return known[id].sounds && known[id].sounds[e]; }).map(function (id) {
      return { label: known[id].name, icon: 'play', run: function () { set(one('replace', e, { pack: id, sound: e })); SND.hear(e); App.refresh(); } };
    });
    items.push('sep', { label: 'A file of yours…', icon: 'import', run: function () { pickFile('.ogg,.opus,audio/ogg', function (f) { ownSound(e, f); }); } });
    if (st().replace[e]) items.push({ label: 'Back to the packs\' own', icon: 'undo', run: function () { var r = Object.assign({}, st().replace); delete r[e]; set({ replace: r }); App.refresh(); } });
    App.menu(el, items, { label: 'Replace ' + (WORDS[e] || [e])[0] });
  });

  /* ---------- packs ---------- */
  function packList() {
    var known = SND.known(), on = SND.stack().map(function (p) { return p.id; }), look = SND.look();
    var listed = st().packs.concat(look && st().packs.indexOf(look) < 0 ? [look] : [], ['default']).filter(function (id, i, a) { return known[id] && a.indexOf(id) === i; });
    var others = Object.keys(known).filter(function (id) { return listed.indexOf(id) < 0; });
    return html`<p class="sys-lede">A sound comes from the first pack that has it. Your look brings its own pack; add others to mix them in.</p>
      <ul class="sys-packs">${listed.map(function (id, i) {
        var p = known[id], added = st().packs.indexOf(id) >= 0, isOn = on.indexOf(id) >= 0;
        var what = id === look ? 'Your look\'s pack' : id === 'default' ? 'lull\'s own, under every other' : 'Added';
        return html`<li class="sys-row"><div class="sys-row__text"><span class="sys-row__label">${p.name}${p.author ? html` <span class="fg-2">by ${p.author}</span>` : ''}</span><span class="sys-row__help">${what}${p.description ? '. ' + p.description : ''}</span></div>
          <div class="sys-row__ctl"><span class="cluster">
            ${added && i > 0 ? html`<button type="button" class="btn-icon btn-icon--sm" data-act="pack-up" data-id="${id}" aria-label="Move ${p.name} up">${icon('chevron-up', null, 'ic-16')}</button>` : ''}
            ${added ? html`<button type="button" class="btn btn--quiet btn--sm" data-act="pack-remove" data-id="${id}">Remove</button>` : ''}
            <button type="button" class="switch" role="switch" aria-checked="${isOn ? 'true' : 'false'}" aria-label="${p.name}" data-act="pack-toggle" data-id="${id}"></button>
          </span></div></li>`;
      })}</ul>
      <div class="cluster">
        ${others.length ? html`<button type="button" class="btn btn--sm" data-act="pack-add" aria-haspopup="menu">${icon('plus', null, 'ic-16')}Add a pack</button>` : ''}
        <button type="button" class="btn btn--quiet btn--sm" data-act="pack-import">${icon('import', null, 'ic-16')}Import a .lullpack</button>
      </div>`;
  }
  App.action('pack-toggle', function (el) {
    var id = el.getAttribute('data-id'), off = Object.assign({}, st().packsOff);
    if (off[id]) delete off[id]; else off[id] = true;
    set({ packsOff: off }); App.refresh();
  });
  App.action('pack-remove', function (el) { var id = el.getAttribute('data-id'); set({ packs: st().packs.filter(function (x) { return x !== id; }) }); App.refresh(); });
  App.action('pack-up', function (el) {
    var id = el.getAttribute('data-id'), p = st().packs.slice(), i = p.indexOf(id);
    if (i > 0) { p.splice(i, 1); p.splice(i - 1, 0, id); set({ packs: p }); App.refresh(); }
  });
  App.action('pack-add', function (el) {
    var known = SND.known(), listed = SND.stack().map(function (p) { return p.id; });
    App.menu(el, Object.keys(known).filter(function (id) { return listed.indexOf(id) < 0 && st().packs.indexOf(id) < 0; }).map(function (id) {
      return { label: known[id].name, icon: 'plus', run: function () { set({ packs: [id].concat(st().packs) }); App.refresh(); } };
    }), { label: 'Add a pack' });
  });
  App.action('pack-import', function () { pickFile('.lullpack,.zip', importPack); });
  App.importPack = importPack;
  function importPack(file) {
    fetch(M.url('/api/packs'), { method: 'POST', body: file, credentials: 'same-origin' })
      .then(function (r) { return r.json().then(function (j) { return { status: r.status, body: j }; }); })
      .then(function (r) {
        if (r.status === 409) {
          App.confirm({ title: 'Replace the pack?', text: r.body.message + '. Importing replaces it with this file.', confirm: 'Replace' }, function () {
            fetch(M.url('/api/packs/' + encodeURIComponent(r.body.id || '')), { method: 'PUT', body: file, credentials: 'same-origin' }).then(function () { return SND.reload(); }).then(App.refresh);
          });
          return;
        }
        if (r.status !== 200 && r.status !== 201) throw new Error(r.body.message || 'The hub answered ' + r.status);
        set({ packs: [r.body.id].concat(st().packs.filter(function (x) { return x !== r.body.id; })) });
        App.toast('Added ' + r.body.name, { kind: 'ok', icon: 'bell' });
        return SND.reload().then(function () { if (/^\/settings\/sounds/.test(App.current.path)) App.refresh(); });
      })
      .catch(function (err) { App.toast('Couldn\'t import ' + file.name + ': ' + err.message, { kind: 'error' }); });
  }

  /* a file of the listener's for one sound: kept on the hub in their own pack (my-sounds), every other sound of it kept */
  function ownSound(event, file) {
    var mine = SND.known()[S.MY_SOUNDS], keep = mine ? Object.keys(mine.sounds).filter(function (e) { return e !== event; }) : [];
    Promise.all(keep.map(function (e) {
      return fetch(S.normal(mine.sounds[e]).urls[0], { credentials: 'same-origin' }).then(function (r) { return r.arrayBuffer(); }).then(function (b) { return [e, b]; });
    })).then(function (kept) {
      return file.arrayBuffer().then(function (b) { return kept.concat([[event, b]]); });
    }).then(function (all) {
      var manifest = { id: S.MY_SOUNDS, name: 'Your sounds', author: '', api: 1, sounds: {}, eggs: [] }, files = [];
      all.forEach(function (x) { manifest.sounds[x[0]] = 'sounds/' + x[0] + '.ogg'; files.push(['sounds/' + x[0] + '.ogg', new Uint8Array(x[1])]); });
      files.unshift(['pack.json', new TextEncoder().encode(JSON.stringify(manifest, null, 2))]);
      return fetch(M.url('/api/packs/' + S.MY_SOUNDS), { method: 'PUT', body: zip(files), credentials: 'same-origin' });
    }).then(function (r) {
      if (!r.ok) return r.json().then(function (j) { throw new Error(j.message || 'The hub answered ' + r.status); });
      set(one('replace', event, { pack: S.MY_SOUNDS, sound: event }));
      return SND.reload().then(function () { SND.hear(event); App.refresh(); });
    }).catch(function (err) { App.toast('Couldn\'t use ' + file.name + ': ' + err.message, { kind: 'error' }); });
  }

  /* ---------- surprises: behind a spoiler, each with its own switch and a way to try it ---------- */
  function surprises() {
    var eggs = SND.eggs(), names = Object.keys(eggs).sort();
    if (!names.length) return '';
    return html`<details class="sys-surprises"><summary class="btn btn--quiet btn--sm">Show the surprises (spoilers)</summary>
      ${names.map(function (n) {
        var g = eggs[n], p = SND.known()[g.pack];
        return row(label(n), when(g) + '. From ' + (p ? p.name : g.pack), html`<span class="cluster">
          <button type="button" class="btn btn--quiet btn--sm" data-act="egg-try" data-name="${n}" aria-label="Try ${label(n)}">Try it</button>
          <button type="button" class="switch" role="switch" aria-checked="${st().eggsOff[n] ? 'false' : 'true'}" aria-label="${label(n)}" data-act="egg-toggle" data-name="${n}"></button></span>`);
      })}</details>`;
  }
  /* an egg's name as words: "long-wait" -> "Long wait" */
  function label(n) { var w = n.replace(/-/g, ' '); return w.charAt(0).toUpperCase() + w.slice(1); }
  /* "12-31" in this device's language ("31 December") */
  function day(mmdd) {
    var p = mmdd.split('-');
    return new Date(2000, +p[0] - 1, +p[1]).toLocaleDateString(undefined, { day: 'numeric', month: 'long' });
  }
  var TARGETS = { brand: 'logo', cover: 'playing cover', title: 'playing title' };
  /* an egg's conditions in words, for the spoiler list */
  function when(g) {
    return g.when.map(function (c) {
      var k = Object.keys(c)[0], p = c[k], list = function (v) { return [].concat(v).join(', '); };
      switch (k) {
        case 'clock': return (p.date ? 'on ' + [].concat(p.date).map(day).join(', ') + ' ' : '') + (p.weekday ? 'on ' + list(p.weekday) + ' ' : '') + (p.from || p.to ? 'from ' + (p.from || '00:00') + ' to ' + (p.to || '24:00') : 'all day');
        case 'words': return 'a ' + list(p.in) + ' with ' + list(p.any);
        case 'repeat': return 'the same song ' + p.times + ' times in a row';
        case 'gesture': return p.taps + ' taps on the ' + TARGETS[p.on] + ' within ' + p.within + ' s';
        case 'milestone': return 'your ' + list(p.at) + ' ' + p.of;
        case 'player': return 'the player ' + [p.status && list(p.status), p.playing === true && 'playing', p.playing === false && 'silent', p.screen && 'on ' + list(p.screen), p.idle && 'left alone ' + p.idle + ' s', p.held && 'for ' + p.held + ' s'].filter(Boolean).join(', ');
        case 'event': return 'at ' + ((WORDS[p.on] || [p.on])[0]).toLowerCase();
        case 'chance': return 'one time in ' + p.one_in;
        default: return k;
      }
    }).map(function (t, i) { return i ? t : t.charAt(0).toUpperCase() + t.slice(1); }).join('; ');
  }
  App.action('egg-toggle', function (el) {
    var n = el.getAttribute('data-name'), off = Object.assign({}, st().eggsOff);
    if (off[n]) delete off[n]; else off[n] = true;
    set({ eggsOff: off }); el.setAttribute('aria-checked', off[n] ? 'false' : 'true');
  });
  App.action('egg-try', function (el) {
    var n = el.getAttribute('data-name');
    App.needEggs().then(function () { if (!App.eggs.fire(n)) App.toast('That surprise is switched off', { icon: 'info' }); });
  });

  /* ---------- files ---------- */
  function pickFile(accept, then) {
    var input = document.createElement('input');
    input.type = 'file'; input.accept = accept;
    input.addEventListener('change', function () { if (input.files[0]) then(input.files[0]); });
    input.click();
  }
  /* a zip of `files` ([name, Uint8Array]), stored without compression: Ogg is compressed already */
  var CRC = (function () {
    var t = new Uint32Array(256);
    for (var n = 0; n < 256; n++) { var c = n; for (var k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; t[n] = c >>> 0; }
    return t;
  })();
  function crc32(b) { var c = 0xffffffff; for (var i = 0; i < b.length; i++) c = CRC[(c ^ b[i]) & 0xff] ^ (c >>> 8); return (c ^ 0xffffffff) >>> 0; }
  function zip(files) {
    var parts = [], central = [], offset = 0, enc = new TextEncoder();
    files.forEach(function (f) {
      var name = enc.encode(f[0]), data = f[1], crc = crc32(data);
      var local = new DataView(new ArrayBuffer(30));
      local.setUint32(0, 0x04034b50, true); local.setUint16(4, 20, true); local.setUint32(14, crc, true);
      local.setUint32(18, data.length, true); local.setUint32(22, data.length, true); local.setUint16(26, name.length, true);
      parts.push(new Uint8Array(local.buffer), name, data);
      var cd = new DataView(new ArrayBuffer(46));
      cd.setUint32(0, 0x02014b50, true); cd.setUint16(4, 20, true); cd.setUint16(6, 20, true); cd.setUint32(16, crc, true);
      cd.setUint32(20, data.length, true); cd.setUint32(24, data.length, true); cd.setUint16(28, name.length, true); cd.setUint32(42, offset, true);
      central.push(new Uint8Array(cd.buffer), name);
      offset += 30 + name.length + data.length;
    });
    var size = central.reduce(function (s, p) { return s + p.length; }, 0), end = new DataView(new ArrayBuffer(22));
    end.setUint32(0, 0x06054b50, true); end.setUint16(8, files.length, true); end.setUint16(10, files.length, true);
    end.setUint32(12, size, true); end.setUint32(16, offset, true);
    return new Blob(parts.concat(central, [new Uint8Array(end.buffer)]), { type: 'application/zip' });
  }
})();
