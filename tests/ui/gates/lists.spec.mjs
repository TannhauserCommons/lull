// A long list draws only the rows near the screen (ui.row, docs/context/web.md, Long lists cost what short ones do).
// What a listener does with a long list must work on every row, drawn or not, and a row drawn again must show its song
// as it is now. On a fixture hub of its own: these change likes, downloads and a playlist's order.

import { expect, test as base } from '@playwright/test';
import { designs, ids, openScreen, ownHub, settle } from './lib.mjs';

const test = base.extend({
  hub: [async ({}, use) => { const hub = await ownHub(); await use(hub); hub.close(); }, { scope: 'worker' }],
  baseURL: async ({ hub }, use) => use(hub.url),
});

const LIST = '#view .lib-pl-list';
const rows = (page) => page.locator(`${LIST} > li[data-track-id]`);
// the view's scroller (the page's in some shells)
const scrollTo = (page, y) => page.evaluate((y) => {
  let s = document.querySelector('#view li[data-fill]');
  while (s && !(s.scrollHeight > s.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
  (s || document.scrollingElement).scrollTop = y === 'end' ? 1e9 : y;
}, y);
const frames = (page, n = 3) => page.evaluate((n) => new Promise((ok) => { const f = (k) => (k ? requestAnimationFrame(() => f(k - 1)) : ok()); f(n); }), n);
const drawn = (row) => row.evaluate((li) => !!li.firstElementChild);
// every row on screen has its contents
const blankOnScreen = (page) => page.evaluate((sel) => [...document.querySelectorAll(sel)].filter((li) => {
  const r = li.getBoundingClientRect();
  return r.bottom > 0 && r.top < innerHeight && !li.firstElementChild;
}).length, `${LIST} > li[data-track-id]`);
// what each drawn row shows against its song and the state of the moment
const mismatches = (page) => page.evaluate((sel) => [...document.querySelectorAll(sel)].filter((li) => li.firstElementChild).flatMap((li) => {
  const id = li.getAttribute('data-track-id'), t = App.data.track(id), out = [];
  const q = (s) => li.querySelector(s);
  if (q('.trow__title').textContent !== t.title) out.push(id + ' title');
  if (q('[data-like]').getAttribute('data-like') !== id || q('[data-like]').getAttribute('aria-pressed') !== String(App.isLiked(id))) out.push(id + ' like');
  if (q('[data-src-mini] svg').getAttribute('aria-label') !== (App.isSaved(id) ? 'Saved' : 'Soulseek')) out.push(id + ' source');
  if (q('[data-dl-mark]').hasAttribute('data-on') !== App.downloads.has(id)) out.push(id + ' download');
  if (!!q('.trow__num > .eq') !== (App.player.currentId() === id)) out.push(id + ' bars');
  const img = q('.cover__img');
  if (img && img.getAttribute('data-art') !== t.albumId) out.push(id + ' cover');
  return out;
}), `${LIST} > li[data-track-id]`);

test('every song has its row, named for screen readers and reachable from the keyboard', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  const facts = await rows(page).evaluateAll((lis) => ({
    count: lis.length,
    empty: lis.filter((li) => !li.firstElementChild).length,
    unnamed: lis.filter((li) => !li.getAttribute('aria-label') || li.getAttribute('tabindex') !== '0').length,
  }));
  expect(facts.count, 'rows').toBe(1200);
  expect(facts.empty, 'rows away from the screen are empty').toBeGreaterThan(1000);
  expect(facts.unnamed, 'rows without a name or a place in the tab order').toBe(0);
  expect(await blankOnScreen(page), 'blank rows on screen when opened').toBe(0);

  await rows(page).nth(0).focus();
  for (let i = 0; i < 40; i++) await page.keyboard.press('ArrowDown');
  await frames(page);
  const at = await page.evaluate((sel) => {
    const a = document.activeElement, r = a.getBoundingClientRect();
    return { i: [...document.querySelectorAll(sel)].indexOf(a), drawn: !!a.firstElementChild, onScreen: r.top >= 0 && r.bottom <= innerHeight };
  }, `${LIST} > li[data-track-id]`);
  expect(at, 'forty rows down by the arrow keys').toEqual({ i: 40, drawn: true, onScreen: true });

  await scrollTo(page, 'end');
  await page.waitForTimeout(300);
  expect(await drawn(rows(page).nth(40)), 'the focused row keeps its contents when scrolled far away').toBe(true);
});

test('a selection reaches across rows never drawn', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  await rows(page).nth(1).locator('.trow__album').click();
  expect(await drawn(rows(page).nth(200)), 'row 200 not drawn yet').toBe(false);
  await rows(page).nth(400).scrollIntoViewIfNeeded();
  await frames(page);
  await rows(page).nth(400).locator('.trow__album').click({ modifiers: ['Shift'] });
  const sel = await page.evaluate((sel) => {
    const lis = [...document.querySelectorAll(sel)];
    return { selected: lis.filter((li) => li.classList.contains('is-selected')).length, ids: App.selectedIds(lis[400]).length, row200: lis[200].classList.contains('is-selected') };
  }, `${LIST} > li[data-track-id]`);
  expect(sel).toEqual({ selected: 400, ids: 400, row200: true });
});

