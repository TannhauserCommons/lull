// Right-click (and the Menu key, and a touch hold) on anything that names a song, album, artist or playlist opens
// lull's menu of that thing, whose Copy link copies the link that works for anyone (ADR-0032), never the browser's
// menu, whose "Copy link" copies the hub's own address. A listener (2026-11) right-clicked a long name and got the hub's
// address: lull chose its menu from a list of places (rows, tiles, the player bar), and names elsewhere (Stats,
// Following, "Playing from" in the queue) fell through. Empty space keeps the browser's menu (owner, 2026-11-04).
// Tags have no share link, so a tag keeps the browser's menu.
import { expect, test } from '@playwright/test';
import { ids, openScreen, settle } from './lib.mjs';

const NAMES = 'a[href^="#/album/"], a[href^="#/artist/"], a[href^="#/playlist/"], [data-track-id], [data-act="play-track"][data-id]';

/* every visible element that names an item: how many, and those whose right-click the browser kept (lull's menu
   prevents the browser's); a popover a right-click opened is closed before the next */
async function probe(page) {
  return page.evaluate((sel) => {
    const missed = []; let seen = 0;
    for (const el of document.querySelectorAll(sel)) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || r.bottom < 0 || r.top > innerHeight || r.right < 0 || r.left > innerWidth) continue;
      const x = r.left + Math.min(r.width / 2, 8), y = r.top + r.height / 2;
      const target = document.elementFromPoint(x, y);
      if (!target || !el.contains(target)) continue;   // covered by something else, which is checked on its own
      seen++;
      const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: x, clientY: y, button: 2 });
      target.dispatchEvent(ev);
      if (!ev.defaultPrevented) missed.push((el.parentElement.className.toString().split(' ')[0] || el.parentElement.tagName) + ' > ' + el.tagName.toLowerCase() + ' "' + el.textContent.trim().slice(0, 30) + '"');
      App.closePopover();
    }
    return { seen, missed: [...new Set(missed)] };
  }, NAMES);
}

const SCREENS = ['/home', '/search?q=velvet', '/library/albums', '/library/artists', '/library/songs', '/library/genres', '/liked',
  '/history', '/following', '/playlists', '/stats', '/now', `/playlist/${ids.playlist}`, `/album/${ids.album}`, `/artist/${ids.artist}`];
// where the listener's names fell through: each must have names to check, or the gate proves nothing there
const MUST_SEE = ['/stats', '/following'];

for (const design of ['lull', 'starlight', 'skydeck', 'petal', 'glasshouse']) {
  test(`${design}: a right-click on any song, album, artist or playlist, on every screen and side panel, opens lull's menu`, async ({ page }) => {
    test.setTimeout(180_000);
    await openScreen(page, `/album/${ids.album}`, { design });
    await page.locator('.trow[data-track-id]').first().dblclick();   // a queue, so the queue panel has "Playing from"
    const missed = [], empty = [];
    for (const route of SCREENS) {
      await page.evaluate((r) => { location.hash = '#' + r; }, route);
      await settle(page);
      for (const tab of ['queue', 'lyrics', 'activity']) {
        await page.evaluate((t) => App.openPanel(t), tab);
        await settle(page);
        const { seen, missed: m } = await probe(page);
        if (MUST_SEE.includes(route) && tab === 'queue' && !seen) empty.push(route);
        m.forEach((x) => missed.push(route + ' [' + tab + '] ' + x));
      }
    }
    expect(empty, 'screens that drew no names to check').toEqual([]);
    expect([...new Set(missed.map((m) => m.replace(/^\S+ \[\w+\] /, '')))], missed.slice(0, 30).join('\n')).toEqual([]);
  });
}

/* the menu a right-click opens is the named thing's: it is named for it, and its Copy link (where it has one) asks
   the hub for that thing's share link */
async function menuOf(page, locator) {
  await locator.click({ button: 'right' });
  const menu = page.locator('[role="menu"]').last();
  await expect(menu).toBeVisible();
  const label = await menu.getAttribute('aria-label');
  const copy = menu.locator('[role="menuitem"]').filter({ hasText: 'Copy link' });
  let share = null;
  if (await copy.count()) {
    const asked = page.waitForRequest((r) => /\/api\/share\//.test(r.url()), { timeout: 5000 });
    await copy.first().click();
    share = new URL((await asked).url()).pathname.replace(/^.*\/api\/share\//, '');
  } else await page.evaluate(() => App.closePopover());
  return { label, share };
}

test('a right-click on a Stats artist, a Stats song and "Playing from" opens that thing\'s menu', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await openScreen(page, `/album/${ids.album}`);
  await page.locator('.trow[data-track-id]').first().dblclick();
  await page.evaluate(() => { location.hash = '#/stats'; });
  await settle(page);
  const artist = page.locator('a.sys-bars__name[href^="#/artist/"]').first();
  const artistId = decodeURIComponent((await artist.getAttribute('href')).split('/')[2]);
  const a = await menuOf(page, artist);
  expect(a.label).toContain(await artist.textContent());
  expect(a.share).toBe(`artist/${artistId}`);
  const song = page.locator('button.sys-bars__name[data-act="play-track"][data-id]').first();
  const s = await menuOf(page, song);
  expect(s.label).toContain(await song.textContent());
  if (s.share) expect(s.share).toBe(`track/${await song.getAttribute('data-id')}`);
  await page.evaluate(() => App.openPanel('queue'));
  await settle(page);
  const from = page.locator('#ctx a[href^="#/album/"]').first();
  const albumId = decodeURIComponent((await from.getAttribute('href')).split('/')[2]);
  const f = await menuOf(page, from);
  expect(f.share).toBe(`album/${albumId}`);
});

/* a heart's menu is the playlist picker: a click likes, a right-click adds to a playlist and likes nothing; on a
   selected row it takes the whole selection, as the row's own menu does. Its tooltip says so, for any heart */
test('a right-click on a heart opens the playlist picker for its song, or the selection, and likes nothing', async ({ page }) => {
  await openScreen(page, `/album/${ids.album}`);
  const rows = page.locator('#view .trow[data-track-id]');
  const heart = rows.nth(1).locator('[data-act="like"]');
  const before = await heart.getAttribute('aria-pressed');
  await heart.hover();
  await expect(page.locator('#tooltip')).toHaveText(/· right-click for playlists$/);
  await heart.click({ button: 'right' });
  const title = await rows.nth(1).locator('.trow__title').textContent();
  await expect(page.locator('.lib-picker__what')).toHaveText(`Adding ${title}. Tick every playlist it goes in, then Done.`);
  await expect(heart, 'the right-click liked nothing').toHaveAttribute('aria-pressed', before);
  await page.keyboard.press('Escape');
  await expect(page.locator('.lib-picker')).toHaveCount(0);
  // three rows selected (a click on a row's empty space, then Shift): the heart of one of them takes all three
  const spot = async (i) => ({ x: (await rows.nth(i).boundingBox()).width * 0.6, y: 20 });
  await rows.nth(1).click({ position: await spot(1) });
  await rows.nth(3).click({ position: await spot(3), modifiers: ['Shift'] });
  await expect(page.locator('#view .trow.is-selected')).toHaveCount(3);
  await rows.nth(2).locator('[data-act="like"]').click({ button: 'right' });
  await expect(page.locator('.lib-picker__what')).toHaveText('Adding 3 songs. Tick every playlist it goes in, then Done.');
});
