// latest.json for a v* release (.github/workflows/app.yml; docs/context/api.md, App releases).
//   node scripts/updater-manifest.mjs entries <platform> <bundle dir> > <file>
//     a bundle job's entries: the signed installer (nsis/) or AppImage (appimage/) as <platform>, and
//     the .deb and .rpm as <platform>-deb and <platform>-rpm. <platform> is Tauri's, e.g. linux-aarch64.
//   node scripts/updater-manifest.mjs manifest <version> <download base url> <dir>
//     the release job: every job's entries (<dir>/updater-*.json, removed after) and the Android APK
//     into <dir>/latest.json. Fails unless every platform a release must have is there.
import { readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

/** What a release must hold: each desktop platform the app updates on, and the APK. */
const PLATFORMS = ['windows-x86_64', 'windows-aarch64'].concat(
  ...['linux-x86_64', 'linux-aarch64'].map((p) => [p, `${p}-deb`, `${p}-rpm`]),
);
/** The bundler's folders and the extension of the file each holds, with the platform key's suffix. */
const BUNDLES = [
  ['nsis', '-setup.exe', ''],
  ['appimage', '.AppImage', ''],
  ['deb', '.deb', '-deb'],
  ['rpm', '.rpm', '-rpm'],
];

const [mode, ...args] = process.argv.slice(2);
if (mode === 'entries') entries(...args);
else if (mode === 'manifest') manifest(...args);
else fail(`unknown mode ${mode}: entries or manifest`);

function entries(platform, dir) {
  if (!platform || !dir) fail('entries <platform> <bundle dir>');
  const out = {};
  for (const [folder, ext, suffix] of BUNDLES) {
    let names;
    try { names = readdirSync(join(dir, folder)); } catch { continue; }   // this job doesn't make that bundle
    const file = one(names.filter((n) => n.endsWith(ext)), join(dir, folder, `*${ext}`));
    out[platform + suffix] = { file, signature: readFileSync(join(dir, folder, `${file}.sig`), 'utf8').trim() };
  }
  if (!Object.keys(out).length) fail(`no bundles in ${dir}`);
  process.stdout.write(JSON.stringify(out));
}

function manifest(version, base, dir) {
  if (!version || !base || !dir) fail('manifest <version> <download base url> <dir>');
  const names = readdirSync(dir);
  const parts = names.filter((n) => /^updater-.*\.json$/.test(n));
  const platforms = {};
  for (const part of parts) {
    for (const [key, { file, signature }] of Object.entries(JSON.parse(readFileSync(join(dir, part), 'utf8')))) {
      if (platforms[key]) fail(`${key} comes from two builds`);
      platforms[key] = { url: `${base}/${file}`, signature };
    }
  }
  const missing = PLATFORMS.filter((p) => !platforms[p]);
  if (missing.length) fail(`the release has no update for ${missing.join(', ')}`);
  const apk = one(names.filter((n) => n.endsWith('-android-arm64.apk')), join(dir, '*-android-arm64.apk'));
  const latest = { version, pub_date: new Date().toISOString(), platforms, android: { url: `${base}/${apk}` } };
  writeFileSync(join(dir, 'latest.json'), JSON.stringify(latest, null, 2) + '\n');
  for (const part of parts) rmSync(join(dir, part));
}

function one(found, pattern) {
  if (found.length !== 1) fail(`expected one ${pattern}, found ${found.length ? found.join(', ') : 'none'}`);
  return found[0];
}

function fail(message) {
  console.error(`updater-manifest: ${message}`);
  process.exit(1);
}
