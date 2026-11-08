// themes-check.mjs's command line, read apart from the run so a test can pin it (tests/web/themes-check-args.test.mjs).
// Node's own modules only: tests/web runs without tests/ui's packages.

import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export class UsageError extends Error {}

export const USAGE = `usage: mise run themes:check -- [--only <id>[,<id>...] | --mine] [--sizes WxH[,WxH...]] [--mockup <file-or-url>] [--out <dir>] <route> [<route>...]
  a route: home, search, playlist, album, now, settings, mini (the desktop app's mini player), or a path (/settings/appearance)
  --only    these looks only: built-in designs or themes in LULL_THEMES_DIR, by id
  --mine    every theme in LULL_THEMES_DIR, and none of the built-ins
  --sizes   the desktop at each of these sizes (--size: one), instead of 1440x900
  --mockup  a mockup page drawn beside each picture at its size; {mode}, {route} and {size} in it are filled in
  --out     the folder the contact sheet and pictures go in, from the current folder (default tests/ui/themes-check)`;

// { routes, only, mine, sizes, mockup, out }: routes as given, only a list of ids or null, sizes a list of { width, height }
// or null (the default desktop), out the folder as given or null (outDir resolves it). Throws UsageError on anything it
// doesn't know.
export function parseArgs(argv) {
  const o = { routes: [], only: null, mine: false, sizes: null, mockup: null, out: null };
  for (let i = 0; i < argv.length; i++) {
    let flag = argv[i], given;
    const eq = flag.startsWith('--') ? flag.indexOf('=') : -1;
    if (eq > 0) { given = flag.slice(eq + 1); flag = flag.slice(0, eq); }
    const value = () => {
      if (given !== undefined) return given;
      const next = argv[++i];
      if (next === undefined || next.startsWith('--')) throw new UsageError(`${flag} needs a value`);
      return next;
    };
    switch (flag) {
      case '--only': o.only = [...(o.only || []), ...ids(value())]; break;
      case '--mine':
        if (given !== undefined) throw new UsageError('--mine takes no value');
        o.mine = true; break;
      case '--size': case '--sizes': o.sizes = [...(o.sizes || []), ...value().split(',').map(size)]; break;
      case '--mockup': o.mockup = value(); break;
      case '--out':
        o.out = value();
        if (!o.out.trim()) throw new UsageError('--out needs a folder');
        break;
      default:
        if (flag.startsWith('-')) throw new UsageError(`unknown flag ${flag}`);
        o.routes.push(flag);
    }
  }
  if (o.only && o.mine) throw new UsageError('--only and --mine pick the looks two ways: give one');
  if (o.only && !o.only.length) throw new UsageError('--only needs at least one id');
  if (!o.routes.length) throw new UsageError('name at least one route');
  if (o.sizes) o.sizes = o.sizes.filter((s, i) => o.sizes.findIndex((t) => t.width === s.width && t.height === s.height) === i);
  return o;
}

const ids = (list) => list.split(',').map((s) => s.trim()).filter(Boolean);

function size(s) {
  const m = /^(\d+)x(\d+)$/.exec(s.trim());
  if (!m || +m[1] < 1 || +m[2] < 1) throw new UsageError(`a size is WIDTHxHEIGHT, as 1536x864: ${JSON.stringify(s)}`);
  return { width: +m[1], height: +m[2] };
}

// The mockup's address for one picture: {mode} (day, night), {route} (the route's name: home, mini, settings-appearance)
// and {size} (desktop, phone, device, strip) filled in. A local path, with or without a query (C:/x/a.html?mode=night),
// becomes a file:// URL with its query kept; a URL (http:, https:, file:) is used as it is. A scheme is two letters or
// more, so a drive letter (C:) is a path.
export function mockupUrl(template, { mode, route, size: sized }, cwd = process.cwd()) {
  const filled = template.replace(/\{(mode|route|size)\}/g, (_, k) => ({ mode, route, size: sized })[k]);
  if (/^[a-z][a-z0-9+.-]+:/i.test(filled)) return new URL(filled).href;
  const at = filled.search(/[?#]/), path = at < 0 ? filled : filled.slice(0, at);
  return pathToFileURL(resolve(cwd, path)).href + (at < 0 ? '' : filled.slice(at));
}

// Where the contact sheet and pictures go: --out's folder, from the current folder or absolute, else the default. Under
// `mise run` the task runs in the checkout's root, so the current folder is the one mise was run from.
export const outDir = (given, fallback, cwd = process.env.MISE_ORIGINAL_CWD || process.cwd()) => (given ? resolve(cwd, given) : fallback);
