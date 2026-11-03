/* screens/stats.js.
   Stats (/stats?period=7d|30d|12m|all): GET /api/stats?period=, and the taste profile (GET /api/taste). Loaded on the
   first visit, with stats.css (system.js registers /stats with need: App.needScript), so a start that never
   opens it never fetches either.
   Built with system.js's shared parts (App.settingsKit), which loads first. */
(function () {
  'use strict';
  var html = App.html, ui = App.ui, M = App.data, icon = App.icon, raw = App.raw, $ = App.$, $$ = App.$$;
  var P = App.player, K = App.settingsKit;
  var num = K.num, hours = K.hours, pct = K.pct, share = K.share, readError = K.readError;

  /* [id, tab label, phrase, days in the period (0: unknown, all time)] */
  var PERIODS = [['7d', '7 days', 'in the last 7 days', 7], ['30d', '30 days', 'in the last 30 days', 30], ['12m', '12 months', 'in the last 12 months', 365], ['all', 'All time', 'since you started', 0]];
  var HEAT = ['--lavender', '--lav', '--peri', '--peri-deep', '--indigo'];
  var NEEDED_MIN = 120;                           // stats and the taste profile need about two hours of listening
  var statsUi = { period: '7d', focusTab: false };

  /* tz = minutes east of UTC, so the clock and the periods follow the listener's own hours */
  function statsPath(p) { return '/api/stats?period=' + p + '&tz=' + -new Date().getTimezoneOffset(); }
  var TASTE = '/api/taste';
  function periodOf(id) { return PERIODS.filter(function (p) { return p[0] === id; })[0] || PERIODS[0]; }
  function hh(h) { return (h < 10 ? '0' : '') + h + ':00'; }
  function topValue(list) { return Math.max.apply(null, [1].concat(list.map(function (x) { return x[1]; }))); }
  /* the clock: plays per local hour, 24 values */
  function heatCells(clock) {
    var max = Math.max.apply(null, clock), total = clock.reduce(function (a, b) { return a + b; }, 0);
    return clock.map(function (v, h) {
      return { h: h, share: share(v, total), level: !max || v < max * .12 ? 0 : Math.min(4, Math.ceil(v / max * 4)) };
    });
  }
  /* the busiest two hours */
  function peakRange(clock) {
    var best = -1, at = 0;
    for (var h = 0; h < 24; h++) { var s = clock[h] + clock[(h + 1) % 24]; if (s > best) { best = s; at = h; } }
    return hh(at) + ' and ' + hh((at + 2) % 24);
  }
  function shareText(x) { return x && x < .005 ? 'under 1%' : pct(x); }
  function bar(value, max) { return html`<span class="sys-bar" aria-hidden="true"><i style="--p:${Math.max(.02, share(value, max))}"></i></span>`; }

  function statsTabs(cur) {
    var i = PERIODS.map(function (p) { return p[0]; }).indexOf(cur);
    return html`<div class="seg sys-period" role="tablist" aria-label="Period" style="--n:4;--i:${i}"><span class="seg__ind" aria-hidden="true"></span>${PERIODS.map(function (p, j) {
      return html`<button type="button" class="seg__btn" role="tab" id="sys-tab-${p[0]}" aria-selected="${j === i ? 'true' : 'false'}" aria-controls="sys-stats-panel" tabindex="${j === i ? '0' : '-1'}" data-period="${p[0]}">${p[1]}</button>`;
    })}</div>`;
  }
  function tile(ic, label, value, sub) {
    return html`<div class="card sys-tile"><span class="sys-tile__label">${icon(ic, null, 'ic-16')}${label}</span><span class="sys-tile__value num">${value}</span><span class="sys-tile__sub">${sub}</span></div>`;
  }

  function renderStats(ctx) {
    var per = periodOf(ctx.query.period || statsUi.period), path = statsPath(per[0]), r = M.get(path), d = r.data;
    statsUi.period = per[0];
    function page(lede, body, busy) {
      return html`<div class="sys-page sys-stats">
        <header class="sys-head"><div><h1 class="t-display">stats</h1><p class="sys-lede">${lede}</p></div>${statsTabs(per[0])}</header>
        <div id="sys-stats-panel" role="tabpanel" aria-labelledby="sys-tab-${per[0]}" ${busy ? raw('aria-busy="true"') : ''}>${body}</div></div>`;
    }
    if (r.loading) {
      return page('Counting your plays', html`<div class="sys-tiles">${[1, 2, 3, 4, 5].map(function () { return raw('<div class="card sys-tile" aria-hidden="true"><span class="skel skel--text" style="width:60%"></span><span class="skel skel--title" style="width:45%;height:26px;margin-top:8px"></span></div>'); })}</div>
        <div class="sys-grid">${ui.skeleton('card', 2)}</div>`, true);
    }
    if (r.error) return page('Couldn\'t count your plays.', App.errorCard(r.error, { path: path, title: 'Couldn\'t load your stats' }));
    if (d.minutes < NEEDED_MIN) return page('Stats fill in once you have listened for a while.', emptyStats(d));
    return page(hours(d.minutes) + ' across ' + num(d.plays) + ' plays ' + per[2] + ', mostly between ' + peakRange(d.clock) + '.', statsBody(d, per));
  }

  function emptyStats(d) {
    var p = Math.min(1, d.minutes / NEEDED_MIN);
    return html`<div class="card sys-empty-stats">${ui.emptyState({ art: 'wave', title: 'Not enough listening yet', text: 'Stats and your taste profile need about two hours of music. Play something and check back.', action: { label: 'Find something to play', href: '#/home' } })}
      <div class="sys-empty-stats__progress"><div class="sys-meter-label"><span>${hours(d.minutes)} of 2 h</span><span class="fg-2">${pct(p)}</span></div><div class="progress" role="progressbar" aria-label="Listening needed for stats" aria-valuenow="${Math.round(p * 100)}" aria-valuemin="0" aria-valuemax="100" style="--p:${p}"><i></i></div></div></div>`;
  }

  function statsBody(d, per) {
    var days = per[3], hr = d.hitRate, likedP = share(hr.liked, hr.recommended), savedP = share(hr.saved, hr.recommended);
    /* ids the cache can't resolve render nothing (docs/context/web.md) */
    var artists = d.topArtists.filter(function (x) { return M.artist(x[0]); });
    var tracks = d.topTracks.filter(function (x) { return M.track(x[0]); });
    var maxA = topValue(artists), maxT = topValue(tracks), maxG = topValue(d.topTags);
    var listening = d.minutes / 60 < 100 ? (d.minutes / 60).toFixed(1) : num(Math.round(d.minutes / 60));

    return html`<div class="sys-tiles">
        ${tile('clock', 'Listening', listening + ' h', days ? 'about ' + hours(Math.round(d.minutes / days)) + ' a day' : 'in total')}
        ${tile('play', 'Plays', num(d.plays), days ? num(Math.round(d.plays / days)) + ' a day' : 'in total')}
        ${tile('peer', 'Artists', num(d.artists), num(d.newArtists) + ' new to you')}
        ${tile('saved', 'Saves', num(d.saves), 'to your library')}
        ${tile('next', 'Skip rate', pct(d.skipRate), 'skipped before 30 s')}
      </div>
      <div class="sys-grid">
        <section class="card sys-card" aria-labelledby="sys-h-artists">${ui.labelRule('top artists', { id: 'sys-h-artists', action: { label: 'Library', href: '#/library/artists' } })}
          <ol class="sys-bars">${artists.map(function (x, i) {
            var a = M.artist(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(a, 40, { round: true })}
              <span class="sys-bars__main"><a class="sys-bars__name" href="#/artist/${a.id}">${a.name}</a>${bar(x[1], maxA)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card" aria-labelledby="sys-h-tracks">${ui.labelRule('top songs', { id: 'sys-h-tracks', action: tracks.length ? { label: 'Play all', act: 'sys-play-top' } : null })}
          <ol class="sys-bars" data-ids="${tracks.map(function (x) { return x[0]; }).join(',')}">${tracks.map(function (x, i) {
            var t = M.track(x[0]);
            return html`<li class="sys-bars__row"><span class="sys-bars__n num">${i + 1}</span>${ui.cover(t, 40)}
              <span class="sys-bars__main"><button class="sys-bars__name" data-act="play-track" data-id="${t.id}" aria-label="Play ${t.title} by ${t.artist}">${t.title}</button><span class="sys-bars__by"><a href="#/artist/${t.artistId}">${t.artist}</a></span>${bar(x[1], maxT)}</span><span class="sys-bars__val num">${num(x[1])}<span class="fg-2"> plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card sys-span" aria-labelledby="sys-h-clock">${ui.labelRule('listening clock', { id: 'sys-h-clock' })}
          <p class="sys-card__lede">When you listen, by hour of the day. Darker squares mean more plays.</p>
          <div class="sys-heat" role="img" aria-label="${'Listening clock. Most listening between ' + peakRange(d.clock) + '.'}">
            <span></span>${[0, 6, 12, 18].map(function (h) { return html`<span class="sys-heat__hour" style="grid-column:${h + 2} / span 6">${hh(h)}</span>`; })}
            <span class="sys-heat__day">All</span>${heatCells(d.clock).map(function (c) {
              return html`<i class="sys-heat__cell" style="--c:var(${HEAT[c.level]})" data-l="${c.level}" data-tip="${hh(c.h)}: ${shareText(c.share)} of your plays"></i>`;
            })}
          </div>
          <div class="sys-legend" aria-hidden="true"><span>less</span>${HEAT.map(function (v, i) { return html`<i class="sys-heat__cell" style="--c:var(${v})" data-l="${i}"></i>`; })}<span>more</span></div>
        </section>
        <section class="card sys-card" aria-labelledby="sys-h-tags">${ui.labelRule('top tags', { id: 'sys-h-tags' })}
          <ol class="sys-bars sys-bars--tags">${d.topTags.map(function (x) {
            return html`<li class="sys-bars__row">${ui.tagChip(x[0])}<span class="sys-bars__main">${bar(x[1], maxG)}</span><span class="sys-bars__val num">${x[1]}%<span class="vh"> of plays</span></span></li>`;
          })}</ol></section>
        <section class="card sys-card" aria-labelledby="sys-h-hit">${ui.labelRule('discovery hit rate', { id: 'sys-h-hit' })}
          <p class="sys-hit__big"><span class="num">${pct(likedP)}</span> of the picks you played became likes</p>
          <dl class="sys-funnel">
            <div><dt>Recommended and played</dt><dd>${bar(1, 1)}<span class="num">${num(hr.recommended)}</span></dd></div>
            <div><dt>Liked</dt><dd>${bar(likedP, 1)}<span class="num">${num(hr.liked)}</span></dd></div>
            <div><dt>Saved</dt><dd>${bar(savedP, 1)}<span class="num">${num(hr.saved)}</span></dd></div>
          </dl>
          <div class="inset sys-newart"><span class="sys-newart__n num">${num(d.newArtists)}</span><span>new artists ${per[2]}.</span></div>
        </section>
        ${tasteCard()}
      </div>`;
  }

  /* discovery's taste profile (GET /api/taste): { artists: [[id, weight 0-1]], tags: [[name, weight]] }, its own read */
  function tasteCard() {
    var r = M.get(TASTE);
    function card(body, action) {
      return html`<section class="card sys-card sys-span" aria-labelledby="sys-h-taste" id="sys-taste">
        ${ui.labelRule('your taste profile', { id: 'sys-h-taste', action: action })}${body}</section>`;
    }
    if (r.loading) return card(ui.skeleton('row', 3));
    if (r.error) return card(readError(r.error, TASTE, 'Couldn\'t load your taste profile'));
    var taste = r.data, artists = taste.artists.filter(function (x) { return M.artist(x[0]); }), empty = !artists.length && !taste.tags.length;
    function col(title, list, kind) {
      return html`<div class="sys-taste__col">${ui.labelRule(title, { level: 'h3', sm: true })}<ol class="sys-bars sys-bars--taste">${list.map(function (x) {
        var name = kind === 'artist' ? M.artist(x[0]).name : x[0], w = Math.round(x[1] * 100);
        var href = kind === 'artist' ? '#/artist/' + x[0] : '#/tag/' + encodeURIComponent(x[0]);
        return html`<li class="sys-bars__row"><span class="sys-bars__main"><a class="sys-bars__name" href="${href}">${name}</a>${bar(x[1], 1)}</span>
          <span class="sys-bars__val num">${w}<span class="vh"> out of 100</span></span>
          <button class="btn-icon btn-icon--bare btn-icon--sm" data-act="sys-taste-why" data-kind="${kind}" data-id="${x[0]}" data-weight="${w}" aria-haspopup="dialog" aria-label="Why ${name} weighs ${w}" data-tip="Why this weight">${icon('info', null, 'ic-16')}</button></li>`;
      })}</ol></div>`;
    }
    var body = empty
      ? ui.emptyState({ art: 'moon', title: 'No taste profile yet', text: 'Home learns from what you play, like and save next.' })
      : html`<p class="sys-card__lede">This is what Home leans on. Weights come from plays, likes, saves and follows over the last 90 days; 100 is your strongest signal. Use the info buttons to see why, or turn something down.</p>
        <div class="sys-taste">${col('artists', artists, 'artist')}${col('tags', taste.tags, 'tag')}</div>`;
    return card(body, empty ? null : { label: 'Reset taste', act: 'sys-taste-reset' });
  }

  function mountStats(ctx) {
    var tl = $('.sys-period', ctx.el); if (!tl) return;
    function choose(btn) {
      statsUi.period = btn.getAttribute('data-period'); statsUi.focusTab = true;
      history.replaceState(history.state, '', '#/stats?period=' + statsUi.period);
      App.refresh();
    }
    ctx.on(tl, 'click', function (e) { var b = e.target.closest('[role="tab"]'); if (b) choose(b); });
    ctx.on(tl, 'keydown', function (e) {
      var tabs = $$('[role="tab"]', tl), i = tabs.indexOf(document.activeElement);
      var d = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
      if (e.key === 'Home') d = -i; if (e.key === 'End') d = tabs.length - 1 - i;
      if (d == null || i < 0) return;
      e.preventDefault(); choose(tabs[(i + d + tabs.length) % tabs.length]);
    });
    if (statsUi.focusTab) { statsUi.focusTab = false; var sel = $('[aria-selected="true"]', tl); if (sel) sel.focus(); }
  }

  /* weights change with feedback and reset: refetch the taste read in place */
  function refreshTaste() { M.reload(TASTE); }
  App.action('sys-play-top', function (el) {
    var ids = $('[data-ids]', el.closest('.sys-card')).getAttribute('data-ids').split(',');
    P.playList(ids, 0); App.toast('Playing your top songs', { icon: 'play' });
  });
  App.action('sys-taste-why', function (el) {
    var kind = el.getAttribute('data-kind'), id = el.getAttribute('data-id'), w = el.getAttribute('data-weight');
    var per = periodOf(statsUi.period), d = M.peek(statsPath(per[0])), text, name;
    function inTop(list) { return (list.filter(function (x) { return x[0] === id; })[0] || [])[1]; }
    if (kind === 'artist') {
      var plays = inTop(d.topArtists);
      name = M.artist(id).name;
      text = (plays ? num(plays) + ' plays ' + per[2] + '. ' : '') + (App.isFollowed(id) ? 'You follow them. ' : '') + 'Shows up in Home as "Because you like ' + name + '" and seeds ' + name + ' radio.';
    } else {
      var tagShare = inTop(d.topTags);
      name = id;
      text = (tagShare ? tagShare + '% of your plays ' + per[2] + ' are tagged ' + id + '. ' : '') + (App.isFollowed(id) ? 'You follow the tag. ' : '') + 'Home adds fresh releases and picks from this tag.';
    }
    App.popover(el, html`<div class="popover__head">${name}: ${w}</div><div class="popover__body"><p>${text}</p><p class="t-micro fg-2" style="margin-top:6px">via your listening history</p></div>
      <div class="popover__actions"><button class="btn btn--sm" data-act="sys-taste-less" data-id="${id}" data-name="${name}">${icon('minus')}Less of this</button><a class="btn btn--sm" href="${kind === 'artist' ? '#/artist/' + id : '#/tag/' + encodeURIComponent(id)}">Open ${kind}</a></div>`, { label: 'Why ' + name + ' has this weight' });
  });
  /* "less of this" is the same feedback Home sends; the new weight comes back with the taste read */
  App.action('sys-taste-less', function (el) {
    var id = el.getAttribute('data-id'), name = el.getAttribute('data-name');
    App.closePopover();
    M.api('/api/feedback', { method: 'POST', body: { kind: 'less-like', id: id } }).then(function () {
      refreshTaste();
      App.toast('Turned down ' + name, { icon: 'minus' });
    }, function (err) { App.toast('Couldn\'t turn down ' + name + ': ' + err.message, { kind: 'error' }); });
  });
  App.action('sys-taste-reset', function () {
    App.confirm({ title: 'reset your taste profile?', text: 'Home forgets the weights for every artist and tag and relearns from what you play next. Your plays, likes, saves and follows stay as they are.', confirm: 'Reset taste', danger: true }, function () {
      M.api('/api/taste/reset', { method: 'POST' }).then(function () {
        refreshTaste();
        App.toast('Taste profile reset', { icon: 'refresh' });
      }, function (err) { App.toast('Couldn\'t reset the taste profile: ' + err.message, { kind: 'error' }); });
    });
  });

  App.route('/stats', { render: renderStats, mount: mountStats, nav: 'stats', title: 'Stats', place: function () { return App.place('stats', '', 'your top songs'); } });
})();
