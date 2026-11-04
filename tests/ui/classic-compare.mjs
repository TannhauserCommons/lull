// Native Classic beside Webamp (docs/context/desktop.md, Native Classic): every skin in a folder drawn by Webamp (the
// vendored bundle, in Chromium, at 1x, its three windows stacked as the desktop app stacks them) and by native Classic
// (the window's own drawing, rendered headless by `cargo test render_skins -- --ignored` in app/), in the same scene: one
// song loaded and stopped, the equaliser on and flat, and per LULL_SCENE (both sides read it): `stopped` (the default,
// volume 75 %), `sliders` (volume 30 %, balance 60 % left), `shaded` (those, the three windows rolled up) or `unicode`
// (a song named in Cyrillic, Japanese, an accent, a curly quote and an emoji, short enough not to scroll) or `menus`
// (the playlist's REM menu open, the pointer on its second button), `resized` (the playlist 3 steps wider, 2 taller) or
// `resized-even` (4 wider, 1 taller). Writes, per skin, Webamp's picture, a side-by-side (Webamp, native, their difference in red) and a line of
// report.json: how many pixels differ in each window.
//   LULL_SCENE=<scene> node tests/ui/classic-compare.mjs <skins dir> <native renders dir> <out dir> [name filter]
import { chromium } from '@playwright/test';
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const [skins, natives, out, only] = process.argv.slice(2);
if (!out) { console.error('usage: node tests/ui/classic-compare.mjs <skins dir> <native renders dir> <out dir> [filter]'); process.exit(2); }
mkdirSync(out, { recursive: true });
const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const bundle = readFileSync(join(root, 'web/vendor/webamp/webamp.bundle.min.js'));

/* the scenes native Classic's render_skins draws (app/src/desktop/classic_native/window.rs) */
const SCENES = {
  stopped: { volume: 75, balance: 0, shaded: false },
  sliders: { volume: 30, balance: -60, shaded: false },
  shaded: { volume: 30, balance: -60, shaded: true },
  menus: { volume: 75, balance: 0, shaded: false, menu: true },
  resized: { volume: 75, balance: 0, shaded: false, playlist: [3, 2] },
  'resized-even': { volume: 75, balance: 0, shaded: false, playlist: [4, 1] },
  unicode: { volume: 75, balance: 0, shaded: false, artist: '\u041a\u043e\u043c\u0435\u0442\u0430', title: '\u9759\u304b \u2019\u00e9 \u{1f3b5}', still: true },
};
const SCENE = { artist: 'Velvet Comet', title: 'Static Season', seconds: 280, ...SCENES[process.env.LULL_SCENE || 'stopped'] };
if (!SCENE.volume) { console.error('LULL_SCENE is one of', Object.keys(SCENES).join(', ')); process.exit(2); }
/* the windows stacked, each [name, top, bottom] */
const WINDOWS = [];
const [WIDE, TALL] = SCENE.playlist || [0, 0], WIDTH = 275 + 25 * WIDE;
for (const name of ['main', 'equalizer', 'playlist']) {
  const height = SCENE.shaded ? 14 : name === 'playlist' ? 116 + 29 * TALL : 116;
  const top = WINDOWS.length ? WINDOWS[WINDOWS.length - 1][2] : 0;
  WINDOWS.push([name, top, top + height]);
}
const HEIGHT = WINDOWS[2][2];
/* left out of the count: a marquee that scrolls, on its own clock (Webamp's had moved by the time of the picture) */
const MASKS = SCENE.shaded || SCENE.still ? [] : [[111, 24, 154, 12]];

/* the song's file, never played: silence as long as the scene's song (Webamp takes its length from the file) */
function silence() {
  const rate = 1000, n = rate * SCENE.seconds, b = Buffer.alloc(44 + n * 2);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
  b.write('data', 36); b.writeUInt32LE(n * 2, 40);
  return b;
}

const page_ = `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:transparent;overflow:hidden}
#stage{position:absolute;left:0;top:0;width:${WIDTH}px;height:${HEIGHT}px}</style><div id="stage"></div><script src="/webamp.js"></script>`;

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT }, deviceScaleFactor: 1 });
await context.route('http://compare.local/**', (route) => {
  const path = decodeURIComponent(new URL(route.request().url()).pathname);
  if (path === '/') return route.fulfill({ contentType: 'text/html', body: page_ });
  if (path === '/webamp.js') return route.fulfill({ contentType: 'text/javascript', body: bundle });
  if (path === '/song.wav') return route.fulfill({ contentType: 'audio/wav', body: silence() });
  if (path.startsWith('/skins/')) return route.fulfill({ contentType: 'application/zip', body: readFileSync(join(skins, path.slice(7))) });
  if (path.startsWith('/native/')) return route.fulfill({ contentType: 'image/png', body: readFileSync(join(natives, path.slice(8))) });
  return route.fulfill({ status: 404, body: '' });
});

