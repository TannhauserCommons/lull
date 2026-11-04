// lull's share links (docs/adr/0034): a lull link is a page that chat apps show as a card, and that opens lull.
// - /1/<kind>/<names>?<ids>: the page, made from its address alone (page.js);
// - the same address + /cover: the cover, which always answers an image: the picture a source lull tracks has,
//   else lull's card (sources.js, objects.js);
// - everything else this Worker serves is a static file (public/, built by build.mjs): the open page's script and
//   style, the icon and font, lull's card, robots.txt, and Android's /.well-known/assetlinks.json.
// It logs nothing.

import hosts from '../hosts.json';
import { coverUrl, parse, subjectKey, subjectOf } from './link.js';
import { nothing, page, tooMany } from './page.js';
import { looksLikePicture, USER_AGENT } from './sources.js';

export { Cover, Gate } from './objects.js';

const PAGE_CACHE = 'public, max-age=86400';
const PICTURE_CACHE = 'public, max-age=604800';
/** lull's card stands in for a cover: kept an hour when no source has one, a minute when the lookup ran out of time. */
const CARD_CACHE = 'public, max-age=3600';
const CARD_BRIEFLY = 'public, max-age=60';
/** How long a cover request waits for a lookup: crawlers give up after a few seconds. */
const LOOKUP_MS = 6000;
const FETCH_MS = 3000;

export default {
  async fetch(request, env, ctx) {
    if (request.method !== 'GET' && request.method !== 'HEAD') {
      return new Response('Only GET and HEAD.\n', { status: 405, headers: { allow: 'GET, HEAD' } });
    }
    const url = new URL(request.url);
    const origin = hosts.canonical ? 'https://' + hosts.canonical : url.origin;
    const item = parse(url);
    if (item?.cover) return cover(request, env, ctx, item, origin);
    if (!(await allowed(request, env))) return html(tooMany(), 429, 'no-store', { 'retry-after': '60' });
    const template = await openPage(env, url);
    if (!item) return html(nothing(template, origin), 404, CARD_CACHE);
    return html(page(template, origin, item), 200, PAGE_CACHE);
  },
};

/** The per-client limit (60 a minute, per Cloudflare location), keyed by the client's address. */
async function allowed(request, env) {
  const { success } = await env.CLIENTS.limit({ key: request.headers.get('cf-connecting-ip') || 'local' });
  return success;
}

function html(body, status, cache, extra = {}) {
  return new Response(body, {
    status,
    headers: {
      'content-type': 'text/html; charset=utf-8',
      'cache-control': cache,
      'x-robots-tag': 'noindex',
      'referrer-policy': 'no-referrer',
      'x-content-type-options': 'nosniff',
      ...extra,
    },
  });
}

/** site/open.html, read once per instance from the Worker's files. */
let template = null;
async function openPage(env, url) {
  if (template) return template;
  const res = await env.ASSETS.fetch(new URL('/open.html', url));
  if (!res.ok) throw new Error(`the open page is missing from the Worker's files (${res.status})`);
  template = await res.text();
  return template;
}

// ---- The cover ----

/** What this instance already knows, by album or artist: `{ url, until }`, trusted at most this long. */
const known = new Map();
const KNOWN_MS = 10 * 60 * 1000;
const KNOWN_MAX = 1000;

/**
 * The cover for `item`: the edge cache (once a domain makes it work), this instance's memory, then the album's
 * Cover object, which runs the lookup once for everyone. A picture whose address stopped answering is reported
 * there, and looked for again. With no picture in time, lull's card.
 */
async function cover(request, env, ctx, item, origin) {
  const cacheKey = new Request(coverUrl(origin, item));
  const cached = await caches.default.match(cacheKey);
  if (cached) return cached;
  // Over the limit, a card rather than an error: a cover never breaks.
  if (!(await allowed(request, env))) return card(env, request, CARD_BRIEFLY);

  const subject = subjectOf(item);
  const key = subjectKey(subject);
  let found = remembered(key) || (await lookup(env, key, subject));
  if (found?.url) {
    let res = await fetchPicture(found.url);
    if (!res) {
      known.delete(key);
      found = await lookup(env, key, subject, found.url);
      res = found?.url ? await fetchPicture(found.url) : null;
    }
    if (res) {
      remember(key, found);
      ctx.waitUntil(caches.default.put(cacheKey, res.clone()));
      return res;
    }
  }
  if (!found) return card(env, request, CARD_BRIEFLY);
  remember(key, found);
  return card(env, request, CARD_CACHE);
}

/** The Cover object's answer, or null when it took longer than a crawler waits (it finishes for the next request)
    or failed: the caller then answers lull's card. */
async function lookup(env, key, subject, broken = null) {
  const stub = env.COVERS.get(env.COVERS.idFromName(key));
  let timer;
  const late = new Promise((resolve) => { timer = setTimeout(() => resolve(null), LOOKUP_MS); });
  try {
    return await Promise.race([stub.picture(subject, broken), late]);
  } catch {
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/** The picture's bytes, passed through, when its address still answers a picture (the lookup's own check); else null. */
async function fetchPicture(url) {
  let res;
  try {
    res = await fetch(url, { headers: { 'user-agent': USER_AGENT, accept: 'image/*' }, signal: AbortSignal.timeout(FETCH_MS) });
  } catch {
    return null;
  }
  if (!looksLikePicture(res)) {
    await res.body?.cancel();
    return null;
  }
  return new Response(res.body, {
    headers: { 'content-type': res.headers.get('content-type'), 'cache-control': PICTURE_CACHE, 'x-content-type-options': 'nosniff' },
  });
}

async function card(env, request, cache) {
  const res = await env.ASSETS.fetch(new URL('/lull-card.png', request.url));
  return new Response(res.body, {
    status: 200,
    headers: { 'content-type': 'image/png', 'cache-control': cache, 'x-content-type-options': 'nosniff' },
  });
}

function remembered(key) {
  const k = known.get(key);
  return k && Date.now() < Math.min(k.until, k.at + KNOWN_MS) ? k : null;
}

function remember(key, found) {
  known.delete(key);
  known.set(key, { url: found.url, until: found.until, at: Date.now() });
  if (known.size > KNOWN_MAX) known.delete(known.keys().next().value);
}