test('a song far down a playlist moves by dragging it', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  const order = () => rows(page).evaluateAll((lis) => lis.map((li) => li.getAttribute('data-track-id')));
  const before = await order(), moving = before[800];
  const row = page.locator(`${LIST} > li[data-track-id="${moving}"]`);
  await row.scrollIntoViewIfNeeded();
  await frames(page);
  const from = await row.locator('.trow__album').boundingBox();
  await page.mouse.move(from.x + 4, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + 4, from.y + from.height / 2 + 12, { steps: 3 });
  // back to the top while dragging: the dragged row is far from the screen now and must keep its contents
  await scrollTo(page, 0);
  await frames(page);
  expect(await drawn(row), 'the dragged row keeps its contents far from the screen').toBe(true);
  const to = await rows(page).nth(3).boundingBox();
  await page.mouse.move(to.x + 40, to.y + 6, { steps: 5 });
  await page.mouse.up();
  await expect.poll(async () => (await order()).indexOf(moving), 'where the song landed').toBe(3);
  expect((await order()).length).toBe(before.length);
});

test('the queue drags a song far down it', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .lib-pl-list > li[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await page.evaluate(() => App.openPanel('queue'));
  await settle(page);
  const queue = () => page.evaluate(() => App.player.state.queue.map((q) => q.trackId));
  const before = await queue(), row = page.locator('.ctx .now-q__row[data-qi="600"]');
  await row.scrollIntoViewIfNeeded();
  await frames(page);
  expect(await drawn(row), 'the queue row scrolled to is drawn').toBe(true);
  const a = await row.boundingBox();
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
  await page.mouse.down();
  for (let k = 1; k <= 20; k++) await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2 + (a.height * 3.4 * k) / 20);
  await page.mouse.up();
  await expect.poll(async () => (await queue()).indexOf(before[600]), 'where the song landed in the queue').toBe(603);
});