const report = [];
for (const file of readdirSync(skins).filter((f) => f.endsWith('.wsz') && (!only || f.includes(only))).sort()) {
  const name = file.slice(0, -4), page = await context.newPage();
  try {
    await page.goto('http://compare.local/');
    await page.evaluate(async ({ file, SCENE, WINDOWS, WIDE, TALL }) => {
      const webamp = new window.Webamp({
        initialSkin: { url: '/skins/' + file },
        initialTracks: [{ url: '/song.wav', metaData: { artist: SCENE.artist, title: SCENE.title }, duration: SCENE.seconds }],
        windowLayout: {
          main: { position: { top: WINDOWS[0][1], left: 0 }, shadeMode: SCENE.shaded },
          equalizer: { position: { top: WINDOWS[1][1], left: 0 }, shadeMode: SCENE.shaded },
          playlist: { position: { top: WINDOWS[2][1], left: 0 }, size: { extraWidth: WIDE, extraHeight: TALL }, shadeMode: SCENE.shaded },
        },
        enableHotkeys: false,
      });
      await webamp.renderInto(document.getElementById('stage'));
      if (webamp.skinIsLoaded) await webamp.skinIsLoaded();
      webamp.store.dispatch({ type: 'SET_VOLUME', volume: SCENE.volume });
      webamp.store.dispatch({ type: 'SET_BALANCE', balance: SCENE.balance });
    }, { file, SCENE, WINDOWS, WIDE, TALL });
    await page.waitForTimeout(600);                                   // the skin's CSS and sprites applied
    if (SCENE.menu) {                                                 // REM's button, then its second button
      const top = WINDOWS[2][1] + 86;
      /* the first click only selects the playlist: Webamp's window remounts on it, losing the click */
      await page.mouse.click(43 + 11, top + 9);
      await page.waitForTimeout(100);
      await page.mouse.click(43 + 11, top + 9);
      await page.mouse.move(43 + 11, top - 2 * 18 + 9);
      await page.waitForTimeout(100);
    }
    const shot = await page.screenshot({ clip: { x: 0, y: 0, width: WIDTH, height: HEIGHT }, omitBackground: true });
    writeFileSync(join(out, name + '.webamp.png'), shot);
    const result = await page.evaluate(async ({ shot, native, WINDOWS, MASKS, WIDTH, HEIGHT }) => {
      const load = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = no; i.src = src; });
      const [a, b] = await Promise.all([load(shot), load(native)]);
      const pixels = (img) => { const c = new OffscreenCanvas(WIDTH, HEIGHT), g = c.getContext('2d'); g.drawImage(img, 0, 0); return g.getImageData(0, 0, WIDTH, HEIGHT).data; };
      const pa = pixels(a), pb = pixels(b);
      const sheet = new OffscreenCanvas(WIDTH * 3 + 8, HEIGHT), g = sheet.getContext('2d');
      g.drawImage(a, 0, 0); g.drawImage(b, WIDTH + 4, 0);
      const diff = g.createImageData(WIDTH, HEIGHT), counts = {};
      for (const [w] of WINDOWS) counts[w] = 0;
      for (let i = 0; i < WIDTH * HEIGHT; i++) {
        const x = i % WIDTH, y = Math.floor(i / WIDTH), k = i * 4;
        if (MASKS.some(([mx, my, mw, mh]) => x >= mx && x < mx + mw && y >= my && y < my + mh)) { diff.data.set([0, 0, 96, 255], k); continue; }
        const visibleA = pa[k + 3] > 127, visibleB = pb[k + 3] > 127;
        const same = visibleA === visibleB && (!visibleA ||
          Math.abs(pa[k] - pb[k]) + Math.abs(pa[k + 1] - pb[k + 1]) + Math.abs(pa[k + 2] - pb[k + 2]) <= 6);
        const grey = visibleA ? (pa[k] + pa[k + 1] + pa[k + 2]) / 6 : 0;
        diff.data.set(same ? [grey, grey, grey, 255] : [255, 0, 0, 255], k);
        if (!same) counts[WINDOWS.find(([, top, bottom]) => y >= top && y < bottom)[0]]++;
      }
      g.putImageData(diff, 2 * WIDTH + 8, 0);
      const blob = await sheet.convertToBlob({ type: 'image/png' });
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
      return { counts, sheet: btoa(s) };
    }, { shot: 'data:image/png;base64,' + shot.toString('base64'), native: '/native/' + name + '.png', WINDOWS, MASKS, WIDTH, HEIGHT });
    writeFileSync(join(out, name + '.side.png'), Buffer.from(result.sheet, 'base64'));
    report.push({ skin: name, differ: result.counts });
    console.log(name.padEnd(34), JSON.stringify(result.counts));
  } catch (e) {
    report.push({ skin: name, error: String(e.message || e) });
    console.log(name.padEnd(34), 'ERROR', e.message || e);
  }
  await page.close();
}
writeFileSync(join(out, 'report.json'), JSON.stringify(report, null, 1) + '\n');
await browser.close();
