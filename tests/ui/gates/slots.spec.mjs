// A theme's markup in every region and screen, and lull's parts in every slot (ADR-0101, docs/themes.md section 6), in a
// real page: a listener's theme puts an LCD after the player bar (the copy's quality by the live model, and a shuffle
// key of its own), an LED before the header (the song's title, and lull's Like), a box before Home and the queue's next
// songs after the side panel. Each is drawn where the theme said, stays through what lull draws again (another song,
// the panel, another screen), keeps current, and the key works; a built-in design gets none of it, nor the script that
// draws it (designs/parts.js), which loads only for a theme whose slots use it. On a fixture hub of the gate's own,
// serving the themes as the hub would.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { designs, ids, openScreen, ownHub, settle } from './lib.mjs';

const MANIFEST = { id: 'slotted', name: 'Slotted', api: 1, extends: 'lull', shell: 'rail', swatches: ['#141516', '#d6d3c9', '#ffa53d', '#ffb24a'], modes: ['day', 'night'] };
const CSS = `#pbar > [data-slot="after.playerbar"] { grid-column: 1 / -1; display: flex; gap: 8px; align-items: center; }
.ts-shuf[aria-pressed="true"] { text-decoration: underline; }`;
const SLOTS = {
  'after.playerbar': '<div class="ts-lcd"><b class="ts-q" data-bind="song.quality"></b> <button class="ts-shuf" data-act="shuffle" data-bind="shuffle">SHUF</button></div>',
  'before.header': '<div class="ts-led"><b class="ts-title" data-bind="song.title"></b><lull-part name="like-button"></lull-part></div>',
  'before.home': '<div class="ts-home">a window of the theme\'s</div>',
  'before.now': '<div class="ts-now">beside the hero</div>',
  'after.ctx': '<section class="ts-next" aria-label="Next songs"><lull-part name="up-next" count="3"></lull-part></section>',
};

/* two themes without a region or screen slot: one whose hero takes only the parts Now Playing hands it, and one whose
   Home lead takes a part only the catalogue draws there (Home hands no state) and one it never hands (next) */
const HANDED = { 'nowHero.full': '<div class="th-hero"><lull-part name="play-button" size="56"></lull-part><lull-part name="facts"></lull-part><lull-part name="state"></lull-part></div>' };
const PARTED = { homeLead: '<div class="tp-lead"><b class="tp-state"><lull-part name="state"></lull-part></b><lull-part name="next"></lull-part></div>' };
/* a theme with an upright volume fader after the player bar, and a hero whose "playing from" has words of its own */
const UPRIGHT = {
  'after.playerbar': '<div class="tu-fader"><lull-part name="volume" orientation="vertical"></lull-part></div>',
  'nowHero.full': '<div class="tu-hero"><p class="now-head__from tu-from"><lull-part name="playing-from" label="out of"></lull-part></p><lull-part name="play-button" size="56"></lull-part></div>',
};
/* a theme whose every own element says, in data-own, the words it holds (ADR-0101 amended, D): beside lull's parts,
   in keys lull keeps (data-bind), in a hero, Home's lead, a region and a screen */
const OWNED = {
  'nowHero.full': '<div class="ow-hero"><p class="now-head__from" data-own="FROM"><lull-part name="playing-from" label="out of"></lull-part>FROM</p>' +
    '<lull-part name="play-button" size="56"></lull-part><button data-act="like" data-bind="like" data-own="LIKE">LIKE</button></div>',
  'homeLead.playing': '<div class="ow-lead" data-own="LEAD">LEAD<lull-part name="state"></lull-part><lull-part name="play-button" size="48"></lull-part></div>',
  'after.playerbar': '<div class="ow-bar" data-own="BAR">BAR<button data-act="play" data-bind="play" data-own="PLAY"><lull-part name="icon" icon="play"></lull-part>PLAY</button>' +
    '<button data-act="shuffle" data-bind="shuffle" data-own="SHUF">SHUF</button><button data-act="repeat" data-bind="repeat" data-own="REP">REP</button>' +
    '<button data-act="save" data-bind="save" data-own="SAVE">SAVE</button><button data-act="mute" data-bind="mute" data-own="MUTE">MUTE</button>' +
    '<span data-own="NOW">NOW <b data-bind="song.title"></b></span><lull-part name="like-button"></lull-part><lull-part name="up-next" count="2"></lull-part></div>',
  'before.home': '<p class="ow-home" data-own="HOME">HOME</p>',
  'after.ctx': '<p class="ow-ctx" data-own="CTX">CTX<lull-part name="playing-from"></lull-part></p>',
};
/* a theme that places lull's sections and lists anywhere, with rows of its own, and draws at the edges of the whole
   shell and of a Home shelf (ADR-0101 amended, B and C) */
