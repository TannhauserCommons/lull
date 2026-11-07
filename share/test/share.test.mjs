// lull's share links (docs/adr/0034), run as they deploy with the outside world faked (harness.mjs):
// the card a chat app reads, a cover that never breaks, one lookup for a burst, and politeness to every source.
// Run: npm test (in share/)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { HOST, internet, json, picture, start } from './harness.mjs';
import { SOLID } from './pictures.mjs';

const MB = '48140466-cff6-3222-bd55-63c27e43190d';
const SONG = '/1/song/Portishead/Glory%20Box?al=Dummy&len=305&it=1440823733&ic=1440823517&mb=' + MB;
const COVER = '/1/song/Portishead/Glory%20Box/cover?al=Dummy&len=305&it=1440823733&ic=1440823517&mb=' + MB;
const CARD = '/1/song/Portishead/Glory%20Box/card?al=Dummy&len=305&it=1440823733&ic=1440823517&mb=' + MB;
const STORE_ART = 'https://is1-ssl.mzstatic.com/image/thumb/Music/dummy/100x100bb.jpg';

/** iTunes knows the album; its CDN serves the cover at the size asked. */
function storeKnowsIt(net) {
  net.on(/^https:\/\/itunes\.apple\.com\/lookup\?id=1440823517&country=US$/, () => json({ results: [{ artworkUrl100: STORE_ART }] }));
  net.on(/^https:\/\/is1-ssl\.mzstatic\.com\/.*\/600x600bb\.jpg$/, () => picture('itunes'));
}

async function withWorker(net, body) {
  const worker = await start(net);
  try { await body(worker); } finally { await worker.dispose(); }
}

function metaOf(html, key) {
  const m = new RegExp(`<meta (?:property|name)="${key}" content="([^"]*)">`).exec(html);
  return m && m[1].replace(/&#(\d+);/g, (_, c) => String.fromCharCode(+c));
}

test('a song link is a page whose tags give a chat app its card, with no outside call', async () => {
  const net = internet();
  await withWorker(net, async (w) => {
    const res = await w.get(SONG);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('x-robots-tag'), 'noindex');
    const html = await res.text();
    assert.equal(metaOf(html, 'og:title'), 'Glory Box');
    assert.equal(metaOf(html, 'og:description'), 'Portishead · Dummy');
    assert.equal(metaOf(html, 'og:site_name'), 'lull');
    assert.equal(metaOf(html, 'og:url'), HOST + SONG, 'the canonical address, ids in their order');
    assert.equal(metaOf(html, 'og:image'), HOST + COVER);
    assert.equal(metaOf(html, 'twitter:card'), 'summary_large_image');
    assert.equal(metaOf(html, 'music:duration'), '305');
    const item = JSON.parse(/<script type="application\/json" id="lull-item">(.*?)<\/script>/.exec(html)[1]);
    assert.deepEqual(item, {
      kind: 'song', artist: 'Portishead', title: 'Glory Box', album: 'Dummy',
      page: 'https://song.link/i/1440823733', cover: COVER,
    });
    assert.match(html, /<script src="\/open\.js" defer>/, 'the open page\'s files are named from the root');
    assert.match(html, /<link rel="stylesheet" href="\/look\.css">/, 'lull\'s own look, for everyone');
    assert.doesNotMatch(html, /(href|src)="(open\.|app-icon)/);
    assert.equal(net.asked.length, 0, 'the page needs nobody else');
  });
});

test('names are exact and escaped; a malformed id is dropped, not trusted', async () => {
  const net = internet();
  await withWorker(net, async (w) => {
    const name = encodeURIComponent('AC/DC "</script><b>&');
    const html = await (await w.get(`/1/artist/${name}?mb=not-an-mbid&dz=115&x=1`)).text();
    assert.equal(metaOf(html, 'og:title'), 'AC/DC "</script><b>&');
    assert.doesNotMatch(html, /<b>/);
    assert.equal(metaOf(html, 'og:url'), `${HOST}/1/artist/${name.replace(/%20/g, '%20')}?dz=115`);
    const item = JSON.parse(/id="lull-item">(.*?)<\/script>/.exec(html)[1]);
    assert.equal(item.artist, 'AC/DC "</script><b>&');
    assert.equal(item.page, 'https://www.last.fm/music/AC%2FDC%20%22%3C%2Fscript%3E%3Cb%3E%26');
  });
});

test('an address that names nothing is a 404 page with lull\'s card', async () => {
  await withWorker(internet(), async (w) => {
    for (const path of ['/', '/1/song/OnlyAnArtist', '/2/song/a/b', '/1/song/a/b/c', '/1/podcast/a/b']) {
      const res = await w.get(path);
      assert.equal(res.status, 404, path);
      const html = await res.text();
      assert.equal(metaOf(html, 'og:image'), HOST + '/lull-card.png', path);
      assert.doesNotMatch(html, /lull-item/, path);
    }
  });
});

test('the cover is the first source\'s picture, ids first, kept a week by whoever shows it', async () => {
  const net = internet();
  storeKnowsIt(net);
  await withWorker(net, async (w) => {
    const res = await w.get(COVER);
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('content-type'), 'image/jpeg');
    assert.equal(res.headers.get('cache-control'), 'public, max-age=604800');
    assert.equal(await res.text(), 'picture:itunes');
  });
});

