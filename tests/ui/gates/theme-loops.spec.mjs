// A theme's loops that draw more than they need to (docs/themes.md, Drawing cheaply), named by Theme Studio and when a
// theme is imported (web/designs/loops.js), each with what it costs and the cheaper way. A hint, never a block:
// - no rate: it redraws on every screen refresh and ignores "Pause animations when lull isn't the active window". Add
//   it writes the guide's rate; then lull paces the loop, measured as the loops gate does: stepped to the rate it
//   names, and held while the page asks loops to hold still;
// - it animates more than transform and opacity, so lull redraws it at every step;
// - it sits inside glass (the CSS says so), or passes under it (Theme Studio's live previews show it; import says it
//   can't tell). The five built-in designs get none of these.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';
import { failingAlbum } from '../fixture-hub/library.mjs';

// falling petals: two pseudo-elements on the page, drifting down forever
const PETALS = `/* falling petals */
body::before, body::after { content: ""; position: fixed; z-index: 9; top: -5vh; left: 30%; width: 14px; height: 10px;
  border-radius: 70% 0; background: #f4a7c0; pointer-events: none; animation: petal-fall 9s linear infinite; }
body::after { left: 70%; animation-duration: 13s; }
@keyframes petal-fall { to { transform: translate(-6vw, 110vh) rotate(320deg); } }
`;
// the petals' loops in a page: the rate each names, whether it is stepped, its playback rate (0 = held)
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
const OK = 'Every loop names its rate, moves by transform and opacity, and stays off glass on this screen.';
const WORDS_OK = 'Every state the player shows is in words that hold for every song.';
const petals = (frame) => frame.evaluate(() => document.getAnimations().filter((a) => a.animationName === 'petal-fall').map((a) => {
  const e = a.effect, cs = getComputedStyle(e.target, e.pseudoElement);
  return { fps: parseFloat(cs.getPropertyValue('--loop-fps')) || 0, stepped: /steps/.test(e.getTiming().easing), rate: a.playbackRate };
}));

test('Theme Studio names a loop with no rate and what it costs, adds the rate, and the preview paces the loop', async ({ page }, info) => {
  await openScreen(page, '/settings/appearance', { clock: false });
  await page.getByRole('button', { name: 'Open Theme Studio' }).click();
  const list = page.locator('[data-studio-loops]');
  await expect(list).toContainText(OK);

  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill(PETALS);
  await expect(list).toContainText('petal-fall on body::before, body::after, line 3.');
  await expect(list.locator('h4')).toHaveText(['No rate']);
  await expect(list.locator('.field__help')).toHaveText(/^It never ends and names no rate, so it redraws on every screen refresh, up to 165 times a second/);
  await expect(list).toContainText('It moves, so 20 a second suits it.');
  await list.locator('xpath=..').screenshot({ path: info.outputPath('studio-loops-before.png') });

  // the preview runs it at the screen's rate: no rate, not stepped, not held
  const frame = page.frames().find((f) => /index\.html#\/home\?.*theme=day/.test(f.url()));
  await expect.poll(() => petals(frame).then((p) => p.length)).toBe(2);
  expect(await petals(frame)).toEqual([{ fps: 0, stepped: false, rate: 1 }, { fps: 0, stepped: false, rate: 1 }]);

  await page.getByRole('button', { name: 'Add --loop-fps: 20 to body::before, body::after' }).click();
  await expect(page.locator('[data-studio="css"]')).toHaveValue(/animation: petal-fall 9s linear infinite; --loop-fps: 20; \}/);
  await expect(list).toContainText(OK);
  await list.locator('xpath=..').screenshot({ path: info.outputPath('studio-loops-after.png') });

  // paced: stepped to 20 a second, and held while the page asks
  await expect.poll(() => petals(frame), { timeout: 5000 }).toEqual([{ fps: 20, stepped: true, rate: 1 }, { fps: 20, stepped: true, rate: 1 }]);
  await frame.evaluate(() => App.still('gate', true));
  expect((await petals(frame)).map((p) => p.rate)).toEqual([0, 0]);
  await frame.evaluate(() => App.still('gate', false));
  expect((await petals(frame)).map((p) => p.rate)).toEqual([1, 1]);
});