const PLACED = {
  'before.app': '<div class="pl-top">a strip above everything</div>',
  'after.app': '<div class="pl-foot">and one below</div>',
  'after.jump': '<p class="pl-jump-foot">after the shelf</p>',
  'after.ctx': '<section class="pl-side" aria-label="Lists"><h3>Next up · yours</h3>' +
    '<lull-part name="list" of="queue" where="group=yours" label="Yours"><lull-row><a class="pl-row" href="#/album/{{item.albumId}}" data-id="{{item.id}}">{{item.title | upper}}</a> <i class="pl-len">{{item.duration | time}}</i></lull-row></lull-part>' +
    '<h3>Then</h3><lull-part name="list" of="queue" where="group=place|radio" count="2"></lull-part>' +
    '<lull-part name="home.jump"></lull-part><lull-part name="list" of="home.jump" count="3"><lull-row><b class="pl-jump" data-kind="{{item.kind}}">{{item.title}}{{item.name}}</b></lull-row></lull-part></section>',
};
/* the contract theme (ADR-0101, amended again): built like the hardest mockups so far, with nothing lull adds for it.
   Its own rows with lull's cover and source mark bound to each song, a list from any read, tabs it keeps itself, a
   piece of another screen, a strip above the shell and facts by path. A wall a later mockup finds is added here */
const HARD = {
  'before.app': '<div class="hd-top" data-live><b class="hd-day">{{ clock.now | date:weekday-short | upper }}</b> <b class="hd-src" data-bind="copy.where | map:library=LIB,soulseek=SLSK,device=DEV,youtube=YT"></b></div>',
  'after.ctx': '<section class="hd-side" aria-label="Side"><div data-states="tab:queue"><div role="tablist" aria-label="Side">' +
    '<button role="tab" data-act="state" data-set="tab=queue">QUEUE</button><button role="tab" data-act="state" data-set="tab=albums">ALBUMS</button></div>' +
    '<div class="hd-panel hd-q" data-show="tab:queue"><lull-part name="list" of="queue" count="5" label="Up next"><lull-row>' +
      '<lull-part name="cover" size="40"></lull-part><b class="hd-t">{{item.title}}</b> <i class="hd-s">{{item.source | map:library=● LIB,soulseek=◆ SLSK,device=▼ DEV}}</i> <i class="hd-l">{{item.duration | time}}</i>' +
      '<lull-part name="like-button"></lull-part><lull-part name="play-button"></lull-part></lull-row></lull-part></div>' +
    '<div class="hd-panel hd-a" data-show="tab:albums"><lull-part name="list" read="/api/library/albums" key="items" count="3" label="Albums"><lull-row>' +
      '<lull-part name="cover" size="40"></lull-part><b class="hd-al">{{item.title}}</b></lull-row></lull-part></div></div>' +
    '<lull-part name="screen" path="/home" pick=\'[data-region="jump"]\'></lull-part></section>',
  // walls the Fuji themes found after the second amendment, each now part of the contract
  'before.content': '<section class="hd-more" aria-label="More">' +
    '<lull-part name="list" of="activity" count="5" label="Activity" empty="NO ACTIVITY"><lull-row><b class="hd-job">{{item.entry.kind}} {{item.state}}</b></lull-row></lull-part>' +
    '<lull-part name="list" of="playlists" count="2" label="Playlists"><lull-row><lull-part name="cover" size="40"></lull-part><b class="hd-pl">{{item.name}}</b></lull-row></lull-part>' +
    '<lull-part name="list" of="home.jump" count="4" label="Jump back in"><lull-row><lull-part name="play-button"></lull-part><b class="hd-j">{{item.title}}{{item.name}}</b></lull-row></lull-part>' +
    '<p class="hd-line"><span>Kasagumo</span>, <span>Night drive</span> · <span>1984</span></p></section>',
};
const UPRIGHT_CSS = `#pbar > [data-slot="after.playerbar"] { position: fixed; right: 40px; bottom: 120px; }
.tu-fader .seek[aria-orientation="vertical"] { width: 24px; min-width: 0; height: 160px; }`;

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-slots-'));
  for (const [id, slots, css] of [['slotted', SLOTS, CSS], ['handed', HANDED, ''], ['parted', PARTED, ''], ['upright', UPRIGHT, UPRIGHT_CSS], ['owned', OWNED, ''], ['placed', PLACED, ''], ['hard', HARD, '']]) {
    mkdirSync(join(dir, id, 'slots'), { recursive: true });
    writeFileSync(join(dir, id, 'theme.json'), JSON.stringify({ ...MANIFEST, id, name: id }));
    writeFileSync(join(dir, id, 'theme.css'), css);
    for (const [name, html] of Object.entries(slots)) writeFileSync(join(dir, id, 'slots', `${name}.html`), html);
  }
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

