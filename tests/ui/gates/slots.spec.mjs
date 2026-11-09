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
const UPRIGHT_CSS = `#pbar > [data-slot="after.playerbar"] { position: fixed; right: 40px; bottom: 120px; }
.tu-fader .seek[aria-orientation="vertical"] { width: 24px; min-width: 0; height: 160px; }`;

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-slots-'));
  for (const [id, slots, css] of [['slotted', SLOTS, CSS], ['handed', HANDED, ''], ['parted', PARTED, ''], ['upright', UPRIGHT, UPRIGHT_CSS]]) {
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