// the hub's side of an import (the fixture hub serves themes but doesn't store them): the theme as stored, and its edits
async function importing(page, id, name, css) {
  const stored = { css, puts: 0 };
  const item = { id, name, api: 1, extends: 'lull', shell: 'rail', modes: ['day', 'night'], builtIn: false };
  await page.route('**/api/themes', (r) => (r.request().method() === 'POST' ? r.fulfill({ json: item }) : r.fallback()));
  await page.route(`**/api/themes/${id}/theme.css`, (r) => {
    if (r.request().method() !== 'PUT') return r.fulfill({ body: stored.css, contentType: 'text/css' });
    stored.css = r.request().postData(); stored.puts++;
    return r.fulfill({ json: item });
  });
  await openScreen(page, '/settings/appearance', { clock: false });
  return stored;
}
const drop = (name) => ({ name: name + '.lulltheme', mimeType: 'application/zip', buffer: Buffer.from('PK') });

test('an imported theme with a loop that names no rate is offered it, and works as it is without', async ({ page }, info) => {
  const stored = await importing(page, 'petals', 'Petals', PETALS);

  await page.locator('[data-theme-file]').setInputFiles(drop('petals'));
  const dialog = page.getByRole('dialog', { name: 'A loop in Petals could draw less' });
  await expect(dialog).toContainText('petal-fall on body::before, body::after. It moves, so 20 a second suits it.');
  await expect(dialog).toContainText('Petals works as it is');
  await dialog.screenshot({ path: info.outputPath('import-loops.png') });
  await dialog.getByRole('button', { name: 'Not now' }).click();
  await expect(dialog).toBeHidden();
  expect(stored.puts, 'Not now leaves the theme as it is').toBe(0);

  await page.locator('[data-theme-file]').setInputFiles(drop('petals'));
  await page.getByRole('dialog', { name: 'A loop in Petals could draw less' }).getByRole('button', { name: 'Add it' }).click();
  await expect(page.locator('.toast').filter({ hasText: 'Added its rate to Petals' })).toBeVisible();
  expect(stored.css).toBe(PETALS.replace('linear infinite; }', 'linear infinite; --loop-fps: 20; }'));
});

// a glow that repaints, on the play button in the player bar made glass, and a speck that drifts under that glass
const GLASSY = `/* the player bar as glass */
[data-region="playerbar"] { backdrop-filter: blur(14px); }
/* the play button breathes a glow */
[data-region="playerbar"] .btn-gloss { animation: glow 2s ease-in-out infinite alternate; --loop-fps: 10; }
@keyframes glow { to { box-shadow: 0 0 18px 6px #f4a7c0; } }
/* a speck that drifts across the bottom of the window */
body::before { content: ""; position: fixed; z-index: 9; left: 45%; bottom: 24px; width: 24px; height: 24px; border-radius: 50%;
  background: #f4a7c0; pointer-events: none; animation: drift 6s linear infinite alternate; --loop-fps: 20; }
@keyframes drift { to { transform: translateX(120px); } }
`;

test('Theme Studio names a loop that redraws at every step, one inside glass, and one its previews show passing under glass', async ({ page }, info) => {
  await openScreen(page, '/settings/appearance', { clock: false });
  await page.getByRole('button', { name: 'Open Theme Studio' }).click();
  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill(GLASSY);
  const list = page.locator('[data-studio-loops]');
  await expect(list.locator('h4')).toHaveText(['Redrawn at every step', 'Under glass'], { timeout: 10000 });
  await expect(list).toContainText('glow on [data-region="playerbar"] .btn-gloss, line 4 changes box-shadow. For a glow, draw it once on a ::before or ::after and fade and grow that with opacity and transform: scale().');
  await expect(list).toContainText('lull can’t rewrite it for you and be sure it looks the same, so here’s the cheaper way.');
  await expect(list.locator('li', { hasText: 'glow' })).toHaveCount(2);           // redrawn, and inside glass: not again from the previews
  await expect(list).toContainText('glow on [data-region="playerbar"] .btn-gloss, line 4 sits inside [data-region="playerbar"].');
  // only where things land says the speck passes under the glass: the live previews, day and night
  await expect(list).toContainText(/drift on body::before passes under \S+ on Home, in the day and night previews\./, { timeout: 10000 });
  await expect(list).toContainText('or bake the glass');
  expect(await list.getByRole('button', { name: /^Add --loop-fps/ }).count(), 'nothing it can rewrite and be sure').toBe(0);
  await list.locator('xpath=..').screenshot({ path: info.outputPath('studio-glass.png') });

  // the speck moved up, clear of the bar: measured by its own box, not by the page it belongs to
  await page.locator('[data-studio="css"]').fill(GLASSY.replace('bottom: 24px', 'bottom: 200px'));
  await expect(list).not.toContainText('drift', { timeout: 10000 });
  await page.waitForTimeout(2500);
  await expect(list).not.toContainText('drift');
});

