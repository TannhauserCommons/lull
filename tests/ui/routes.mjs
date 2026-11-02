// What the visual matrix covers. Fixed routes are listed here (web/screens/*.js registers them
// with App.route); detail routes (album, artist, tag, playlist) are added by global-setup from
// whatever the hub's library holds, so they appear as soon as a library source is active.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));

export const designs = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
export const themes = ['day', 'night'];
// Pixel sizes of the design package's two reference layouts.
export const viewports = { phone: { width: 390, height: 844 }, desktop: { width: 1440, height: 900 } };

const library = JSON.parse(readFileSync(join(here, 'fixtures/library.json'), 'utf8'));
const settingsSections = [
  'playback', 'sources', 'cache', 'library', 'sharing', 'lastfm', 'discovery', 'appearance', 'shortcuts', 'app',
];

export const fixedRoutes = [
  '/home',
  '/search',
  `/search?q=${encodeURIComponent(library.albums[0].artist)}`,
  '/library/albums',
  '/library/artists',
  '/library/songs',
  '/library/genres',
  '/liked',
  '/history',
  '/following',
  '/playlists',
  '/stats',
  '/now',
  '/classic',
  '/welcome',
  ...settingsSections.map((s) => `/settings/${s}`),
];

// The ids global-setup found (`.auth/ids.json`): { album, artist, tag, playlist }, each optional.
export function detailRoutes() {
  let ids = {};
  try {
    ids = JSON.parse(readFileSync(join(here, '.auth/ids.json'), 'utf8'));
  } catch {
    // global-setup has not run (listing tests without a hub): fixed routes only.
  }
  return ['album', 'artist', 'tag', 'playlist'].filter((k) => ids[k]).map((k) => `/${k}/${encodeURIComponent(ids[k])}`);
}

// File-name safe: /settings/cache -> settings-cache, /search?q=Night -> search-q-Night.
export function slug(route) {
  return route.replace(/^\//, '').replace(/[^A-Za-z0-9]+/g, '-').replace(/-$/, '') || 'root';
}