async function context(browser) {
  return browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 }, timezoneId: 'UTC', locale: 'en-GB' });
}
/* where each box is: its region or screen container, and whether it is that container's first or last child */
const placed = (page) => page.evaluate(() => [...document.querySelectorAll('.region-slot')].map((box) => {
  const host = box.parentElement;
  return { slot: box.getAttribute('data-slot'), in: host.getAttribute('data-region') || host.className,
    edge: host.firstElementChild === box ? 'first' : host.lastElementChild === box ? 'last' : 'middle' };
}).sort((a, b) => a.slot.localeCompare(b.slot)));
/* what the theme's markup shows now */
const shown = (page) => page.evaluate(() => ({
  title: document.querySelector('.ts-title').textContent, quality: document.querySelector('.ts-q').textContent,
  barQuality: document.querySelector('#pbar .pbar__badges .q-chip')?.textContent.trim() ?? '',
  next: [...document.querySelectorAll('.ts-next .now-q__row .trow__title')].map((b) => b.textContent),
  likes: document.querySelector('.ts-led [data-act="like"]').getAttribute('data-id'),
}));

test('a theme\'s slots are drawn at the edges it names, and lull\'s parts in them are its own', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  const served = await (await page.request.get('/api/themes/slotted/theme.json')).json();
  expect(served.slots).toEqual(['after.ctx', 'after.playerbar', 'before.header', 'before.home', 'before.now']);
  await openScreen(page, '/home', { design: 'slotted' });
  expect(await placed(page)).toEqual([
    { slot: 'after.ctx', in: 'ctx', edge: 'last' },
    { slot: 'after.playerbar', in: 'playerbar', edge: 'last' },
    { slot: 'before.header', in: 'header', edge: 'first' },
    { slot: 'before.home', in: 'disc-home', edge: 'first' },
  ]);
  // the catalogue's parts stay in their <lull-part> box; nothing plays yet, so Like names no song and is off
  expect(await page.evaluate(() => [...document.querySelectorAll('lull-part')].map((p) => [p.getAttribute('name'), getComputedStyle(p).display])))
    .toEqual([['like-button', 'contents'], ['up-next', 'contents']]);
  await expect(page.locator('.ts-led [data-act="like"]')).toHaveAttribute('aria-pressed', 'false');
  await expect(page.locator('.ts-led [data-act="like"]')).toHaveAttribute('data-id', '');
  await expect(page.locator('.ts-next')).toContainText('Nothing up next');
  // Now Playing's screen slot is an item of its grid, beside the hero
  await page.evaluate(() => { location.hash = '#/now'; });
  await expect(page.locator('.now__grid > .region-slot[data-slot="before.now"]')).toHaveCount(1);
  expect(await page.evaluate(() => document.querySelector('.now__grid').firstElementChild.getAttribute('data-slot'))).toBe('before.now');
  await ctx.close();
});

