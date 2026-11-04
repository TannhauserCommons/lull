// share/hosts.json is the one list of share hosts (docs/adr/0034): the hub compiles it in, the Worker names its
// canonical host, and the Android app must claim every host as an App Link, or a tap on a phone opens the browser.
// Run: npm test (in share/)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const hosts = JSON.parse(readFileSync(new URL('../hosts.json', import.meta.url), 'utf8'));
const manifest = readFileSync(new URL('../../app/gen/android/app/src/main/AndroidManifest.xml', import.meta.url), 'utf8');

test('the canonical share host is one of the hosts', () => {
  assert.ok(Array.isArray(hosts.hosts));
  assert.ok(hosts.canonical === null || hosts.hosts.includes(hosts.canonical), hosts.canonical);
});

test('the Android app claims every share host as a verified App Link for /1/', () => {
  const filters = [...manifest.matchAll(/<intent-filter android:autoVerify="true">([\s\S]*?)<\/intent-filter>/g)].map((m) => m[1]);
  for (const host of hosts.hosts) {
    const claimed = filters.some((f) =>
      f.includes('android.intent.action.VIEW') && f.includes('android.intent.category.BROWSABLE') &&
      new RegExp(`<data android:scheme="https" android:host="${host.replace(/\./g, '\\.')}" android:pathPrefix="/1/" />`).test(f));
    assert.ok(claimed, `${host} has no autoVerify filter for https://${host}/1/ in the app's manifest`);
  }
});
