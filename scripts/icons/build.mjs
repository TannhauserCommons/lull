// Renders every packaged app icon from the three masters (design/desktop/DESKTOP.md section 8):
//   design/assets/app-icon.svg              48, 64, 128, 192, 256 and 512 px
//   design/assets/icons/app-icon-small.svg  16, 24 and 32 px, each from its own hand-tuned <svg>
//   design/assets/app-icon-maskable.svg     icon-maskable-512.png
// plus app.ico (16, 24, 32, 48, 64, 256) and the fastlane store icon, then copies the masters and the
// results into web/assets, which the hub and the apps ship. Usage: node scripts/icons/build.mjs [repo root]
import { Resvg } from '@resvg/resvg-js';
import { readFileSync, writeFileSync, copyFileSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

const root = process.argv[2] || join(import.meta.dirname, '..', '..');
const design = join(root, 'design', 'assets'), out = join(design, 'icons'), web = join(root, 'web', 'assets');
const render = (svg, size) => new Resvg(svg, { fitTo: { mode: 'width', value: size }, font: { loadSystemFonts: false } }).render().asPng();

const master = readFileSync(join(design, 'app-icon.svg'), 'utf8');
const small = readFileSync(join(out, 'app-icon-small.svg'), 'utf8');
const png = {};
for (const block of small.match(/<svg xmlns[\s\S]*?<\/svg>/g)) {
  const size = Number(block.match(/width="(\d+)"/)[1]);
  png[size] = render(block, size);
}
for (const size of [48, 64, 128, 192, 256, 512]) png[size] = render(master, size);
for (const [size, data] of Object.entries(png)) writeFileSync(join(out, `icon-${size}.png`), data);
writeFileSync(join(out, 'icon-maskable-512.png'), render(readFileSync(join(design, 'app-icon-maskable.svg'), 'utf8'), 512));

// app.ico: PNG-compressed entries, which Windows reads at every size since Vista
const sizes = [16, 24, 32, 48, 64, 256], head = Buffer.alloc(6 + 16 * sizes.length);
head.writeUInt16LE(0, 0); head.writeUInt16LE(1, 2); head.writeUInt16LE(sizes.length, 4);
let offset = head.length;
sizes.forEach((size, i) => {
  const e = 6 + 16 * i, data = png[size];
  head.writeUInt8(size % 256, e); head.writeUInt8(size % 256, e + 1);      // 0 means 256
  head.writeUInt16LE(1, e + 4); head.writeUInt16LE(32, e + 6);
  head.writeUInt32LE(data.length, e + 8); head.writeUInt32LE(offset, e + 12);
  offset += data.length;
});
writeFileSync(join(out, 'app.ico'), Buffer.concat([head, ...sizes.map((s) => png[s])]));

// what ships: the web app's copy of the masters and sizes, and the F-Droid/fastlane store icon
const shipped = ['app.ico', 'app-icon-small.svg', 'icon-maskable-512.png', ...Object.keys(png).map((s) => `icon-${s}.png`)];
mkdirSync(join(web, 'icons'), { recursive: true });
for (const f of shipped) copyFileSync(join(out, f), join(web, 'icons', f));
for (const f of ['app-icon.svg', 'app-icon-maskable.svg']) copyFileSync(join(design, f), join(web, f));
copyFileSync(join(out, 'icon-512.png'), join(root, 'fastlane', 'metadata', 'android', 'en-US', 'images', 'icon.png'));
console.log(`icons: ${shipped.length} files in design/assets/icons and web/assets/icons, plus the fastlane icon`);