test('an imported theme is told what its loops redraw, and that the CSS alone can\'t tell what passes under glass', async ({ page }, info) => {
  const stored = await importing(page, 'glassy', 'Glassy', GLASSY.replace(/\[data-region="playerbar"\] \.btn-gloss/, '.brand'));
  await page.locator('[data-theme-file]').setInputFiles(drop('glassy'));
  const dialog = page.getByRole('dialog', { name: 'A loop in Glassy could draw less' });
  await expect(dialog.locator('h4')).toHaveText(['Redrawn at every step', 'Under glass']);
  await expect(dialog).toContainText('glow on .brand changes box-shadow.');
  await expect(dialog).toContainText('lull can’t tell from the CSS alone whether glow or drift pass under the glass of [data-region="playerbar"]. Theme Studio checks it on its live preview.');
  await dialog.screenshot({ path: info.outputPath('import-glass.png') });
  await dialog.getByRole('button', { name: 'OK' }).click();
  await expect(dialog).toBeHidden();
  expect(stored.puts).toBe(0);
});

for (const design of DESIGNS) {
  test(`${design}: Theme Studio finds nothing to say about the design's own loops, on Home and on Now Playing`, async ({ page }) => {
    await page.addInitScript((d) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: 'day' })), design);
    await openScreen(page, '/settings/appearance', { design, clock: false });
    await page.getByRole('button', { name: 'Open Theme Studio' }).click();
    const list = page.locator('[data-studio-loops]');
    for (const route of ['home', 'now']) {
      await page.locator('[data-studio="route"]').selectOption(route);
      await page.waitForTimeout(2500);                        // the previews' loops started and looked at
      await expect(list).toHaveText(OK);
    }
    await expect(page.locator('[data-studio-words]')).toHaveText(WORDS_OK);
  });
}

// The player's words (docs/themes.md, The player's words): each state on #pbar, #mini and the mini player windows in
// lull's own words (data-status-text, and --status-text for anything drawn inside), the hub's reason for a failure among
// them, so a theme shows the right reason for every song instead of one of its own. A theme that words a reason of its
// own for such a state is told so, and offered lull's own words.
const CANNED = `/* the failed song's label, as themes wrote it */
.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  no peer answered"; color: #c0336a; }
`;
const STATES = [
  ['finding', { peersFound: 4 }, /^Finding a source, 4 peers answered/],
  ['queued', { peer: { name: 'mira' }, queuePos: 3 }, /^Queued at peer mira, position 3/],
  ['stalled', { peer: { name: 'mira', rank: 1 }, peersFound: 5 }, /^Peer stalled, switching to the next source/],
  ['choose', {}, /^No copy meets your streaming minimum/],
  ['failed', { error: 'YouTube couldn’t find this song' }, /^YouTube couldn’t find this song$/],
  ['jellyfin-error', { error: 'the file is missing' }, /^Library item unavailable: the file is missing$/],
  ['streaming', { peer: { name: 'YouTube' } }, /^Streaming from YouTube$/],
  ['ended', {}, /^End of queue$/],
];
// the player put in `status` with `fields`, and what #pbar and #mini then carry
const inState = (page, status, fields) => page.evaluate(([status, fields]) => {
  Object.assign(App.player.state, { seekWait: null, crossfade: null }, fields, { status });
  App.emit('player', 'status');
  const bar = document.querySelector('#pbar'), mini = document.querySelector('#mini');
  return { status: bar.dataset.status, text: bar.dataset.statusText, mini: mini.dataset.statusText,
    prop: getComputedStyle(bar).getPropertyValue('--status-text').trim(), inside: getComputedStyle(bar.querySelector('.pbar__seek')).getPropertyValue('--status-text').trim() };
}, [status, fields]);

