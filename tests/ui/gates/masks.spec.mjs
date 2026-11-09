// Masks draw as they should in every engine (docs/themes.md, Drawing cheaply). WebKit (the Linux app, Safari) paints a
// mask on an inline box inside out: the whole box in its background, with the mask's shape cut from it. Petal's title
// heart was one (a pixel-heart mask on the title's inline ::after), a pink block in the Linux app.
// - In every design and mode, on the key screens, no element or pseudo-element with a mask is displayed inline (the
//   computed style, so it holds in any engine, a theme's own rules included).
// - In WebKit, Petal's title heart is a heart: little of the title's box is the heart's colour.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
const SCREENS = ['/home', '/library/albums', `/playlist/${ids.longPlaylist}`, '/now', '/settings/app'];

for (const design of DESIGNS) {
  test(`${design}: no mask sits on an inline box`, async ({ page, browserName }) => {
    test.skip(browserName !== 'chromium', 'computed style: one engine is enough');
    const found = [];
    for (const theme of ['day', 'night']) {
      for (const route of SCREENS) {
        await openScreen(page, route, { design, theme });
        const hits = await page.evaluate(() => {
          const out = [];
          for (const el of document.querySelectorAll('body *')) {
            for (const pseudo of [null, '::before', '::after']) {
              const cs = getComputedStyle(el, pseudo);
              if (pseudo && (cs.content === 'none' || cs.content === 'normal')) continue;
              const mask = cs.maskImage || cs.webkitMaskImage;
              if (mask && mask !== 'none' && cs.display === 'inline') out.push(`${el.tagName.toLowerCase()}.${String(el.className).split(' ').join('.')}${pseudo || ''}`);
            }
          }
          return out;
        });
        for (const h of hits) found.push(`${route} ${theme}: ${h}`);
      }
    }
    expect([...new Set(found)], 'masked inline boxes, which WebKit paints inside out').toEqual([]);
  });
}

test('Petal\'s title heart is a heart, not a block with a heart cut out', async ({ page }) => {
  await openScreen(page, '/settings/app', { design: 'petal', theme: 'night' });
  const title = page.locator('h1.sys-settings__title');
  const png = await title.screenshot();
  const share = await page.evaluate(async (data) => {
    const img = new Image(); img.src = 'data:image/png;base64,' + data; await img.decode();
    const c = document.createElement('canvas'); c.width = img.width; c.height = img.height;
    const g = c.getContext('2d'); g.drawImage(img, 0, 0);
    const probe = document.createElement('i'); probe.style.color = getComputedStyle(document.documentElement).getPropertyValue('--heart');
    document.body.appendChild(probe); const want = getComputedStyle(probe).color.match(/\d+/g).map(Number); probe.remove();
    const px = g.getImageData(0, 0, c.width, c.height).data; let hits = 0;
    for (let i = 0; i < px.length; i += 4) if (Math.abs(px[i] - want[0]) + Math.abs(px[i + 1] - want[1]) + Math.abs(px[i + 2] - want[2]) < 24) hits++;
    return hits / (px.length / 4);
  }, png.toString('base64'));
  // a 9x8 pixel heart is about 1-2% of the title's box; the inside-out block was 15% and more
  expect(share, 'the heart\'s colour covers a heart, not a block').toBeGreaterThan(0.003);
  expect(share).toBeLessThan(0.06);
});

// The contrast check judges text over a masked shape (fog, a feathered patch) against the shape as drawn, and leaves
// out only the pixels a mask fades of text it is laid on itself (a list dissolving at its scroller's edge).
test('the contrast check judges text over a masked shape as drawn, and lets a masked text fade', async ({ page }) => {
  const { contrast } = await import('../checks.mjs');
  const fog = 'radial-gradient(closest-side, rgba(0,0,0,.9), rgba(0,0,0,.6))';
  await page.setContent(`<body style="margin:0;background:#000">
    <div style="position:relative;width:400px;height:120px">
      <div style="position:absolute;inset:0;background:#e8e8e8;-webkit-mask:${fog};mask:${fog}"></div>
      <p style="position:relative;margin:0;padding:40px;font:16px sans-serif;color:#d0d0d0">low on the fog</p>
    </div></body>`);
  expect(await contrast(page), 'pale text on pale fog').toMatch(/where 4\.5:1 is needed: p "low on the fog"/);
  await page.setContent(`<body style="margin:0;background:#000">
    <div style="width:400px;height:120px;-webkit-mask:linear-gradient(#000 40%, transparent);mask:linear-gradient(#000 40%, transparent)">
      <p style="margin:0;padding:10px 40px;font:16px sans-serif;color:#fff">a line that fades</p>
      <p style="margin:0;padding:10px 40px;font:16px sans-serif;color:#fff">and its next line</p>
    </div></body>`);
  expect(await contrast(page), 'white text a mask fades over black').toBeNull();
});

// The contrast check counts a text's own shadows as drawn: white words on a pale sky read when a dense, close shadow
// backs them, and still fall short with a faint one.
test('the contrast check counts a text\'s shadow as what readers see behind it', async ({ page }) => {
  const { contrast } = await import('../checks.mjs');
  const sky = (shadow) => `<body style="margin:0;background:#cfe3f2"><p style="margin:0;padding:40px;font:600 18px sans-serif;color:#fff;text-shadow:${shadow}">white on the day sky</p></body>`;
  await page.setContent(sky('none'));
  expect(await contrast(page), 'no shadow').toMatch(/white on the day sky/);
  await page.setContent(sky('0 0 1px #0b1828, 0 0 2px #0b1828, 0 0 3px #0b1828, 0 0 6px #0b1828e0'));
  expect(await contrast(page), 'a dense shadow close to the letters').toBeNull();
  await page.setContent(sky('0 2px 12px #0b182822'));
  expect(await contrast(page), 'a faint shadow').toMatch(/white on the day sky/);
});