test('the theme\'s markup follows the player, stays through what lull draws again, and its key works', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, `/album/${ids.album}`, { design: 'slotted' });
  const boxes = await page.evaluateHandle(() => [...document.querySelectorAll('.region-slot')]);
  const songs = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
    return rows.slice(0, 5).map((id) => ({ id, title: App.data.track(id).title }));
  });
  await expect(page.locator('.ts-title')).toHaveText(songs[0].title);
  await expect(page.locator('.ts-next .now-q__row')).toHaveCount(3);
  let now = await shown(page);
  expect(now.quality, 'the copy\'s quality, as the bar\'s chip says it').not.toBe('');
  expect(now.quality).toBe(now.barQuality);
  expect(now.next).toEqual(songs.slice(1, 4).map((s) => s.title));
  expect(now.likes).toBe(songs[0].id);

  // another song: the words, Like and the next songs follow; the boxes are the same elements, never drawn again
  await page.evaluate(() => App.player.next());
  await expect(page.locator('.ts-title')).toHaveText(songs[1].title);
  await expect(page.locator('.ts-next .now-q__row .trow__title').first()).toHaveText(songs[2].title);
  now = await shown(page);
  expect(now.next).toEqual(songs.slice(2, 5).map((s) => s.title));
  expect(now.likes).toBe(songs[1].id);
  // the panel closes and opens, the route changes: lull draws the bar, the panel and the view again
  await page.evaluate(() => App.togglePanel('queue'));
  await page.evaluate(() => App.togglePanel('queue'));
  await page.evaluate(() => { location.hash = '#/home'; });
  await settle(page);
  expect(await boxes.evaluate((list) => list.map((b) => b.isConnected))).toEqual([true, true, true]);
  await expect(page.locator('.disc-home > .region-slot[data-slot="before.home"]')).toHaveCount(1);

  // the theme's own shuffle key: it shuffles, and lull shows it pressed, both ways
  const shuf = page.locator('.ts-shuf');
  await expect(shuf).toHaveAttribute('aria-pressed', 'false');
  await shuf.click();
  await expect(shuf).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate(() => App.player.state.shuffle)).toBe(true);
  await shuf.click();
  await expect(shuf).toHaveAttribute('aria-pressed', 'false');
  expect(await page.evaluate(() => App.player.state.shuffle)).toBe(false);

  // lull's Like in the theme's LED likes the playing song, and shows it
  const like = page.locator('.ts-led [data-act="like"]');
  await like.click();
  await expect(like).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate((id) => App.isLiked(id), songs[1].id)).toBe(true);
  await ctx.close();
});

/* the requests for designs/parts.js a page makes */
function partsAsked(page) {
  const asked = [];
  page.on('request', (r) => { if (/\/parts\.js(\?|$)/.test(r.url())) asked.push(r.url()); });
  return asked;
}

for (const design of designs) {
  test(`${design}, which has no region or screen slot, gets no box and no part's box, and loads none of it`, async ({ browser }) => {
    const ctx = await context(browser), page = await ctx.newPage();
    const asked = partsAsked(page);
    for (const route of ['/home', '/now', `/album/${ids.album}`]) {
      await openScreen(page, route, { design });
      expect(await page.evaluate(() => [document.querySelectorAll('.region-slot').length, document.querySelectorAll('lull-part').length]), route)
        .toEqual([0, 0]);
    }
    expect(asked, 'parts.js fetched').toEqual([]);
    expect(await page.evaluate(() => [typeof App.parts, typeof App.edges])).toEqual(['undefined', 'undefined']);
    await ctx.close();
  });
}

test('a theme whose slots take only the parts lull hands them loads no catalogue; one that takes another part does', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  let asked = partsAsked(page);
  await openScreen(page, '/now', { design: 'handed' });
  await expect(page.locator('.th-hero .btn-gloss[data-act="play"]')).toHaveCount(1);
  expect(asked, 'parts.js fetched for parts the hero hands').toEqual([]);
  expect(await page.evaluate(() => typeof App.parts)).toBe('undefined');
  await ctx.close();

  const ctx2 = await context(browser), page2 = await ctx2.newPage();
  asked = partsAsked(page2);
  await openScreen(page2, '/home', { design: 'parted' });
  expect(asked.length, 'parts.js fetched once').toBe(1);
  await expect(page2.locator('.tp-lead lull-part[name="next"] [data-act="next"]')).toHaveCount(1);
  await expect(page2.locator('.tp-state')).toHaveText('nothing playing');
  await ctx2.close();
});

