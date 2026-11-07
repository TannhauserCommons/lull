// The page a lull link opens (docs/adr/0034, "The card"): lull's open page (site/open.html, the same page GitHub Pages
// serves for older links) with the tags chat apps build a card from, and the item handed to site/open.js.
// Made from the address alone: no outside call, so the card's words never depend on anyone else being up.
// Discord shows lull's own card instead (card.js), from the address this page links it at.

import { ACCENT, alt, discordCard } from './card.js';
import { cardUrl, coverUrl, pageUrl, publicPage } from './link.js';

const TYPES = { song: 'music.song', album: 'music.album', artist: 'profile' };

/**
 * The page for `item` at `origin` (the canonical host's), from the open page's HTML. `here` is the origin it was asked
 * at: Discord reads a card only from the page's own host.
 */
export function page(template, origin, item, here) {
  const { kind, names, ids } = item;
  const title = kind === 'artist' ? names.artist : names.title;
  const description = kind === 'song' ? [names.artist, ids.al].filter(Boolean).join(' · ')
    : kind === 'album' ? 'Album by ' + names.artist
    : 'Artist';
  const url = pageUrl(origin, item);
  const cover = coverUrl(origin, item);
  const tags = [
    `<title>${esc(kind === 'artist' ? title : `${title} · ${names.artist}`)} · lull</title>`,
    meta('name', 'description', description),
    `<link rel="canonical" href="${esc(url)}">`,
    meta('property', 'og:site_name', 'lull'),
    meta('property', 'og:type', TYPES[kind]),
    meta('property', 'og:title', title),
    meta('property', 'og:description', description),
    meta('property', 'og:url', url),
    meta('property', 'og:image', cover),
    meta('property', 'og:image:alt', alt(item)),
    meta('name', 'twitter:card', 'summary_large_image'),
    meta('name', 'twitter:title', title),
    meta('name', 'twitter:description', description),
    meta('name', 'twitter:image', cover),
    meta('name', 'theme-color', ACCENT),
    ...(ids.len ? [meta('property', 'music:duration', ids.len)] : []),
    // Only when the card fits Discord's limits whatever its colour (white is the longest).
    ...(discordCard(origin, item, '#ffffff')
      ? [`<link rel="discord:component-embed" type="application/json" href="${esc(cardUrl(here, item))}">`]
      : []),
  ];
  const handed = {
    kind,
    artist: names.artist,
    title: kind === 'artist' ? '' : names.title,
    album: ids.al || '',
    page: publicPage(item),
    cover: new URL(cover).pathname + new URL(cover).search,
  };
  return fill(template, tags, handed);
}

/** The page for an address that names nothing: the open page says so, and the card is lull's. */
export function nothing(template, origin) {
  return fill(template, [
    '<title>lull</title>',
    meta('property', 'og:site_name', 'lull'),
    meta('property', 'og:title', 'This link names nothing to open'),
    meta('property', 'og:image', origin + '/lull-card.png'),
    meta('name', 'theme-color', ACCENT),
  ], null);
}

/** Over the per-client limit: a page in lull's style that says what happened and what to do. */
export function tooMany() {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<meta name="color-scheme" content="light dark">
<title>Too many at once · lull</title>
<link rel="stylesheet" href="/look.css">
<link rel="stylesheet" href="/open.css">
</head>
<body>
<main class="card">
  <img class="icon" src="/app-icon.svg" alt="" width="64" height="64">
  <h1>Too many links at once</h1>
  <p class="by">lull's link pages answer each address 60 times a minute, so that nobody can use them up for everyone. Wait a minute, then open the link again.</p>
</main>
</body>
</html>
`;
}

/** The open page with these tags in place of its title, its files at this host's root, and `item` for open.js. */
function fill(template, tags, item) {
  let html = template
    .replace(/<title>[^<]*<\/title>/, tags.join('\n'))
    // GitHub Pages serves the page beside its files; here a page's address is deeper, so they are named from the root.
    .replace(/\b(href|src)="(?![a-z]+:|\/|#)([^"]+)"/g, '$1="/$2"');
  if (item) html = html.replace('</head>', `<script type="application/json" id="lull-item">${json(item)}</script>\n</head>`);
  return html;
}

function meta(attr, key, value) {
  return `<meta ${attr}="${key}" content="${esc(value)}">`;
}

function esc(s) {
  return String(s).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** JSON inside a <script>: `<` escaped, so no name can close the element. */
function json(value) {
  return JSON.stringify(value).replace(/</g, '\\u003c');
}
