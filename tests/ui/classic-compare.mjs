// Native Classic beside Webamp (docs/context/desktop.md, Native Classic): every skin in a folder drawn by Webamp (the
// vendored bundle, in Chromium, at 1x, its three windows stacked as the desktop app stacks them) and by native Classic
// (the window's own drawing, rendered headless by `cargo test render_skins -- --ignored` in app/), in the same scene: one
// song loaded and stopped, volume 75 %, the equaliser on and flat. Writes, per skin, Webamp's picture, a side-by-side
// (Webamp, native, their difference in red) and a line of report.json: how many pixels differ in each window.
//   node tests/ui/classic-compare.mjs <skins dir> <native renders dir> <out dir> [name filter]
import { chromium } from '@playwright/test';
import { readFileSync, readdirSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const [skins, natives, out, only] = process.argv.slice(2);
if (!out) { console.error('usage: node tests/ui/classic-compare.mjs <skins dir> <native renders dir> <out dir> [filter]'); process.exit(2); }
mkdirSync(out, { recursive: true });
const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const bundle = readFileSync(join(root, 'web/vendor/webamp/webamp.bundle.min.js'));

/* the scene native Classic's render_skins draws (app/src/desktop/classic_native/window.rs) */
const SCENE = { artist: 'Velvet Comet', title: 'Static Season', seconds: 280, volume: 75 };
const WINDOWS = [['main', 0, 116], ['equalizer', 116, 232], ['playlist', 232, 348]];
/* left out of the count: the marquee scrolls on its own clock (Webamp's had moved by the time of the picture) */
const MASKS = [[111, 24, 154, 12]];

/* the song's file, never played: silence as long as the scene's song (Webamp takes its length from the file) */
function silence() {
  const rate = 1000, n = rate * SCENE.seconds, b = Buffer.alloc(44 + n * 2);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n * 2, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22); b.writeUInt32LE(rate, 24); b.writeUInt32LE(rate * 2, 28); b.writeUInt16LE(2, 32); b.writeUInt16LE(16, 34);
  b.write('data', 36); b.writeUInt32LE(n * 2, 40);
  return b;
}

const page_ = `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:transparent;overflow:hidden}
#stage{position:absolute;left:0;top:0;width:275px;height:348px}</style><div id="stage"></div><script src="/webamp.js"></script>`;

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: { width: 275, height: 348 }, deviceScaleFactor: 1 });
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
    await page.evaluate(async ({ file, SCENE }) => {
      const webamp = new window.Webamp({
        initialSkin: { url: '/skins/' + file },
        initialTracks: [{ url: '/song.wav', metaData: { artist: SCENE.artist, title: SCENE.title }, duration: SCENE.seconds }],
        windowLayout: {
          main: { position: { top: 0, left: 0 } },
          equalizer: { position: { top: 116, left: 0 } },
          playlist: { position: { top: 232, left: 0 }, size: { extraWidth: 0, extraHeight: 0 } },
        },
        enableHotkeys: false,
      });
      await webamp.renderInto(document.getElementById('stage'));
      if (webamp.skinIsLoaded) await webamp.skinIsLoaded();
      webamp.store.dispatch({ type: 'SET_VOLUME', volume: SCENE.volume });
    }, { file, SCENE });
    await page.waitForTimeout(600);                                   // the skin's CSS and sprites applied
    const shot = await page.screenshot({ clip: { x: 0, y: 0, width: 275, height: 348 }, omitBackground: true });
    writeFileSync(join(out, name + '.webamp.png'), shot);
    const result = await page.evaluate(async ({ shot, native, WINDOWS, MASKS }) => {
      const load = (src) => new Promise((ok, no) => { const i = new Image(); i.onload = () => ok(i); i.onerror = no; i.src = src; });
      const [a, b] = await Promise.all([load(shot), load(native)]);
      const pixels = (img) => { const c = new OffscreenCanvas(275, 348), g = c.getContext('2d'); g.drawImage(img, 0, 0); return g.getImageData(0, 0, 275, 348).data; };
      const pa = pixels(a), pb = pixels(b);
      const sheet = new OffscreenCanvas(275 * 3 + 8, 348), g = sheet.getContext('2d');
      g.drawImage(a, 0, 0); g.drawImage(b, 279, 0);
      const diff = g.createImageData(275, 348), counts = {};
      for (const [w] of WINDOWS) counts[w] = 0;
      for (let i = 0; i < 275 * 348; i++) {
        const x = i % 275, y = Math.floor(i / 275), k = i * 4;
        if (MASKS.some(([mx, my, mw, mh]) => x >= mx && x < mx + mw && y >= my && y < my + mh)) { diff.data.set([0, 0, 96, 255], k); continue; }
        const visibleA = pa[k + 3] > 127, visibleB = pb[k + 3] > 127;
        const same = visibleA === visibleB && (!visibleA ||
          Math.abs(pa[k] - pb[k]) + Math.abs(pa[k + 1] - pb[k + 1]) + Math.abs(pa[k + 2] - pb[k + 2]) <= 6);
        const grey = visibleA ? (pa[k] + pa[k + 1] + pa[k + 2]) / 6 : 0;
        diff.data.set(same ? [grey, grey, grey, 255] : [255, 0, 0, 255], k);
        if (!same) counts[WINDOWS.find(([, top, bottom]) => y >= top && y < bottom)[0]]++;
      }
      g.putImageData(diff, 558, 0);
      const blob = await sheet.convertToBlob({ type: 'image/png' });
      const bytes = new Uint8Array(await blob.arrayBuffer());
      let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
      return { counts, sheet: btoa(s) };
    }, { shot: 'data:image/png;base64,' + shot.toString('base64'), native: '/native/' + name + '.png', WINDOWS, MASKS });
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
