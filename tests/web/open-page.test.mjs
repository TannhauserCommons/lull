// lull's open page (site/open.js, docs/adr/0032, 0034): the link it hands the lull app. On Android an intent URL, which Chrome
// opens on one tap (it opens no app without one), as the app's lull://open with its package, and this page again,
// marked ?app=none, when the app isn't installed. Elsewhere lull://open. Read back here as Chrome reads the intent
// and as MainActivity's SharedLinks reads the link (getQueryParameter("link")).
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

// the page's script as a browser runs it (a classic script), whatever package.json is above the checkout
const loaded = { exports: {} };
runInNewContext(readFileSync(new URL('../../site/open.js', import.meta.url), 'utf8'), { module: loaded, URL, URLSearchParams });
const O = loaded.exports;
const PAGE = 'https://tannhausercommons.github.io/lull/open';
const LINK = PAGE + '#lull?kind=song&artist=' + encodeURIComponent('Sigur Rós') + '&title=' + encodeURIComponent('Hoppípolla; #2 & more') +
  '&page=' + encodeURIComponent('https://song.link/i/1234');

/* Chrome's reading of intent://HOST/PATH?QUERY#Intent;scheme=S;package=P;S.key=value;end: the data URI is S://HOST/PATH?QUERY,
   and each extra's value is URL-decoded */
function chromeReads(url) {
  const m = /^intent:\/\/([^#]*)#Intent;(.*);end$/.exec(url);
  assert.ok(m, url);
  const fields = Object.fromEntries(m[2].split(';').map((f) => { const i = f.indexOf('='); return [f.slice(0, i), f.slice(i + 1)]; }));
  return { data: `${fields.scheme}://${m[1]}`, package: fields.package, fallback: decodeURIComponent(fields['S.browser_fallback_url']) };
}

test('on Android the app link is an intent for lull://open, the app\'s package, and this page to come back to', () => {
  const url = O.appUrl(LINK, true);
  assert.equal(url.split('#').length, 2, 'the link\'s own # stays inside its value');
  const intent = chromeReads(url);
  assert.equal(intent.package, 'io.github.tannhausercommons.lull');
  const data = new URL(intent.data);
  assert.equal(data.protocol, 'lull:');
  assert.equal(data.host, 'open', 'MainActivity takes scheme lull, host open');
  assert.equal(data.searchParams.get('link'), LINK, 'the link arrives whole');
  assert.equal(intent.fallback, PAGE + '?app=none#' + LINK.split('#')[1]);
  assert.equal(O.linkOf(intent.fallback), LINK, 'back on the page, the link it hands a lull has no ?app=none');
});

test('elsewhere the app link is lull://open with the link', () => {
  const url = O.appUrl(LINK, false);
  assert.ok(url.startsWith('lull://open?link='), url);
  assert.equal(new URL(url).searchParams.get('link'), LINK);
});

test('Android is told apart by its user agent', () => {
  assert.equal(O.isAndroid('Mozilla/5.0 (Linux; Android 14; Pixel 7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36'), true);
  assert.equal(O.isAndroid('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36'), false);
  assert.equal(O.isAndroid('Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1'), false);
  assert.equal(O.isAndroid(''), false);
});

test('a share host link keeps its ids: only the app=none it came back with is dropped', () => {
  const link = 'https://lull.share-host.invalid/1/song/Portishead/Glory%20Box?al=Dummy&it=1440823733';
  const intent = chromeReads(O.appUrl(link, true));
  assert.equal(new URL(intent.data).searchParams.get('link'), link, 'the link arrives whole');
  assert.equal(intent.fallback, link + '&app=none');
  assert.equal(O.linkOf(intent.fallback), link);
});