test('a source that has none or can\'t answer is passed over for the next', async () => {
  const net = internet();
  net.on(/itunes\.apple\.com\/lookup/, () => json({ resultCount: 0, results: [] }));
  net.on(/api\.deezer\.com\/album\/302127$/, () => new Response('down', { status: 503 }));
  net.on(new RegExp(`coverartarchive\\.org/release-group/${MB}/front-500$`), () =>
    new Response(null, { status: 307, headers: { location: 'https://ia800.us.archive.org/dummy-500.jpg' } }));
  net.on(/ia800\.us\.archive\.org\/dummy-500\.jpg$/, () => picture('archive'));
  await withWorker(net, async (w) => {
    const res = await w.get('/1/album/Portishead/Dummy/cover?it=1440823517&dz=302127&mb=' + MB);
    assert.equal(await res.text(), 'picture:archive');
    assert.equal(net.count(/api\.deezer\.com/), 1);
  });
});

test('with no picture anywhere, the cover is lull\'s card, never a broken image', async () => {
  const net = internet();
  net.on(/ws\.audioscrobbler\.com/, () => json({ album: { image: [{ '#text': '' }] } }));
  net.on(/itunes\.apple\.com\/search/, () => json({ results: [{ artistName: 'Someone Else', collectionName: 'Untitled', artworkUrl100: STORE_ART }] }));
  await withWorker(net, async (w) => {
    const res = await w.get('/1/album/Nobody/Untitled/cover');
    assert.equal(res.status, 200);
    assert.equal(res.headers.get('content-type'), 'image/png');
    assert.equal(res.headers.get('cache-control'), 'public, max-age=3600');
    const card = readFileSync(new URL('../../web/assets/fallback-art-512.png', import.meta.url));
    assert.deepEqual(Buffer.from(await res.arrayBuffer()), card);
    assert.equal(net.count(/mzstatic/), 0, 'another artist\'s album is never taken by its name');
  });
});

test('an album the US store doesn\'t list is looked up in the store of whoever opened the link', async () => {
  const net = internet();
  net.on(/itunes\.apple\.com\/lookup\?id=1440823517&country=US$/, () => json({ resultCount: 0, results: [] }));
  net.on(/itunes\.apple\.com\/lookup\?id=1440823517&country=IN$/, () => json({ results: [{ artworkUrl100: STORE_ART }] }));
  net.on(/mzstatic\.com/, () => picture('itunes'));
  await withWorker(net, async (w) => {
    assert.equal(await (await w.get('/1/album/Portishead/Dummy/cover?it=1440823517', { country: 'IN' })).text(), 'picture:itunes');
    assert.ok(net.asked.every((u) => !u.includes('lull.share-host')), 'the link never says where its sender is');
  });
});

