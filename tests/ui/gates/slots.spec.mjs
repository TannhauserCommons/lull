// A theme's markup in every region and screen, and lull's parts in every slot (ADR-0101, docs/themes.md section 6), in a
// real page: a listener's theme puts an LCD after the player bar (the copy's quality by the live model, and a shuffle
// key of its own), an LED before the header (the song's title, and lull's Like), a box before Home and the queue's next
// songs after the side panel. Each is drawn where the theme said, stays through what lull draws again (another song,
// the panel, another screen), keeps current, and the key works; a built-in design gets none of it. On a fixture hub of
// the gate's own, serving the theme as the hub would.

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

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-slots-'));
  mkdirSync(join(dir, 'slotted', 'slots'), { recursive: true });
  writeFileSync(join(dir, 'slotted', 'theme.json'), JSON.stringify(MANIFEST));
  writeFileSync(join(dir, 'slotted', 'theme.css'), CSS);
  for (const [name, html] of Object.entries(SLOTS)) writeFileSync(join(dir, 'slotted', 'slots', `${name}.html`), html);
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

for (const design of designs) {
  test(`${design}, which has no region or screen slot, gets no box and no part's box`, async ({ browser }) => {
    const ctx = await context(browser), page = await ctx.newPage();
    for (const route of ['/home', '/now', `/album/${ids.album}`]) {
      await openScreen(page, route, { design });
      expect(await page.evaluate(() => [document.querySelectorAll('.region-slot').length, document.querySelectorAll('lull-part').length]), route)
        .toEqual([0, 0]);
    }
    await ctx.close();
  });
}