test('back on a built-in, the theme\'s boxes go', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, '/home', { design: 'slotted' });
  await expect(page.locator('.region-slot')).toHaveCount(4);
  await page.evaluate(() => new Promise((ok) => { App.on('design', ok); App.setDesign('lull', true); }));
  await settle(page);
  expect(await page.evaluate(() => [document.querySelectorAll('.region-slot').length, document.querySelectorAll('lull-part').length])).toEqual([0, 0]);
  await ctx.close();
});

test('a vertical slider reads the pointer bottom to top, and the hero\'s "playing from" keeps its own words', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, `/album/${ids.album}`, { design: 'upright' });
  const bar = page.locator('.tu-fader .seek[data-seek="volume"]');
  await expect(bar).toHaveAttribute('aria-orientation', 'vertical');
  const box = await bar.boundingBox();
  // a press near the top is loud, near the bottom quiet: the height in the bar, whatever the pointer's left
  await page.mouse.click(box.x + 4, box.y + box.height * 0.1);
  await expect.poll(() => page.evaluate(() => App.player.state.volume)).toBeGreaterThan(0.85);
  await page.mouse.click(box.x + box.width - 4, box.y + box.height * 0.8);
  await expect.poll(() => page.evaluate(() => App.player.state.volume)).toBeLessThan(0.25);
  // the arrow keys work as before: up raises it
  const was = await page.evaluate(() => App.player.state.volume);
  await bar.press('ArrowUp');
  expect(await page.evaluate(() => App.player.state.volume)).toBeGreaterThan(was);
  // lull's own volume bar stays as it was
  await expect(page.locator('#pbar > :not(.region-slot) .vol .seek[aria-orientation]')).toHaveCount(0);

  // the hero says where the song plays from in the theme's words, and keeps them through another song and the queue
  await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
  });
  await page.evaluate(() => { location.hash = '#/now'; });
  await expect(page.locator('.tu-from .from__label')).toHaveText('out of');
  await page.evaluate(() => App.player.next());
  await expect(page.locator('.tu-from .from__label')).toHaveText('out of');
  await expect(page.locator('.tu-from .from__label')).toBeVisible();
  await ctx.close();
});

/* every own element of the theme's on the page whose words aren't what its template gave it, and how many there are */
const owned = (page) => page.evaluate(() => {
  const els = [...document.querySelectorAll('[data-own]')];
  return { count: els.length, changed: els.map((el) => [el.getAttribute('data-own'), [...el.childNodes].filter((n) => n.nodeType === 3).map((n) => n.data).join('').trim()])
    .filter(([want, got]) => want !== got) };
});

test('lull never writes over a theme\'s own markup: through play, keys, likes, the queue and every screen, its words stay', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  const check = async (where, count) => {
    const o = await owned(page);
    expect(o.changed, `${where}: a theme's own words lull changed`).toEqual([]);
    if (count != null) expect(o.count, `${where}: the theme's own elements on the page`).toBe(count);
  };
  await openScreen(page, `/album/${ids.album}`, { design: 'owned' });
  await check('an album, before playing');
  await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
  });
  await page.waitForFunction(() => App.player.state.playing);
  await check('playing');
  for (const [what, act] of [['next', () => App.player.next()], ['shuffle', () => App.player.setShuffle()], ['repeat', () => App.player.cycleRepeat()],
    ['pause', () => App.player.pause()], ['play', () => App.player.play()], ['play next', () => App.player.playNext([App.player.state.queue[App.player.state.queue.length - 1].trackId])],
    ['volume', () => App.player.setVolume(0.3)], ['mute', () => App.player.toggleMute()]]) {
    await page.evaluate(act);
    await settle(page);
    await check(what);
  }
  await page.locator('.ow-bar [data-act="like"]').first().click();
  await settle(page);
  await check('a like');
  await page.evaluate(() => App.togglePanel('queue'));
  await settle(page);
  await check('the panel open');
  await page.evaluate(() => { location.hash = '#/now'; });
  await settle(page);
  await expect(page.locator('.ow-hero .from__label')).toHaveText('out of');
  await check('Now Playing');
  await page.evaluate(() => App.player.next());
  await settle(page);
  await expect(page.locator('.ow-hero .from__label')).toHaveText('out of');
  await check('Now Playing, another song');
  await page.evaluate(() => { location.hash = '#/home'; });
  await settle(page);
  await check('Home');
  await ctx.close();
});