// "Playing from" opens the playlist at a song 900 rows down: it lands lit, drawn, focused and fully in sight (not under
// the header, the player bar or the tab bar), and stays there while the rows around it are drawn, which needs every
// empty row to hold the height it has drawn (app.js holdPlace). In every design, on a laptop and a phone.
for (const design of designs) {
  for (const [size, use] of Object.entries({ laptop: { viewport: { width: 1440, height: 900 } }, phone: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true } })) {
    test.describe(`${design}, ${size}`, () => {
      test.use(use);
      test('the playing song is shown where it plays from, and stays put', async ({ page }) => {
        await openScreen(page, `/playlist/${ids.longPlaylist}`, { design });
        if (size === 'laptop') {
          const heights = await rows(page).evaluateAll((lis) => ({
            drawn: [...new Set(lis.filter((li) => li.firstElementChild).map((li) => Math.round(li.getBoundingClientRect().height)))],
            empty: [...new Set(lis.filter((li) => !li.firstElementChild).map((li) => Math.round(li.getBoundingClientRect().height)))],
          }));
          expect(heights.empty, 'an empty row holds a drawn row\'s height').toEqual(heights.drawn);
        }
        const id = await rows(page).nth(900).getAttribute('data-track-id');
        await page.evaluate((i) => { const lis = [...document.querySelectorAll('#view .lib-pl-list > li[data-track-id]')]; App.player.playList(lis.map((li) => li.getAttribute('data-track-id')), i); App.player.pause(); }, 900);
        await expect.poll(() => page.evaluate(() => App.player.currentId())).toBe(id);
        await page.evaluate(() => { location.hash = '/home'; });
        await settle(page);
        await page.evaluate(() => App.goPlaying());
        const row = page.locator(`${LIST} > li[data-track-id="${id}"]`);
        await expect(row).toHaveClass(/is-playing/);
        await frames(page, 30);   // a design's own easing on scroll (glasshouse's, 8 px, on main too) has finished
        const at = (r) => r.evaluate((li) => {
          const b = li.getBoundingClientRect(), x = b.left + b.width / 2, seen = (y) => li.contains(document.elementFromPoint(x, y));
          return { top: Math.round(b.top), inSight: seen(b.top + 2) && seen(b.bottom - 2), drawn: !!li.firstElementChild, focused: document.activeElement === li };
        });
        const first = await at(row);
        expect(first, 'where the playing row lands').toEqual({ top: first.top, inSight: true, drawn: true, focused: true });
        await frames(page, 20);
        expect((await at(row)).top, 'the playing row stays where it landed').toBe(first.top);
        expect(await page.locator('#view .trow .trow__num > .eq').count(), 'playing bars in the list').toBeLessThanOrEqual(1);
        expect(await mismatches(page), 'drawn rows showing another song').toEqual([]);
      });
    });
  }
}

test('likes and downloads made while a row is away show when it is drawn', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  const id = await rows(page).nth(100).getAttribute('data-track-id');
  const liked = await page.evaluate((id) => App.isLiked(id), id);
  await scrollTo(page, 'end');
  await expect.poll(() => drawn(rows(page).nth(100)), 'row 100 emptied once far away').toBe(false);
  await page.evaluate((id) => { App.toggleLike(id, true); return App.downloads.keep({ kind: 'track', id }, [id]); }, id);
  await expect.poll(() => page.evaluate((id) => App.downloads.has(id), id), 'the download finished').toBe(true);
  await rows(page).nth(100).scrollIntoViewIfNeeded();
  await frames(page);
  const row = rows(page).nth(100);
  await expect(row.locator('[data-like]')).toHaveAttribute('aria-pressed', 'true');
  await expect(row.locator('[data-dl-mark]')).toHaveAttribute('data-on', '');
  // and back, while away again
  await scrollTo(page, 'end');
  await expect.poll(() => drawn(rows(page).nth(100))).toBe(false);
  await page.evaluate(([id, liked]) => { App.toggleLike(id, liked); App.downloads.remove([id]); }, [id, liked]);
  await rows(page).nth(100).scrollIntoViewIfNeeded();
  await frames(page);
  await expect(row.locator('[data-like]')).toHaveAttribute('aria-pressed', String(liked));
  await expect(row.locator('[data-dl-mark]')).not.toHaveAttribute('data-on', '');
});

test('rows drawn while scrolling up and down each show their own song', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  const height = await page.evaluate(() => document.querySelector('#view .lib-pl-list').getBoundingClientRect().height);
  for (const y of [height * 0.5, height * 0.2, height, height * 0.7, 0]) {
    await scrollTo(page, y);
    await frames(page, 4);
    expect(await blankOnScreen(page), `blank rows on screen at ${Math.round(y)} px`).toBe(0);
    expect(await mismatches(page), `drawn rows showing another song at ${Math.round(y)} px`).toEqual([]);
  }
});

test.describe('on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  test('touch scrolling draws the rows it reaches', async ({ page }) => {
    await openScreen(page, `/playlist/${ids.longPlaylist}`);
    const cdp = await page.context().newCDPSession(page);
    for (const y of [-4000, -4000, -6000, 3000]) {
      await cdp.send('Input.synthesizeScrollGesture', { x: 200, y: 500, yDistance: y, speed: 4000, gestureSourceType: 'touch' });
      await frames(page, 4);
      expect(await blankOnScreen(page), `blank rows on screen after a ${-y} px swipe`).toBe(0);
    }
    expect(await mismatches(page), 'drawn rows showing another song').toEqual([]);
  });
});
