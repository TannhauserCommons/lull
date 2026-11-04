// The Worker's static files (docs/adr/0034), gathered into public/ from where each one lives, so nothing is copied
// into the repository twice: the open page and its script and style (site/, also GitHub Pages' copy for older
// links), lull's icon and font (web/), lull's card (the app's own fallback art), Android's App Links statement
// (packaging/android/) and robots.txt (static/). Run by wrangler before every dev, test and deploy build.
import { cpSync, mkdirSync, rmSync } from 'node:fs';

const root = new URL('../', import.meta.url);
const out = new URL('public/', import.meta.url);
const files = {
  'open.html': 'site/open.html',
  'open.css': 'site/open.css',
  'open.js': 'site/open.js',
  'app-icon.svg': 'web/assets/app-icon.svg',
  'nunito-latin.woff2': 'web/fonts/nunito-latin.woff2',
  'lull-card.png': 'web/assets/fallback-art-512.png',
  '.well-known/assetlinks.json': 'packaging/android/assetlinks.json',
  'robots.txt': 'share/static/robots.txt',
};

rmSync(out, { recursive: true, force: true });
mkdirSync(new URL('.well-known/', out), { recursive: true });
for (const [to, from] of Object.entries(files)) cpSync(new URL(from, root), new URL(to, out));