test('a theme places lull\'s sections and lists anywhere, with its own rows, and draws around the shell and a shelf', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  const feeds = [];
  page.on('request', (r) => { if (/\/api\/feed(\?|$)/.test(r.url())) feeds.push(r.url()); });
  await openScreen(page, `/album/${ids.album}`, { design: 'placed' });
  // the whole shell's edges: siblings of .app, outside its grid
  expect(await page.evaluate(() => [document.querySelector('#app').previousElementSibling?.getAttribute('data-slot'),
    document.querySelector('#app').nextElementSibling?.getAttribute('data-slot')])).toEqual(['before.app', 'after.app']);
  // Home's shelf and its list, away from Home: the feed is fetched for them, and they draw when it arrives
  await expect(page.locator('.pl-side lull-part[name="home.jump"] .disc-jump').first()).toBeAttached();
  await expect(page.locator('.pl-side li .pl-jump').first()).toBeAttached();
  expect(feeds.length, 'the feed, asked once').toBe(1);
  expect(await page.locator('.pl-side li .pl-jump').count()).toBeLessThanOrEqual(3);
  // the queue's groups, the theme's own rows filled from each song, through pipes
  const songs = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
    return rows.map((id) => App.data.track(id));
  });
  await expect(page.locator('.pl-side lull-part[of="queue"][where="group=place|radio"] .trow')).toHaveCount(2);
  await expect(page.locator('li .pl-row')).toHaveCount(0);   // nothing queued by the listener yet
  await page.evaluate((id) => App.player.playNext([id]), songs[songs.length - 1].id);
  await expect(page.locator('li .pl-row')).toHaveCount(1);
  const last = songs[songs.length - 1];
  await expect(page.locator('li .pl-row')).toHaveText(last.title.toUpperCase());
  await expect(page.locator('li .pl-row')).toHaveAttribute('data-id', last.id);
  const mins = Math.floor(last.duration / 60), secs = String(last.duration % 60).padStart(2, '0');
  await expect(page.locator('li .pl-len')).toHaveText(`${mins}:${secs}`);
  // Home: the box after the jump-back-in shelf, kept when the shelf is drawn again
  await page.evaluate(() => { location.hash = '#/home'; });
  await settle(page);
  await expect(page.locator('[data-region="jump"] > .region-slot[data-slot="after.jump"]')).toHaveCount(1);
  await page.evaluate(() => App.emit('server:feed.updated', {}));
  await settle(page);
  await expect(page.locator('[data-region="jump"] > .region-slot[data-slot="after.jump"]')).toHaveCount(1);
  // back on a built-in, every box goes, the shell's too
  await page.evaluate(() => new Promise((ok) => { App.on('design', ok); App.setDesign('lull', true); }));
  await settle(page);
  expect(await page.evaluate(() => document.querySelectorAll('.region-slot').length)).toBe(0);
  await ctx.close();
});

