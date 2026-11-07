// lull's card in Discord (docs/adr/0034, "The Discord card"): a component embed, Discord's own layout for a site's
// link, which Discord fetches from the page's `discord:component-embed` link. Everything in it comes from the link,
// except the colour along its edge, which is the cover's when the cover's lookup has read it.
//   Song shared from lull                 ┌──────┐
//   Glory Box                             │cover │
//   Portishead                            └──────┘
//   Dummy · 5:01
//   [Open in lull ↗] [Every music app ↗]
// Spotify's card in Discord is Discord's own (with its player); a site's card has no player and no audio, so lull's
// says what was shared and opens it.

import { coverUrl, pageUrl, publicPage } from './link.js';

/** lull's primary colour (site/open.css --primary): the edge of a card whose cover's colour isn't known. */
export const ACCENT = '#3d3491';
/** Discord's limits: the whole payload, a link button's address, and any address (over it, no card at all). */
const MAX_BYTES = 3000;
const MAX_BUTTON_URL = 512;
const MAX_URL = 2048;
const NOUN = { song: 'Song', album: 'Album', artist: 'Artist' };

/**
 * The card for `item`, as Discord's JSON, with `colour` (`#rrggbb`) along its edge; null when it doesn't fit Discord's
 * limits (long names in many scripts), and the page's Open Graph card is shown instead.
 */
export function discordCard(origin, item, colour = ACCENT) {
  const { kind, names, ids } = item;
  const page = pageUrl(origin, item);
  const cover = coverUrl(origin, item);
  if (cover.length > MAX_URL) return null;
  const lines = [`-# ${NOUN[kind]} shared from lull`, '### ' + md(kind === 'artist' ? names.artist : names.title)];
  if (kind !== 'artist') lines.push(md(names.artist));
  if (kind === 'song') {
    const more = [ids.al && md(ids.al), ids.len && duration(+ids.len)].filter(Boolean);
    if (more.length) lines.push('-# ' + more.join(' · '));
  }
  const elsewhere = publicPage(item);
  const buttons = [
    page.length <= MAX_BUTTON_URL && button('Open in lull', page),
    button(/^https:\/\/(song|album)\.link\//.test(elsewhere) ? 'Every music app' : 'Last.fm', elsewhere),
  ].filter(Boolean);
  const card = JSON.stringify({
    component: {
      type: 17,
      accent_color: parseInt(colour.slice(1), 16),
      components: [
        {
          type: 9,
          components: [{ type: 10, content: lines.join('\n') }],
          accessory: { type: 11, media: { url: cover }, description: alt(item) },
        },
        { type: 1, components: buttons },
      ],
    },
  });
  return new TextEncoder().encode(card).length <= MAX_BYTES ? card : null;
}

function button(label, url) {
  return { type: 2, style: 5, label, url };
}

/** What the cover shows, for screen readers. */
export function alt({ kind, names, ids }) {
  return kind === 'artist' ? names.artist : `Cover of ${ids.al || names.title} by ${ids.aa || names.artist}`;
}

/**
 * A name as Discord's markdown prints it: what markdown reads as markup anywhere in a line (emphasis, spoilers,
 * links, mentions, emoji, timestamps) is escaped, and what it reads only at a line's start (a heading, a list).
 */
function md(name) {
  return name
    .replace(/[\\*_~`|<>[\]():@]/g, '\\$&')
    .replace(/^[#-]/, '\\$&')
    .replace(/^(\d+)\./, '$1\\.');
}

function duration(seconds) {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = String(seconds % 60).padStart(2, '0');
  return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}
