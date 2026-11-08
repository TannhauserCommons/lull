// themes:check's command line (tests/ui/themes-check-args.mjs): which looks, sizes and mockup a run draws, and where. A flag read
// wrong draws the wrong pictures without saying so. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs, mockupUrl, outDir, UsageError } from '../ui/themes-check-args.mjs';

test('no flag keeps the plain run: every look, the default desktop', () => {
  assert.deepEqual(parseArgs(['home', '/settings/appearance']),
    { routes: ['home', '/settings/appearance'], only: null, mine: false, sizes: null, mockup: null, out: null });
});

test('--only takes a list, given once or several times, with or without =', () => {
  assert.deepEqual(parseArgs(['--only', 'lull,probe', 'home']).only, ['lull', 'probe']);
  assert.deepEqual(parseArgs(['--only=lull', '--only', 'petal', 'home']).only, ['lull', 'petal']);
  assert.equal(parseArgs(['--mine', 'mini']).mine, true);
  assert.throws(() => parseArgs(['--only', 'lull', '--mine', 'home']), UsageError);
  assert.throws(() => parseArgs(['--only', '--mine', 'home']), UsageError, 'a flag is not a value');
});

test('--sizes takes several desktop sizes, and --size still takes one', () => {
  assert.deepEqual(parseArgs(['--sizes', '1280x800,1440x900,1920x1080', 'home']).sizes,
    [{ width: 1280, height: 800 }, { width: 1440, height: 900 }, { width: 1920, height: 1080 }]);
  assert.deepEqual(parseArgs(['home', '--size', '1536x864']).sizes, [{ width: 1536, height: 864 }]);
  assert.deepEqual(parseArgs(['--sizes=1280x800,1280x800', 'home']).sizes, [{ width: 1280, height: 800 }]);
  assert.throws(() => parseArgs(['--sizes', '1280', 'home']), UsageError);
});

test('a run needs a route, and an unknown flag is refused', () => {
  assert.throws(() => parseArgs(['--mine']), UsageError);
  assert.throws(() => parseArgs(['--colour', 'home']), UsageError);
});

test('a mockup path with a query becomes a file URL with its placeholders filled in', () => {
  const at = { mode: 'night', route: 'home', size: 'phone' };
  const dir = join(tmpdir(), 'lull-mockups');   // need not exist: nothing is read
  assert.equal(mockupUrl('a window.html?mode={mode}&screen={route}&at={size}', at, dir),
    pathToFileURL(join(dir, 'a window.html')).href + '?mode=night&screen=home&at=phone');
  assert.equal(mockupUrl(join(dir, '{route}.html'), at), pathToFileURL(join(dir, 'home.html')).href);
  if (process.platform === 'win32') assert.equal(mockupUrl('C:/mock/a.html?mode={mode}', at), 'file:///C:/mock/a.html?mode=night', 'a drive letter is a path');
  assert.equal(mockupUrl('file:///C:/mock/a.html', at), 'file:///C:/mock/a.html', 'a plain file is shown as is');
  assert.equal(mockupUrl('http://localhost:8000/a.html?mode={mode}', at), 'http://localhost:8000/a.html?mode=night');
});

test('--out names the folder the sheet goes in, from the current folder or absolute; without it, the default', () => {
  const cwd = join(tmpdir(), 'lull-runs'), fallback = join(tmpdir(), 'default-sheet');   // need not exist: nothing is read
  assert.equal(parseArgs(['--out', 'run-a', 'home']).out, 'run-a');
  assert.equal(parseArgs(['home', '--out=../run b']).out, '../run b');
  assert.equal(outDir('run-a', fallback, cwd), join(cwd, 'run-a'));
  assert.equal(outDir('../run b', fallback, cwd), resolve(cwd, '..', 'run b'));
  assert.equal(outDir(join(tmpdir(), 'elsewhere'), fallback, cwd), join(tmpdir(), 'elsewhere'), 'an absolute folder is kept');
  assert.equal(outDir(parseArgs(['home']).out, fallback, cwd), fallback);
  assert.throws(() => parseArgs(['--out', 'home']), UsageError, 'the route was taken as the folder: no route left');
  assert.throws(() => parseArgs(['--out', '--mine', 'home']), UsageError, 'a flag is not a folder');
  assert.throws(() => parseArgs(['--out=', 'home']), UsageError);
});