test('a burst of clicks on a new link makes one lookup', async () => {
  const net = internet();
  let release;
  const held = new Promise((r) => { release = r; });
  net.on(/itunes\.apple\.com\/lookup/, async () => { await held; return json({ results: [{ artworkUrl100: STORE_ART }] }); });
  net.on(/mzstatic\.com/, () => picture('itunes'));
  await withWorker(net, async (w) => {
    const burst = Array.from({ length: 20 }, (_, i) => w.get(COVER, { ip: `198.51.100.${i}` }));
    await new Promise((r) => setTimeout(r, 300));
    release();
    const answers = await Promise.all(burst.map(async (r) => (await r).text()));
    assert.deepEqual(new Set(answers), new Set(['picture:itunes']));
    assert.equal(net.count(/itunes\.apple\.com\/lookup/), 1, 'one call to iTunes for twenty clicks');
    // Another song on the same album shares the lookup.
    await (await w.get('/1/song/Portishead/Roads/cover?al=Dummy&ic=1440823517')).text();
    assert.equal(net.count(/itunes\.apple\.com\/lookup/), 1);
  });
});

test('a picture that stops answering is looked for again, from the next source that has one', async () => {
  const net = internet();
  storeKnowsIt(net);
  net.on(/api\.deezer\.com\/album\/302127$/, () => json({ cover_xl: 'https://cdn-images.dzcdn.net/images/cover/abc/1000x1000-000000-80-0-0.jpg' }));
  net.on(/cdn-images\.dzcdn\.net\/images\/cover\/abc\/600x600-/, () => picture('deezer'));
  await withWorker(net, async (w) => {
    assert.equal(await (await w.get('/1/album/Portishead/Dummy/cover?it=1440823517&dz=302127')).text(), 'picture:itunes');
    // The store drops the album: its cover and its listing are gone. Another song of the album (an address no cache
    // holds) finds the address kept no longer answering.
    net.on(/mzstatic\.com/, () => new Response('gone', { status: 404 }));
    net.on(/itunes\.apple\.com\/lookup/, () => json({ results: [] }));
    const again = await w.get('/1/song/Portishead/Roads/cover?al=Dummy&ic=1440823517&dz=302127');
    assert.equal(again.status, 200);
    assert.equal(await again.text(), 'picture:deezer');
  });
});

test('a source that says slow down is left alone for a while, and a lookup never waits on it', async () => {
  const net = internet();
  net.on(/itunes\.apple\.com/, () => new Response('slow down', { status: 429 }));
  net.on(/api\.deezer\.com\/album\/1$/, () => json({ cover_xl: 'https://cdn-images.dzcdn.net/images/cover/a1/1000x1000-x.jpg' }));
  net.on(/api\.deezer\.com\/album\/2$/, () => json({ cover_xl: 'https://cdn-images.dzcdn.net/images/cover/a2/1000x1000-x.jpg' }));
  net.on(/cdn-images\.dzcdn\.net/, () => picture('deezer'));
  await withWorker(net, async (w) => {
    assert.equal(await (await w.get('/1/album/A/One/cover?it=11&dz=1')).text(), 'picture:deezer');
    assert.equal(await (await w.get('/1/album/B/Two/cover?it=22&dz=2')).text(), 'picture:deezer');
    assert.equal(net.count(/itunes\.apple\.com/), 1, 'paused after its 429');
  });
});

test('past 60 requests a minute from one client: a page says to wait, a cover is still an image', async () => {
  const net = internet();
  await withWorker(net, async (w) => {
    // Miniflare counts in windows aligned to the clock's minutes, so 61 requests across a minute's turn would start a
    // fresh count and the 61st would pass (seen in queue run 42168991192). They take well under a second: start them
    // with a minute's turn at least 10 s away, and check they ended in the window they began in.
    const window = () => Math.floor(Date.now() / 60_000);
    const toTurn = 60_000 - (Date.now() % 60_000);
    if (toTurn < 10_000) await new Promise((r) => setTimeout(r, toTurn + 100));
    const began = window();
    for (let i = 0; i < 60; i++) await (await w.get(SONG, { ip: '192.0.2.77' })).text();
    const page = await w.get(SONG, { ip: '192.0.2.77' });
    assert.equal(window(), began, 'the 61 requests took over 10 s, so they crossed into a new minute: the limiter\'s count started over');
    assert.equal(page.status, 429);
    assert.equal(page.headers.get('retry-after'), '60');
    assert.match(await page.text(), /Wait a minute/);
    const cover = await w.get(COVER, { ip: '192.0.2.77' });
    assert.equal(cover.status, 200);
    assert.equal(cover.headers.get('content-type'), 'image/png');
    assert.equal(net.asked.length, 0, 'and nothing outside was asked for it');
    assert.equal((await w.get(SONG, { ip: '192.0.2.78' })).status, 200, 'another client is unaffected');
  });
});

