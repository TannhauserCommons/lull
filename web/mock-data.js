/* lull prototype mock data. Everything here is fictional: artists, tracks, peers, listeners.
   Exposes window.MOCK (read-only by convention; app state lives in App.store).
   PROTO: replace with the real API (DESIGN.md section 8). Ids are stable strings: ar-*, al-*, t-*, pl-*, tag ids are the tag name. */
window.MOCK = (function () {
  'use strict';

  // ---------- formats ----------
  /* khz = sample rate, bits = bit depth (lossless only), kbps = average bitrate, mbPerMin = average size */
  var FORMATS = {
    'FLAC 24/96':   { codec: 'FLAC', label: 'FLAC 24/96',   detail: '24-bit, 96 kHz',  lossless: true,  mbPerMin: 33.0, kbps: 2800, khz: 96,   bits: 24 },
    'FLAC 16/44.1': { codec: 'FLAC', label: 'FLAC 16/44.1', detail: '16-bit, 44.1 kHz', lossless: true, mbPerMin: 7.8,  kbps: 1040, khz: 44.1, bits: 16 },
    'MP3 320':      { codec: 'MP3',  label: 'MP3 320',      detail: '320 kbps CBR',    lossless: false, mbPerMin: 2.4,  kbps: 320,  khz: 44.1, bits: null },
    'MP3 V0':       { codec: 'MP3',  label: 'MP3 V0',       detail: '~245 kbps VBR',   lossless: false, mbPerMin: 1.85, kbps: 245,  khz: 44.1, bits: null },
    'OPUS 160':     { codec: 'OPUS', label: 'OPUS 160',     detail: '160 kbps',        lossless: false, mbPerMin: 1.2,  kbps: 160,  khz: 48,   bits: null }
  };

  // ---------- tags ----------
  var tags = [
    { id: 'dream-gaze', name: 'dream-gaze', followed: true, related: ['shoegaze', 'slowcore', 'bedroom pop'],
      desc: 'Hazy, reverb-soaked guitar pop that sits between dream pop and shoegaze: soft vocals, slow tempos, big washes of sound.' },
    { id: 'city-pop revival', name: 'city-pop revival', followed: true, related: ['shibuya-kei', 'bedroom pop'],
      desc: 'Glossy, bass-forward pop that borrows the late-night shine of 1980s city pop: electric piano, bright horns, night-drive moods.' },
    { id: 'chiptune-folk', name: 'chiptune-folk', followed: true, related: ['bedroom pop'],
      desc: 'Acoustic folk songwriting played partly on sound chips and handheld consoles: square-wave fiddles, pocket-synth drones.' },
    { id: 'ambient jazz', name: 'ambient jazz', followed: true, related: ['slowcore'],
      desc: 'Spacious jazz with long tones and soft brushes; more about atmosphere than solos.' },
    { id: 'slowcore', name: 'slowcore', followed: false, related: ['dream-gaze', 'ambient jazz'],
      desc: 'Slow, quiet, patient rock: sparse drums, low voices, songs that take their time.' },
    { id: 'shibuya-kei', name: 'shibuya-kei', followed: false, related: ['city-pop revival', 'bedroom pop'],
      desc: 'Playful collage pop mixing bossa nova, lounge and sampled kitsch.' },
    { id: 'bedroom pop', name: 'bedroom pop', followed: false, related: ['dream-gaze', 'chiptune-folk'],
      desc: 'Home-recorded pop with soft vocals and lo-fi warmth.' },
    { id: 'shoegaze', name: 'shoegaze', followed: false, related: ['dream-gaze'],
      desc: 'Loud, blurred guitars and buried vocals; the noisier parent of dream-gaze.' }
  ];

  // ---------- artists ----------
  var artists = [
    { id: 'ar-glass-harbour', name: 'Glass Harbour', tags: ['slowcore', 'dream-gaze'], followed: true, listeners: 18400,
      bio: 'A three-piece that records in a converted boathouse. Slow songs about tides, ropes and patience.' },
    { id: 'ar-velvet-orchard', name: 'Velvet Orchard', tags: ['dream-gaze', 'shoegaze', 'bedroom pop'], followed: true, listeners: 42100,
      bio: 'Bedroom dream-gaze built from a borrowed four-track and a radio that only half works.' },
    { id: 'ar-juniper-coast', name: 'Juniper Coast', tags: ['city-pop revival'], followed: false, listeners: 26800,
      bio: 'Night-drive pop with electric piano and a rhythm section that never stops smiling.' },
    { id: 'ar-tiny-orbit', name: 'Tiny Orbit', tags: ['chiptune-folk', 'bedroom pop'], followed: true, listeners: 9300,
      bio: 'Folk songs played on a fiddle and a handheld console, recorded on the bus.' },
    { id: 'ar-neon-wren', name: 'Neon Wren', tags: ['ambient jazz', 'slowcore'], followed: true, listeners: 15200,
      bio: 'A quartet that plays harbour-side jazz at half speed.' },
    { id: 'ar-moonpail', name: 'Moonpail', tags: ['bedroom pop', 'dream-gaze'], followed: false, listeners: 31700,
      bio: 'Greenhouse pop: soft synths, close vocals, rain on the roof.' },
    { id: 'ar-sora-tide', name: 'Sora Tide', tags: ['shibuya-kei', 'city-pop revival'], followed: false, listeners: 12900,
      bio: 'Collage pop made from seaside arcade samples and bossa guitar.' },
    { id: 'ar-pastel-relay', name: 'Pastel Relay', tags: ['city-pop revival'], followed: false, listeners: 22600,
      bio: 'Glossy late-night pop with a horn section and a fondness for escalators.' },
    { id: 'ar-small-comet-club', name: 'Small Comet Club', tags: ['shibuya-kei', 'bedroom pop'], followed: false, listeners: 7400,
      bio: 'Cassette collage pop from a club that meets in a garden shed.' },
    { id: 'ar-kiyo-lanterns', name: 'Kiyo Lanterns', tags: ['chiptune-folk'], followed: false, listeners: 5800,
      bio: 'Square-wave reels and paper-lantern jigs.' },
    { id: 'ar-marble-static', name: 'Marble Static', tags: ['dream-gaze', 'shoegaze'], followed: false, listeners: 11300,
      bio: 'Weather-obsessed dream-gaze: every song is a forecast.' },
    { id: 'ar-fern-arcade', name: 'Fern Arcade', tags: ['slowcore', 'ambient jazz'], followed: false, listeners: 6100,
      bio: 'Slowcore for the last bus home.' }
  ];

  // ---------- albums (track lists inline: [title, "m:ss"]) ----------
  // saved: 'all' | 'partial' (first `savedCount` tracks) | 'none' (Soulseek only)
  var albumDefs = [
    { id: 'al-lantern-season', title: 'Lantern Season', artistId: 'ar-glass-harbour', year: 2026, type: 'album', released: '2026-10-29',
      label: 'Breakwater Tapes', format: 'FLAC 16/44.1', saved: 'all', isNew: true, tags: ['slowcore', 'dream-gaze'],
      tracks: [['Lantern Season', '4:12'], ['Harbour Lights Out', '3:48'], ['Small Hours Ferry', '5:06'], ['Salt on the Window', '3:31'],
        ['The Long Way Round', '4:44'], ['Weathervane', '3:09'], ['Postcards from the Breakwater', '4:58'], ['Tide Tables', '6:21']] },
    { id: 'al-satellite-porch', title: 'Satellite Porch', artistId: 'ar-velvet-orchard', year: 2025, type: 'EP', released: '2025-06-13',
      label: 'self-released', format: 'FLAC 16/44.1', saved: 'none', tags: ['dream-gaze', 'shoegaze', 'bedroom pop'],
      tracks: [['Paper Moon Static', '3:52'], ['Borrowed Tune', '3:27'], ['Antenna Weather', '4:05'], ['June in the Gutter', '3:14'],
        ['Count the Satellites', '4:40']] },
    { id: 'al-snow-antenna', title: 'Snow on the Antenna', artistId: 'ar-juniper-coast', year: 2026, type: 'EP', released: '2026-10-27',
      label: 'Coupe Records', format: 'MP3 320', saved: 'none', isNew: true, tags: ['city-pop revival'],
      tracks: [['Snow on the Antenna', '3:58'], ['Night Drive Coupe', '4:21'], ['Vending Machine Glow', '3:36'], ['Rooftop Weather Report', '4:02']] },
    { id: 'al-pocket-valentine', title: 'Pocket Valentine', artistId: 'ar-tiny-orbit', year: 2026, type: 'single', released: '2026-10-24',
      label: 'self-released', format: 'MP3 V0', saved: 'none', isNew: true, tags: ['chiptune-folk'],
      tracks: [['Pocket Valentine', '2:58'], ['Pocket Valentine (8-bit lullaby)', '3:12']] },
    { id: 'al-harbour-lights', title: 'Harbour Lights', artistId: 'ar-neon-wren', year: 2026, type: 'album', released: '2026-10-20',
      label: 'Lamplight Jazz', format: 'FLAC 24/96', saved: 'partial', savedCount: 3, isNew: true, tags: ['ambient jazz'],
      tracks: [['Harbour Lights', '5:42'], ['Brass in the Fog', '6:10'], ['Lamplighter', '4:55'], ['Slow Boats', '7:03'],
        ['Blue Hour Standard', '5:18'], ['Night Ferry Coda', '3:40']] },
    { id: 'al-glasshouse-bloom', title: 'Glasshouse Bloom', artistId: 'ar-moonpail', year: 2024, type: 'album', released: '2024-03-08',
      label: 'Greenroom', format: 'FLAC 16/44.1', saved: 'all', tags: ['bedroom pop', 'dream-gaze'],
      tracks: [['Glasshouse Bloom', '3:22'], ['Greenhouse Radio', '3:05'], ['Fern Light', '2:58'], ['Watering Can Heart', '3:41'],
        ['Condensation', '4:10'], ['Bloom Again', '3:33']] },
    { id: 'al-low-tide-radio', title: 'Low Tide Radio', artistId: 'ar-sora-tide', year: 2023, type: 'album', released: '2023-07-21',
      label: 'Arcade Sound', format: 'MP3 320', saved: 'none', tags: ['shibuya-kei', 'city-pop revival'],
      tracks: [['Low Tide Radio', '3:16'], ['Seaside Arcade', '2:49'], ['Postcard Pop', '3:02'], ['Umbrella Choreography', '3:27'],
        ['Last Train Bossa', '4:08']] },
    { id: 'al-marble-soda-club', title: 'Marble Soda Club', artistId: 'ar-pastel-relay', year: 2022, type: 'album', released: '2022-09-02',
      label: 'Escalator Music', format: 'FLAC 16/44.1', saved: 'all', tags: ['city-pop revival'],
      tracks: [['Marble Soda Club', '4:01'], ['Neon Receipt', '3:44'], ['Weekend Escalator', '3:58'], ['Sunset Mixtape', '4:36'],
        ['Lemonade Telephone', '3:29'], ['City Lights Encore', '5:02']] },
    { id: 'al-cassette-garden', title: 'Cassette Garden', artistId: 'ar-small-comet-club', year: 2025, type: 'album', released: '2025-04-18',
      label: 'Shed Sessions', format: 'OPUS 160', saved: 'none', tags: ['shibuya-kei', 'bedroom pop'],
      tracks: [['Cassette Garden', '3:11'], ['Polaroid Weather', '2:54'], ['Comet Club Theme', '2:37'], ['Daisy Rewind', '3:20'],
        ['Side B Forever', '4:12']] },
    { id: 'al-satellite-picnic', title: 'Satellite Picnic', artistId: 'ar-kiyo-lanterns', year: 2024, type: 'album', released: '2024-08-30',
      label: 'Pocket Folk', format: 'MP3 V0', saved: 'all', tags: ['chiptune-folk'],
      tracks: [['Satellite Picnic', '3:34'], ['Paper Lantern Reel', '3:02'], ['Thimble Synth', '2:47'], ['Meadow 1-Up', '3:15'],
        ['Starlight Jig', '3:49']] },
    { id: 'al-quiet-weather', title: 'Quiet Weather Machine', artistId: 'ar-marble-static', year: 2021, type: 'album', released: '2021-11-05',
      label: 'Barometer', format: 'FLAC 24/96', saved: 'all', tags: ['dream-gaze', 'shoegaze'],
      tracks: [['Quiet Weather Machine', '5:20'], ['Drizzle Chorus', '4:46'], ['Static Bloom', '6:02'], ['Overcast Hymn', '5:33']] },
    { id: 'al-night-bus', title: 'Night Bus Lullaby', artistId: 'ar-fern-arcade', year: 2025, type: 'album', released: '2025-01-24',
      label: 'Depot', format: 'MP3 V0', saved: 'none', tags: ['slowcore', 'ambient jazz'],
      tracks: [['Night Bus Lullaby', '4:28'], ['Last Stop Waltz', '5:11'], ['Window Seat Fog', '4:03'], ['Depot Lights', '6:14']] }
  ];

  // ---------- small deterministic PRNG so the data is identical on every load ----------
  var seed = 7;
  function rand() { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; }
  function pick(list) { return list[Math.floor(rand() * list.length)]; }
  function secs(mss) { var p = mss.split(':'); return +p[0] * 60 + +p[1]; }

  var PEERS = ['otter-lamp', 'tin-satellite', 'mossbank', 'quiet.fm', 'parlour-moth', 'east-pier', 'lowtide_rec',
    'cobalt-attic', 'fernwire', 'paperkite', 'velvet-dial', 'moth-radio', 'cardigan-cloud', 'bluebottle'];
  var ALT_FORMATS = ['FLAC 16/44.1', 'FLAC 24/96', 'MP3 320', 'MP3 V0', 'OPUS 160'];

  /* Score 0-100 from four factors (the brief's ranking): format, speed, free slot, queue length. */
  function scoreSource(fmt, speed, freeSlot, queue) {
    var f = { 'FLAC 24/96': 38, 'FLAC 16/44.1': 40, 'MP3 320': 28, 'MP3 V0': 24, 'OPUS 160': 20 }[fmt];
    var s = Math.min(30, Math.round(speed * 9));
    var sl = freeSlot ? 15 : 4;
    var q = Math.max(0, 15 - queue * 2);
    return { score: f + s + sl + q, factors: { format: f, speed: s, slot: sl, queue: q } };
  }
  function makeSources(track, n) {
    var used = {}, list = [];
    for (var i = 0; i < n; i++) {
      var peer; do { peer = pick(PEERS); } while (used[peer]); used[peer] = 1;
      var fmt = i === 0 ? track.format : (rand() < .5 ? track.format : pick(ALT_FORMATS));
      var speed = Math.round((0.3 + rand() * 3.2) * 10) / 10;           // MB/s
      var freeSlot = rand() > .3;
      var queue = freeSlot ? 0 : 1 + Math.floor(rand() * 9);
      var sc = scoreSource(fmt, speed, freeSlot, queue);
      list.push({ peer: peer, format: fmt, sizeMB: Math.round(FORMATS[fmt].mbPerMin * track.duration / 60 * 10) / 10,
        speed: speed, freeSlot: freeSlot, queue: queue, score: sc.score, factors: sc.factors,
        path: track.artist + '/' + track.album + '/' + String(track.no).padStart(2, '0') + ' - ' + track.title + '.' + FORMATS[fmt].codec.toLowerCase() });
    }
    list.sort(function (a, b) { return b.score - a.score; });
    return list;
  }

  // ---------- build albums + tracks ----------
  var artistById = {}; artists.forEach(function (a) { artistById[a.id] = a; });
  var albums = [], tracks = [];
  albumDefs.forEach(function (def) {
    var album = {
      id: def.id, title: def.title, artistId: def.artistId, artist: artistById[def.artistId].name, year: def.year,
      type: def.type, released: def.released, label: def.label, format: def.format, tags: def.tags,
      isNew: !!def.isNew, saved: def.saved, trackIds: [], duration: 0, sizeMB: 0
    };
    def.tracks.forEach(function (t, i) {
      var duration = secs(t[1]);
      var saved = def.saved === 'all' || (def.saved === 'partial' && i < def.savedCount);
      var track = {
        id: 't-' + def.id.slice(3) + '-' + (i + 1), title: t[0], artistId: def.artistId, artist: album.artist,
        albumId: def.id, album: def.title, year: def.year, no: i + 1, disc: 1, duration: duration,
        format: def.format, saved: saved, liked: false, plays: Math.floor(rand() * 40), tags: def.tags
      };
      track.sources = makeSources(track, 3 + Math.floor(rand() * 5));
      track.sizeMB = track.sources[0].sizeMB;
      /* file facts for format lines ("FLAC :: 44.1 kHz, 1040 kbps, 30.2 MB"); the bitrate varies a little per file
         (derived from the title, not rand(), so every other mock value stays the same) */
      var fm = FORMATS[track.format];
      track.khz = fm.khz; track.bits = fm.bits;
      track.kbps = fm.kbps === 320 ? 320 : Math.round(fm.kbps * (0.92 + ((t[0].length * 37 + t[0].charCodeAt(0)) % 17) / 100));
      album.trackIds.push(track.id);
      album.duration += duration; album.sizeMB += track.sizeMB;
      tracks.push(track);
    });
    album.sizeMB = Math.round(album.sizeMB);
    albums.push(album);
  });
  // a two-disc feel for the longest album
  tracks.filter(function (t) { return t.albumId === 'al-lantern-season' && t.no > 5; }).forEach(function (t) { t.disc = 2; });

  var trackById = {}; tracks.forEach(function (t) { trackById[t.id] = t; });
  var albumById = {}; albums.forEach(function (a) { albumById[a.id] = a; });
  var tagById = {}; tags.forEach(function (t) { tagById[t.id] = t; });

  ['t-satellite-porch-1', 't-lantern-season-1', 't-glasshouse-bloom-1', 't-marble-soda-club-1', 't-satellite-picnic-3',
    't-quiet-weather-3', 't-harbour-lights-1', 't-snow-antenna-1', 't-lantern-season-4', 't-low-tide-radio-2']
    .forEach(function (id) { trackById[id].liked = true; });
  // a library duplicate: the Soulseek "Lantern Season" result matches the saved one (duplicate dialog demo)
  trackById['t-lantern-season-1'].sources[0].format = 'FLAC 24/96';

  // ---------- discovery feed ----------
  var feed = {
    greeting: 'good evening',
    updated: '6 min ago',              // when the feed was last rebuilt (title-bar meta)
    summary: '3 new releases from artists you follow, and fresh picks from dream-gaze.',
    newReleases: ['al-lantern-season', 'al-snow-antenna', 'al-pocket-valentine', 'al-harbour-lights'],
    /* reason kinds: like (heart) | tag (sparkle) | listeners (person) | fresh (bell). `via` = data source shown in the popover. */
    picks: [
      { trackId: 't-glasshouse-bloom-1', reasons: [{ kind: 'like', name: 'Velvet Orchard', ref: 'ar-velvet-orchard' }, { kind: 'tag', name: 'bedroom pop', ref: 'bedroom pop' }], via: 'ListenBrainz' },
      { trackId: 't-low-tide-radio-1', reasons: [{ kind: 'tag', name: 'dream-gaze', ref: 'dream-gaze' }], via: 'MusicBrainz tags' },
      { trackId: 't-marble-soda-club-1', reasons: [{ kind: 'like', name: 'Tiny Orbit', ref: 'ar-tiny-orbit' }], via: 'Last.fm similar artists' },
      { trackId: 't-night-bus-1', reasons: [{ kind: 'tag', name: 'ambient jazz', ref: 'ambient jazz' }], via: 'ListenBrainz' },
      { trackId: 't-cassette-garden-1', reasons: [{ kind: 'listeners', name: 'Juniper Coast', ref: 'ar-juniper-coast' }], via: 'Last.fm' },
      { trackId: 't-satellite-picnic-1', reasons: [{ kind: 'tag', name: 'chiptune-folk', ref: 'chiptune-folk' }], via: 'Discogs styles' },
      { trackId: 't-quiet-weather-2', reasons: [{ kind: 'fresh', name: 'Marble Static', ref: 'ar-marble-static' }], via: 'MusicBrainz releases', noSource: true }
    ],
    radioStarters: [
      { id: 'rs-velvet', title: 'Velvet Orchard radio', sub: 'from your most played artist', seed: { kind: 'artist', id: 'ar-velvet-orchard' } },
      { id: 'rs-dreamgaze', title: 'dream-gaze radio', sub: 'a tag you follow', seed: { kind: 'tag', id: 'dream-gaze' } },
      { id: 'rs-papermoon', title: 'More like Paper Moon Static', sub: 'played 6 times this week', seed: { kind: 'track', id: 't-satellite-porch-1' } },
      { id: 'rs-citypop', title: 'city-pop revival radio', sub: 'a tag you follow', seed: { kind: 'tag', id: 'city-pop revival' } }
    ],
    jumpBackIn: [
      { kind: 'album', id: 'al-glasshouse-bloom' }, { kind: 'playlist', id: 'pl-sleepy-sunday' },
      { kind: 'album', id: 'al-marble-soda-club' }, { kind: 'artist', id: 'ar-neon-wren' }, { kind: 'album', id: 'al-quiet-weather' }
    ],
    followedTags: ['dream-gaze', 'city-pop revival', 'chiptune-folk', 'ambient jazz'],
    coldStartTags: ['dream-gaze', 'city-pop revival', 'chiptune-folk', 'ambient jazz', 'slowcore', 'shibuya-kei', 'bedroom pop', 'shoegaze']
  };

  // ---------- playlists ----------
  function pl(id, name, desc, created, updated, ids) {
    return { id: id, name: name, desc: desc, created: created, updated: updated,
      items: ids.map(function (tid, i) { return { trackId: tid, added: addDays(updated, -i * 3) }; }) };
  }
  function addDays(iso, n) { var d = new Date(iso + 'T12:00:00Z'); d.setUTCDate(d.getUTCDate() + n); return d.toISOString().slice(0, 10); }
  var playlists = [
    pl('pl-sleepy-sunday', 'sleepy sunday', 'Slow records for a slow morning. Kettle on, phone off.', '2025-11-02', '2026-10-26',
      ['t-lantern-season-1', 't-glasshouse-bloom-3', 't-harbour-lights-3', 't-quiet-weather-4', 't-night-bus-3', 't-satellite-porch-2',
        't-lantern-season-6', 't-glasshouse-bloom-6', 't-harbour-lights-5']),
    pl('pl-3am-window', '3am window seat', 'Night bus, rain on the glass, nobody else awake.', '2026-01-15', '2026-10-19',
      ['t-night-bus-1', 't-night-bus-3', 't-harbour-lights-6', 't-satellite-porch-1', 't-lantern-season-3', 't-quiet-weather-1',
        't-satellite-porch-5', 't-harbour-lights-4']),
    pl('pl-strawberry-static', 'strawberry static', 'Sugar-coated city pop and fizzy shibuya-kei.', '2026-04-01', '2026-10-28',
      ['t-marble-soda-club-1', 't-low-tide-radio-1', 't-cassette-garden-2', 't-snow-antenna-3', 't-marble-soda-club-5', 't-low-tide-radio-4',
        't-cassette-garden-4', 't-snow-antenna-1']),
    pl('pl-pocket-folk', 'pocket folk', 'Square waves and fiddles. Good for walking.', '2026-08-09', '2026-10-24',
      ['t-satellite-picnic-1', 't-pocket-valentine-1', 't-satellite-picnic-2', 't-satellite-picnic-4', 't-pocket-valentine-2'])
  ];

  // ---------- history (newest first, by day) with scrobble status: done | pending | short ----------
  var history = [
    { day: 'Today', items: [
      { trackId: 't-satellite-porch-1', at: '21:42', scrobble: 'pending' },
      { trackId: 't-satellite-porch-2', at: '21:38', scrobble: 'pending' },
      { trackId: 't-lantern-season-1', at: '21:20', scrobble: 'done' },
      { trackId: 't-pocket-valentine-2', at: '21:17', scrobble: 'short' },
      { trackId: 't-glasshouse-bloom-1', at: '20:58', scrobble: 'done' } ] },
    { day: 'Yesterday', items: [
      { trackId: 't-marble-soda-club-1', at: '23:05', scrobble: 'done' },
      { trackId: 't-marble-soda-club-2', at: '23:01', scrobble: 'done' },
      { trackId: 't-low-tide-radio-1', at: '22:44', scrobble: 'done' },
      { trackId: 't-night-bus-1', at: '08:12', scrobble: 'done' } ] },
    { day: 'Wed 28 Oct', items: [
      { trackId: 't-quiet-weather-1', at: '19:30', scrobble: 'done' },
      { trackId: 't-harbour-lights-1', at: '19:24', scrobble: 'done' },
      { trackId: 't-satellite-picnic-3', at: '07:55', scrobble: 'done' } ] }
  ];

  // ---------- stats ----------
  function clock(peakA, peakB) {
    var out = [];
    for (var h = 0; h < 24; h++) {
      var v = Math.exp(-Math.pow(h - peakA, 2) / 6) * 60 + Math.exp(-Math.pow(h - peakB, 2) / 4) * 40 + rand() * 6;
      out.push(Math.round(v));
    }
    return out;
  }
  var stats = {
    periods: ['7d', '30d', '12m', 'all'],
    byPeriod: {
      '7d': { minutes: 742, plays: 211, artists: 38, saves: 9, skipRate: .14, newArtists: 6, hitRate: { recommended: 41, liked: 9, saved: 5 },
        topArtists: [['ar-velvet-orchard', 34], ['ar-glass-harbour', 27], ['ar-neon-wren', 19], ['ar-moonpail', 15], ['ar-pastel-relay', 12]],
        topTracks: [['t-satellite-porch-1', 11], ['t-lantern-season-1', 9], ['t-glasshouse-bloom-1', 7], ['t-harbour-lights-1', 6], ['t-marble-soda-club-1', 5]],
        topTags: [['dream-gaze', 31], ['slowcore', 22], ['ambient jazz', 17], ['city-pop revival', 15], ['chiptune-folk', 9]],
        clock: clock(22, 8) },
      '30d': { minutes: 3120, plays: 884, artists: 96, saves: 31, skipRate: .17, newArtists: 21, hitRate: { recommended: 164, liked: 33, saved: 18 },
        topArtists: [['ar-glass-harbour', 102], ['ar-velvet-orchard', 97], ['ar-moonpail', 64], ['ar-neon-wren', 58], ['ar-sora-tide', 41]],
        topTracks: [['t-lantern-season-1', 28], ['t-satellite-porch-1', 26], ['t-glasshouse-bloom-3', 19], ['t-marble-soda-club-2', 17], ['t-night-bus-1', 15]],
        topTags: [['dream-gaze', 29], ['slowcore', 20], ['city-pop revival', 18], ['ambient jazz', 16], ['bedroom pop', 12]],
        clock: clock(21, 9) },
      '12m': { minutes: 31800, plays: 9020, artists: 412, saves: 288, skipRate: .19, newArtists: 160, hitRate: { recommended: 1820, liked: 301, saved: 190 },
        topArtists: [['ar-glass-harbour', 820], ['ar-moonpail', 760], ['ar-velvet-orchard', 701], ['ar-pastel-relay', 522], ['ar-neon-wren', 498]],
        topTracks: [['t-glasshouse-bloom-1', 141], ['t-lantern-season-1', 120], ['t-marble-soda-club-1', 112], ['t-quiet-weather-1', 98], ['t-satellite-porch-1', 96]],
        topTags: [['dream-gaze', 27], ['city-pop revival', 21], ['slowcore', 19], ['bedroom pop', 15], ['ambient jazz', 12]],
        clock: clock(22, 8) },
      'all': { minutes: 52400, plays: 14880, artists: 590, saves: 402, skipRate: .2, newArtists: 590, hitRate: { recommended: 2600, liked: 420, saved: 260 },
        topArtists: [['ar-moonpail', 1310], ['ar-glass-harbour', 1190], ['ar-velvet-orchard', 980], ['ar-pastel-relay', 900], ['ar-marble-static', 640]],
        topTracks: [['t-glasshouse-bloom-1', 260], ['t-marble-soda-club-1', 201], ['t-lantern-season-1', 150], ['t-quiet-weather-1', 149], ['t-glasshouse-bloom-4', 133]],
        topTags: [['dream-gaze', 26], ['city-pop revival', 22], ['bedroom pop', 18], ['slowcore', 17], ['ambient jazz', 10]],
        clock: clock(22, 8) }
    },
    taste: {
      artists: [['ar-velvet-orchard', .92], ['ar-glass-harbour', .88], ['ar-moonpail', .71], ['ar-neon-wren', .66], ['ar-tiny-orbit', .52], ['ar-pastel-relay', .44]],
      tags: [['dream-gaze', .9], ['slowcore', .74], ['ambient jazz', .63], ['city-pop revival', .58], ['chiptune-folk', .41], ['shibuya-kei', .22]]
    }
  };

  // ---------- lyrics: LRCLIB-shaped records keyed by track id ----------
  // syncedLyrics = LRC text; wordSync = per-word start times for the first sung lines (optional enhancement)
  var lyrics = {
    't-satellite-porch-1': {
      instrumental: false, attribution: 'LRCLIB',
      syncedLyrics: [
        '[00:12.40] the porch light hums a borrowed tune',
        '[00:18.90] we fold the evening into June',
        '[00:25.10] paper moon, static on the line',
        '[00:31.60] hold the antenna, you\'ll be fine',
        '[00:38.20] count the satellites by name',
        '[00:44.70] none of them are quite the same',
        '[00:51.00] ',
        '[00:58.30] your radio is out of range',
        '[01:04.80] but every crackle sounds like change',
        '[01:11.20] paper moon, static on the line',
        '[01:17.70] hold the antenna, you\'ll be fine',
        '[01:24.00] we tune the dark to something kind',
        '[01:30.50] and leave the daylight far behind',
        '[01:37.00] ',
        '[01:52.40] a kettle whistles two streets down',
        '[01:58.80] the only signal left in town',
        '[02:05.20] paper moon, static on the line',
        '[02:11.70] hold the antenna, you\'ll be fine',
        '[02:18.10] count the satellites by name',
        '[02:24.60] and every one still knows the way',
        '[02:31.00] ',
        '[02:44.00] hold the antenna',
        '[02:50.50] hold the antenna',
        '[02:57.00] you\'ll be fine'
      ].join('\n'),
      wordSync: {
        '25.10': [[25.10, 'paper'], [25.70, 'moon,'], [26.60, 'static'], [27.40, 'on'], [27.70, 'the'], [28.00, 'line']],
        '31.60': [[31.60, 'hold'], [32.10, 'the'], [32.40, 'antenna,'], [33.60, 'you\'ll'], [34.10, 'be'], [34.50, 'fine']]
      }
    },
    't-lantern-season-1': {
      instrumental: false, attribution: 'LRCLIB',
      syncedLyrics: [
        '[00:09.00] the harbour keeps its lanterns low',
        '[00:15.50] for boats that never seem to go',
        '[00:22.00] i counted every rope and knot',
        '[00:28.40] to learn the shape of what we\'ve got',
        '[00:35.00] lantern season, slow and gold',
        '[00:41.20] the kind of light you\'re meant to hold',
        '[00:47.60] ',
        '[01:02.00] the gulls go quiet after six',
        '[01:08.30] the tide forgets its little tricks',
        '[01:14.70] and you and i stay on the pier',
        '[01:21.00] as if the year could stop right here',
        '[01:27.40] lantern season, slow and gold',
        '[01:33.70] the kind of light you\'re meant to hold',
        '[01:40.00] ',
        '[02:10.00] so leave a lantern in the bay',
        '[02:16.40] in case we need to find the way',
        '[02:22.80] lantern season, slow and gold',
        '[02:29.10] the only thing we didn\'t sell'
      ].join('\n')
    },
    't-marble-soda-club-1': {
      instrumental: false, attribution: 'LRCLIB',
      syncedLyrics: [
        '[00:14.00] fizz and neon, quarter past nine',
        '[00:19.80] the jukebox only plays in rhyme',
        '[00:25.60] we split a marble soda twice',
        '[00:31.40] and call the bubbles good advice',
        '[00:37.20] welcome to the marble soda club',
        '[00:43.00] the door is open, come on up',
        '[00:48.80] ',
        '[01:01.00] the escalator hums in key',
        '[01:06.80] the city winks at you and me',
        '[01:12.60] welcome to the marble soda club',
        '[01:18.40] the door is open, come on up'
      ].join('\n')
    },
    't-glasshouse-bloom-1': {
      instrumental: false, attribution: 'LRCLIB', syncedLyrics: '',
      plainLyrics: 'glass on glass and green on green\nthe quietest room you\'ve ever seen\nwe water every word we say\nand watch it grow a different way\n\nbloom, bloom, the windows sigh\nthe rain is practising goodbye\nbloom, bloom, and stay a while\nthe ferns have learned to copy your smile'
    },
    't-harbour-lights-2': { instrumental: true, attribution: 'LRCLIB', syncedLyrics: '', plainLyrics: '' }
  };

  // ---------- activity (transfers, saves at different pipeline steps, import, rescan) ----------
  /* save steps: 1 Completing transfer, 2 Matching on MusicBrainz, 3 Fetching cover art, 4 Tagging and writing to library, 5 Jellyfin rescan */
  var SAVE_STEPS = ['Completing transfer', 'Matching on MusicBrainz', 'Fetching cover art', 'Tagging and writing to library', 'Jellyfin rescan'];
  var activity = [
    { id: 'job-1', kind: 'transfer', trackId: 't-satellite-porch-1', peer: 'otter-lamp', speed: 2.4, pct: 62, state: 'transferring', started: '21:42' },
    { id: 'job-2', kind: 'transfer', trackId: 't-satellite-porch-2', peer: 'mossbank', speed: 0, pct: 0, state: 'queued', queuePos: 3, started: '21:42' },
    { id: 'job-3', kind: 'save', trackId: 't-snow-antenna-1', step: 2, stepState: 'running', pct: 100, started: '21:39' },
    { id: 'job-4', kind: 'save', trackId: 't-harbour-lights-4', step: 1, stepState: 'running', pct: 38, started: '21:40' },
    { id: 'job-5', kind: 'save', trackId: 't-low-tide-radio-3', step: 2, stepState: 'failed', error: 'No confident MusicBrainz match (3 candidates)', started: '21:12' },
    { id: 'job-6', kind: 'save', trackId: 't-cassette-garden-1', step: 5, stepState: 'done', started: '20:51' },
    { id: 'job-7', kind: 'import', name: 'road-trip.m3u8', matched: 14, search: 5, notFound: 2, state: 'done', started: '19:30' },
    { id: 'job-8', kind: 'rescan', state: 'done', detail: 'Jellyfin found 3 new files', started: '20:52' },
    { id: 'job-9', kind: 'transfer', trackId: 't-night-bus-2', peer: 'paperkite', speed: 0, pct: 38, state: 'failed', error: 'Peer stalled at 38%', started: '20:30' }
  ];

  // ---------- service statuses (normal state; banners demo switches them) ----------
  var services = {
    soulseek: { state: 'ok', label: 'soulseek', detail: 'Connected, 63 peers answered the last search' },
    jellyfin: { state: 'ok', label: 'jellyfin', detail: 'Library online, 1,204 songs' },
    lastfm: { state: 'warn', label: 'last.fm', detail: '2 scrobbles waiting, retrying in 40 s', queued: 2, user: 'moth_and_lantern' },
    cache: { state: 'ok', label: 'cache', detail: '3.1 of 5 GB used, oldest audio evicted first', usedGB: 3.1, capGB: 5 }
  };

  // ---------- Classic mode skins (fictional; palettes so placeholders can be drawn) ----------
  var skins = [
    { id: 'a1f3c9e2', name: 'Plum Cassette', file: 'plum_cassette.wsz', palette: ['#5f4fc4', '#e4defe', '#2a2f40'], favourite: true, downloaded: true, nsfw: false },
    { id: 'b72e01d4', name: 'Bubblegum Deck', file: 'bubblegum_deck.wsz', palette: ['#dd9fc9', '#fce8f3', '#664f59'], favourite: true, downloaded: true, nsfw: false },
    { id: 'c90a4f11', name: 'Midnight Ferry', file: 'midnight_ferry.wsz', palette: ['#414759', '#b6c7f5', '#fff9fe'], favourite: false, downloaded: true, nsfw: false },
    { id: 'd45b7e20', name: 'Mint Radio', file: 'mint_radio.wsz', palette: ['#2f6b4f', '#c9e8d8', '#2a0809'], favourite: false, downloaded: false, nsfw: false },
    { id: 'e18c2b93', name: 'Chrome Lilac', file: 'chrome_lilac.wsz', palette: ['#c3c2e2', '#f7f4ff', '#3d3491'], favourite: false, downloaded: false, nsfw: false },
    { id: 'f2d06a57', name: 'Sunset Arcade', file: 'sunset_arcade.wsz', palette: ['#b6969b', '#f6cee4', '#5e4b53'], favourite: false, downloaded: false, nsfw: false },
    { id: '0a7b3c66', name: 'Pixel Tea Party', file: 'pixel_tea_party.wsz', palette: ['#f2e3d1', '#dd9fc9', '#445796'], favourite: false, downloaded: false, nsfw: false },
    { id: '1be94d08', name: 'Graphite 98', file: 'graphite_98.wsz', palette: ['#6f7182', '#e0ccb4', '#2a0809'], favourite: false, downloaded: false, nsfw: false }
  ];

  // ---------- settings values ----------
  var settings = {
    playback: { gapless: true, crossfade: 0, normalisation: 'replaygain-track', volumeStep: 5, radioWhenQueueEnds: true },
    sources: { preferred: 'lossless', minBitrate: 256, maxPeerQueue: 20, stallTimeout: 15, autoPick: true },
    cache: { capGB: 5, usedGB: 3.1, breakdown: { playing: 0.4, recent: 1.9, evictable: 0.8 } },
    library: { jellyfinUrl: 'https://jellyfin.home.arpa', musicFolder: '/music', naming: '{artist}/{year} - {album}/{track:02} - {title}.{ext}',
      duplicatePolicy: 'ask' },
    sharing: { mode: 'library', uploadSlots: 3, uploadLimitKBs: 500, sharedFiles: 1204 },
    lastfm: { connected: true, user: 'moth_and_lantern', scrobbling: true, pending: 2, useHistory: true },
    discovery: { adventurousness: 0.4, sources: { listenbrainz: 'ok', musicbrainz: 'ok', lastfm: 'ok', discogs: 'off' } },
    appearance: { theme: 'system', accent: 'periwinkle', motion: 'system', textSize: 100, defaultSkin: 'a1f3c9e2', showNsfwSkins: false },
    app: { version: '0.1.0', server: 'https://music.home.arpa', health: 'ok', tokenRemembered: true }
  };

  var recentSearches = ['lantern season', 'city-pop revival', 'neon wren', 'tiny orbit live'];

  /* Initial queue: a Velvet Orchard radio session. group: now | next (user "Play next") | context | radio.
     resolve: ready (prefetched) | resolved | resolving | none */
  var queue = {
    radio: { name: 'Velvet Orchard', seed: { kind: 'artist', id: 'ar-velvet-orchard' }, keepsAdding: true, steer: 0.4 },
    items: [
      { trackId: 't-satellite-porch-1', group: 'context', resolve: 'resolved' },
      { trackId: 't-lantern-season-1', group: 'next', resolve: 'ready' },
      { trackId: 't-glasshouse-bloom-1', group: 'next', resolve: 'resolved' },
      { trackId: 't-satellite-porch-2', group: 'context', resolve: 'resolved' },
      { trackId: 't-satellite-porch-3', group: 'context', resolve: 'resolving' },
      { trackId: 't-low-tide-radio-1', group: 'radio', resolve: 'resolving' },
      { trackId: 't-cassette-garden-1', group: 'radio', resolve: 'resolved' },
      { trackId: 't-quiet-weather-2', group: 'radio', resolve: 'none' }
    ]
  };

  return {
    formats: FORMATS, saveSteps: SAVE_STEPS,
    tags: tags, artists: artists, albums: albums, tracks: tracks,
    feed: feed, playlists: playlists, history: history, stats: stats, lyrics: lyrics,
    activity: activity, services: services, skins: skins, settings: settings,
    recentSearches: recentSearches, queue: queue, peers: PEERS,
    // lookups
    track: function (id) { return trackById[id]; },
    album: function (id) { return albumById[id]; },
    artist: function (id) { return artistById[id]; },
    tag: function (id) { return tagById[id]; },
    playlist: function (id) { return playlists.filter(function (p) { return p.id === id; })[0]; },
    albumsBy: function (artistId) { return albums.filter(function (a) { return a.artistId === artistId; }); },
    tracksBy: function (artistId) { return tracks.filter(function (t) { return t.artistId === artistId; }); }
  };
})();