test('the player says each state in its own words, for a theme to show', async ({ page }) => {
  await openScreen(page, `/album/${ids.album}`, { clock: false });
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await page.waitForFunction(() => App.player.current());
  await page.evaluate(() => App.player.pause());
  for (const [status, fields, words] of STATES) {
    const got = await inState(page, status, fields);
    expect(got.status, status).toBe(status);
    expect(got.text, status).toMatch(words);
    expect(got.mini, `${status}: the mini player`).toBe(got.text);
    expect(got.prop, `${status}: --status-text`).toBe(JSON.stringify(got.text));
    expect(got.inside, `${status}: inherited inside the bar`).toBe(got.prop);
  }
  // a theme's rule shows them, its own sign kept: right for this failure, and for the next one's different reason
  await page.addStyleTag({ content: '.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  " var(--status-text) !important; }' });
  const shown = () => page.evaluate(() => getComputedStyle(document.querySelector('.pbar__seek'), '::before').content);
  await inState(page, 'failed', { error: 'YouTube couldn’t find this song' });
  expect(await shown()).toBe('"⚠ YouTube couldn’t find this song"');
  await inState(page, 'failed', { error: 'No peer delivered this song.' });
  expect(await shown()).toBe('"⚠ No peer delivered this song."');
});

test('a song that fails for real carries the hub\'s reason in the player\'s words', async ({ page }) => {
  await openScreen(page, `/album/${failingAlbum}`, { clock: false });
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await page.waitForFunction(() => document.querySelector('#pbar').dataset.status === 'failed', null, { timeout: 15_000 });
  const got = await page.evaluate(() => ({ text: document.querySelector('#pbar').dataset.statusText, tab: document.querySelector('#pbar-state').textContent }));
  expect(got.text).toBeTruthy();
  expect(got.tab).toContain(got.text);
});

test('Theme Studio names a reason a theme words for itself, and puts lull\'s own words in its place', async ({ page }, info) => {
  await openScreen(page, '/settings/appearance', { clock: false });
  await page.getByRole('button', { name: 'Open Theme Studio' }).click();
  const words = page.locator('[data-studio-words]');
  await expect(words).toHaveText(WORDS_OK);
  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill(CANNED);
  await expect(words.locator('h4')).toHaveText(['Its own reasons']);
  await expect(words).toContainText('“⚠ no peer answered” on .pbar[data-status="failed"] .pbar__seek::before, line 2, for the failed state.');
  await expect(words).toContainText('a song can fail because no peer answered, because YouTube couldn’t find it');
  await words.locator('xpath=..').screenshot({ path: info.outputPath('studio-words.png') });
  await page.getByRole('button', { name: 'Use lull’s own words on .pbar[data-status="failed"] .pbar__seek::before' }).click();
  await expect(page.locator('[data-studio="css"]')).toHaveValue(CANNED.replace('"\\26A0  no peer answered"', '"\\26A0  " var(--status-text)'));
  await expect(words).toHaveText(WORDS_OK);
  // the preview's bar, its song failing for a reason that has nothing to do with peers, says that reason
  const frame = page.frames().find((f) => /index\.html#\/home\?.*theme=day/.test(f.url()));
  await expect.poll(() => frame.evaluate(() => !!(window.App && App.player && document.querySelector('#studio-draft')?.textContent.includes('var(--status-text)')))).toBe(true);
  await frame.evaluate((album) => { location.hash = '#/album/' + album; }, ids.album);
  await frame.waitForSelector('#view .trow[data-track-id]');
  await frame.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await frame.waitForFunction(() => App.player.current(), null, { timeout: 10_000 });
  await frame.evaluate(() => { App.player.pause(); Object.assign(App.player.state, { status: 'failed', error: 'YouTube couldn’t find this song' }); App.emit('player', 'status'); });
  expect(await frame.evaluate(() => getComputedStyle(document.querySelector('.pbar__seek'), '::before').content)).toBe('"⚠ YouTube couldn’t find this song"');
  await frame.locator('#pbar').screenshot({ path: info.outputPath('studio-words-preview.png') });
});

test('an imported theme that words a reason for itself is offered lull\'s own words', async ({ page }, info) => {
  const stored = await importing(page, 'canned', 'Canned', CANNED);
  await page.locator('[data-theme-file]').setInputFiles(drop('canned'));
  const dialog = page.getByRole('dialog', { name: 'Canned gives reasons of its own' });
  await expect(dialog).toContainText('Canned works as it is; changing it makes it right for every song.');
  await expect(dialog).toContainText('“⚠ no peer answered” on .pbar[data-status="failed"] .pbar__seek::before, for the failed state.');
  await dialog.screenshot({ path: info.outputPath('import-words.png') });
  await dialog.getByRole('button', { name: 'Use lull’s own words' }).click();
  await expect(page.locator('.toast').filter({ hasText: 'Canned shows lull’s own words now' })).toBeVisible();
  expect(stored.css).toBe(CANNED.replace('"\\26A0  no peer answered"', '"\\26A0  " var(--status-text)'));
});