test('Discord gets lull\'s card: the link\'s words, its cover beside them, the cover\'s colour along its edge', async () => {
  const net = internet();
  storeKnowsIt(net);
  // The cover's small copy, which its colour is read from: one colour, (52, 110, 190).
  net.on(/mzstatic\.com\/.*\/128x128bb\.jpg$/, () => new Response(Buffer.from(SOLID, 'base64'), { headers: { 'content-type': 'image/jpeg' } }));
  await withWorker(net, async (w) => {
    const html = await (await w.get(SONG)).text();
    const link = /<link rel="discord:component-embed" type="application\/json" href="([^"]+)">/.exec(html);
    assert.equal(link?.[1].replace(/&#38;/g, '&'), HOST + CARD, 'the page links its card on its own host');
    const res = await w.get(CARD);
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type'), /^application\/json/);
    const { component } = await res.json();
    assert.equal(component.type, 17);
    const edge = [16, 8, 0].map((shift) => (component.accent_color >> shift) & 255);
    assert.ok(edge.every((v, i) => Math.abs(v - [52, 110, 190][i]) <= 4), `the edge ${edge} is the cover's colour`);
    const [section, row] = component.components;
    assert.equal(section.components[0].content, '-# Song shared from [lull](https://github.com/TannhauserCommons/lull)\n### Glory Box\nPortishead\n-# Dummy · 5:05');
    assert.deepEqual(section.accessory, { type: 11, media: { url: HOST + COVER }, description: 'Cover of Dummy by Portishead' });
    assert.deepEqual(row.components, [
      { type: 2, style: 5, label: 'Open in lull', url: HOST + SONG },
      { type: 2, style: 5, label: 'Every music app', url: 'https://song.link/i/1440823733' },
    ]);
    // The card started the cover's lookup, so the cover Discord asks for next costs no call to iTunes.
    const asked = net.count(/itunes\.apple\.com/);
    assert.equal(await (await w.get(COVER)).text(), 'picture:itunes');
    assert.equal(net.count(/itunes\.apple\.com/), asked);
  });
});

test('a card with no cover\'s colour is lull\'s, names are printed as they are, and a card too long for Discord isn\'t linked', async () => {
  await withWorker(internet(), async (w) => {
    const album = (await (await w.get('/1/album/Nobody/Untitled/card')).json()).component;
    assert.equal(album.accent_color, 0x3d3491, 'no source has a cover: lull\'s colour');
    assert.equal(album.components[0].components[0].content, '-# Album shared from [lull](https://github.com/TannhauserCommons/lull)\n### Untitled\nNobody');
    assert.deepEqual(album.components[1].components[1], { type: 2, style: 5, label: 'Last.fm', url: 'https://www.last.fm/music/Nobody/Untitled' });

    const name = '#1 *Hits* <@everyone> [x](y)';
    const artist = (await (await w.get(`/1/artist/${encodeURIComponent(name)}/card`)).json()).component;
    assert.equal(artist.components[0].components[0].content,
      '-# Artist shared from [lull](https://github.com/TannhauserCommons/lull)\n### \\#1 \\*Hits\\* \\<\\@everyone\\> \\[x\\]\\(y\\)', 'no markup, no mention');

    const long = encodeURIComponent('夜'.repeat(200));
    const song = `/1/song/${long}/${long}`;
    assert.doesNotMatch(await (await w.get(song)).text(), /discord:component-embed/);
    assert.equal((await w.get(`/1/song/${long}/${long}/card`)).status, 404);
  });
});

test('Android reads the App Links statement at the host\'s root', async () => {
  await withWorker(internet(), async (w) => {
    const res = await w.get('/.well-known/assetlinks.json');
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type'), /^application\/json/);
    const statement = readFileSync(new URL('../../packaging/android/assetlinks.json', import.meta.url), 'utf8');
    assert.deepEqual(await res.json(), JSON.parse(statement));
  });
});