test('the contract theme: its own rows with lull\'s parts for each song, any read, its own tabs, a piece of another screen', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await page.clock.install({ time: new Date('2026-11-08T14:12:00Z') });   // a Sunday
  await openScreen(page, `/album/${ids.album}`, { design: 'hard' });
  await expect(page.locator('body > .region-slot[data-slot="before.app"] .hd-day')).toHaveText('SUN');
  const songs = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
    return rows.slice(1, 6).map((id) => App.data.track(id));
  });
  // the queue in its own rows: lull's cover, Like and play for each song, its source mark and length by pipes
  const rows = page.locator('.hd-q li.lull-list__row');
  await expect(rows).toHaveCount(5);
  const first = rows.first(), want = songs[0];
  await expect(first.locator('.hd-t')).toHaveText(want.title);
  await expect(first.locator('lull-part[name="cover"] .cover')).toHaveCount(1);
  await expect(first.locator('lull-part[name="like-button"] [data-act="like"]')).toHaveAttribute('data-id', want.id);
  await expect(first.locator('lull-part[name="play-button"] [data-act="now-q-jump"]'), 'a queued song plays where it is').toHaveAttribute('data-id', want.id);
  expect(['● LIB', '◆ SLSK', '▼ DEV']).toContain(await first.locator('.hd-s').textContent());
  await expect(first.locator('.hd-l')).toHaveText(`${Math.floor(want.duration / 60)}:${String(want.duration % 60).padStart(2, '0')}`);
  // its Like likes that song, and shows it
  await first.locator('[data-act="like"]').click();
  await expect(first.locator('[data-act="like"]')).toHaveAttribute('aria-pressed', 'true');
  expect(await page.evaluate((id) => App.isLiked(id), want.id)).toBe(true);
  // its own tabs: one panel shown, the key says which; the other on a click, and never none
  const tabs = page.locator('.hd-side [role="tab"]');
  await expect(tabs.nth(0)).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('.hd-a')).toBeHidden();
  await tabs.nth(1).click();
  await expect(page.locator('.hd-q')).toBeHidden();
  await expect(page.locator('.hd-a')).toBeVisible();
  await expect(tabs.nth(1)).toHaveAttribute('aria-selected', 'true');
  await tabs.nth(1).click();
  await expect(page.locator('.hd-a'), 'a set state stays set').toBeVisible();
  // a list from any read lull documents: the library's albums, each with lull's cover
  await expect(page.locator('.hd-a li.lull-list__row').first()).toBeVisible();
  await expect(page.locator('.hd-a li.lull-list__row').first().locator('.cover')).toHaveCount(1);
  // a piece of another screen, drawn as that screen draws it
  await expect(page.locator('.hd-side lull-part[name="screen"] [data-region="jump"]')).toHaveCount(1);
  await ctx.close();
});

test('the contract theme, the walls found since: queue rows play in the queue, Activity and playlists follow lull, a playing place is marked', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, `/album/${ids.album}`, { design: 'hard' });
  await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
  });
  // a queue row's play key plays that song where it is in the queue, and its menu is the queue's
  const q = page.locator('.hd-q li.lull-list__row');
  await expect(q.nth(2).locator('[data-act="now-q-jump"]')).toHaveCount(1);
  const before = await page.evaluate(() => App.player.state.queue.map((e) => e.trackId).sort().join());
  const target = await q.nth(2).getAttribute('data-id');
  await q.nth(2).locator('[data-act="now-q-jump"]').click();
  await expect.poll(() => page.evaluate(() => App.player.current().id)).toBe(target);
  expect(await page.evaluate(() => App.player.state.queue.map((e) => e.trackId).sort().join()), 'the same songs queued, as lull\'s own queue row leaves them').toBe(before);
  // Activity, as lull keeps it in the page: drawn again when lull changes it
  await expect(page.locator('.hd-more lull-part[of="activity"]')).toContainText('NO ACTIVITY');
  await page.evaluate(() => App.store.set('activity', [{ id: 'job-1', jobId: 'job-1', kind: 'transfer', trackId: App.player.current().id, state: 'transferring', pct: 40 }]));
  await expect(page.locator('.hd-job').filter({ hasText: 'transfer transferring' })).toHaveCount(1);
  // a playlist's own row: its mosaic cover
  const pl = page.locator('.hd-more lull-part[of="playlists"] li.lull-list__row');
  await expect(pl.first().locator('.mosaic')).toHaveCount(1);
  // Home's list drawn after the feed came: the place the song plays from is marked at once
  await expect(page.locator('.hd-more lull-part[of="home.jump"] li.lull-list__row').first()).toBeAttached();
  const place = await page.locator('.hd-more lull-part[of="home.jump"] li.lull-list__row[data-place^="album:"]').first().getAttribute('data-place');
  await page.locator(`.hd-more li.lull-list__row[data-place="${place}"] [data-act="play-album"]`).click();
  await expect(page.locator(`.hd-more li.lull-list__row[data-place="${place}"]`)).toHaveClass(/is-playing-place/);
  await ctx.close();
});
