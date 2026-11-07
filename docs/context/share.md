# Share links on Cloudflare (`share/`)

lull links that chat apps show as cards, served by a Cloudflare Worker on the free plan. Why this shape: [ADR-0034](../adr/0034-lull-links-are-cards-served-from-cloudflare.md). The hub writes and reads the links (`crates/discovery/src/share_host.rs`, discovery.md#share-links).

## Files

| Path | What it is |
| --- | --- |
| `share/hosts.json` | The one list of lull's own share hosts, and the canonical one. The hub compiles it in and writes links on the canonical host; the Worker names it in `og:url` when serving a listed host (else its own); `test/hosts.test.mjs` checks the app's manifest claims each host |
| `share/src/index.js` | Routes: the page, its `…/cover` and `…/card`, the per-client limit. Everything else is a static file |
| `share/src/link.js` | Reading and writing `/1/<kind>/<names>?<ids>`; the album or artist a cover is looked up for (`subjectKey`) |
| `share/src/page.js` | The page: `site/open.html` with the card's tags (and the link to Discord's card) and the item for `site/open.js`; the 404 and 429 pages |
| `share/src/card.js` | lull's card in Discord: a component embed, as JSON |
| `share/src/colour.js` | A cover's colour, for the card's edge: read from a small JPEG's block averages, no full decode |
| `share/src/sources.js` | The cover chain: iTunes (the US store, then the opener's: `request.cf.country`), Deezer, the Cover Art Archive, Last.fm, iTunes by name; Deezer and Wikidata for artists |
| `share/src/objects.js` | Durable Objects: `Cover` (one per album or artist: one lookup at a time, keeps the address found and its colour) and `Gate` (each service's budget, worldwide) |
| `share/build.mjs` | Gathers the static files into `share/public/` from where they live (`site/`, `web/`, `packaging/android/assetlinks.json`, `share/static/`), and writes `look.css`: lull's base tokens from `web/tokens.css`, the night block as a `prefers-color-scheme` rule. The page (`site/open.css`) is drawn with those tokens, its card as the app's card surface (`--cream`, `--ink`, `--link-on-cream`, `--line`), the same for everyone. Run by wrangler before every build, and by `pages.yml` |
| `share/test/` | `share.test.mjs` runs wrangler's own bundle in Miniflare with every outside service faked (`harness.mjs`); `hosts.test.mjs`; `colour.test.mjs` with tiny synthetic JPEGs (`pictures.mjs`) |
| `.github/workflows/share.yml` | `test`; `deploy` on a dispatch on main |

## The link

For every lull install, in any market (ADR-0034, "For every lull"): links hold public facts only, never the sender's store or hub; every lull reads a `/1/` link on any https host, so a share host someone runs from `share/` works too.

`https://<host>/1/song/<artist>/<title>?al=&aa=&len=&it=&ic=&dz=&mb=`, `/1/album/<artist>/<title>?it=&dz=&mb=`, `/1/artist/<name>?dz=&it=&mb=&al=&ds=`.
- **Names** are exact, each one segment encoded as `encodeURIComponent` does (the hub's `share_host::encode` matches it), so the hub's link is the Worker's canonical one.
- **Ids:** `it` the item's iTunes id (a song's track, an album's collection), `ic` a song's album's iTunes id, `dz` the album's (or artist's) Deezer id, `mb` the release group's (or artist's) MBID; `al`, `aa`, `len` a song's album, album artist and length. Malformed ids are dropped on both sides.
- **An artist link** (ADR-0035, with the `namesakes` switch on the sending hub): `dz`, `it` and `mb` every id it holds for the artist, `al` one album of theirs (so an artist with no id is still opened by its work), `ds` what tells it apart from others of its name (MusicBrainz's comment), for the card and the page. A hub opens it by the rule: the artist holding those anchors, else every artist of the name. The Worker reads `ds` to show it under the name (share session).
- **The public page** is derived: `song.link/i/<it>`, `album.link/i/<it>`, else Last.fm by name.

## The cover

`…/cover` always answers an image, `cache-control` a week for a picture, an hour for lull's card (`web/assets/fallback-art-512.png`), a minute when the lookup ran out of its 6 s.
- **Order:** the edge cache (works on a custom domain, does nothing on `*.workers.dev`), the instance's memory (10 minutes), then the `Cover` object for the album, which runs `sources.find` once for everyone waiting.
- **Kept:** the picture's address, 30 days; "none" a day; a lookup a source couldn't answer, a minute. A kept address that stops answering is reported back (`broken`) and the chain runs again.
- **Budgets** (`Gate`): iTunes 10 a minute, Deezer 10 a second, MusicBrainz and the archive 1 a second, Last.fm 2, Wikidata 1. A turn free within a second is waited for; otherwise the step is skipped. A 429 or 403 from an API pauses it 10 minutes.
- **Its colour:** once a picture is found, the `Cover` object reads its small copy (128 px, `sources.smallCopy`) and keeps the colour (`#rrggbb`) beside the address; null for a picture that isn't a JPEG. Rows kept before colours get theirs on their next request.
- **Per client:** 60 requests a minute per location (`CLIENTS`, the rate-limiting binding). Over it a page gets a 429 page, a cover lull's card.

## Discord's card

Discord shows lull's own card for a link, a [component embed](https://docs.discord.com/developers/link-previews/component-embeds): the page links `…/card` (`<link rel="discord:component-embed">`, on the host the page was asked at, as Discord requires), and everyone else reads the Open Graph tags.
- **What it holds:** "Song shared from lull", the title, the artist, the album and length; the cover beside them (`…/cover`); buttons "Open in lull" (the page) and "Every music app" (song.link or album.link) or "Last.fm". Names are escaped for Discord's markdown.
- **Its edge** is the cover's colour (above), waited for 2.5 s, which also starts the cover's lookup; else lull's `--primary`. Cached a day once known, a minute while it's looked for.
- **Limits:** 3,000 bytes, link buttons' addresses 512 characters, any address 2,048. A card that can't fit (very long names in scripts that percent-encode long) isn't linked, and Discord shows the Open Graph card.
- **Not possible for a site:** a player, a preview or any audio, a card filled with a colour, a logo beside the title. Spotify's card is Discord's own.
- **Check it:** Discord's [Embed Debugger](https://discord.com/developers/embeds) (signed in), with a deployed link. Discord keeps a link's preview about 30 minutes; a new query (`?v=2`) is a new link to it.

## Run it

- **Tests:** `npm ci` then `npm test` in `share/` (also runs `tests/web/open-page.test.mjs`, since the Worker serves that script). CI runs them on Linux (`share.yml`, and the queue whenever `share/`, `site/` or a file the Worker serves changes).
- **On Windows,** Miniflare's local rate limiter fails with "internal error" (Miniflare 5 beta, 2026-11): run the tests in Linux instead, from the repository root:

  ```bash
  docker run --rm -v "$PWD:/repo" -v /repo/share/node_modules -v /repo/share/.wrangler -w /repo/share node:24.11-bookworm sh -c "npm ci && npm test"
  ```

  Miniflare also needs a writable temp folder; a locked-down one fails with `CreateDirectory: Access is denied`.
- **By hand:** `npx wrangler dev` (Linux, as above with `-p 127.0.0.1:8787:8787` and `--ip 0.0.0.0`) runs it against the real services; `http://127.0.0.1:8787/1/album/Massive%20Attack/Mezzanine` shows the page.

## Deploying, and a domain later

- **Secrets:** GitHub `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (Bitwarden `lull/CLOUDFLARE_API_TOKEN`, `lull/CLOUDFLARE_ACCOUNT_ID`, set by the integrator through keygate). The deploy job puts `LULL_LASTFM_API_KEY` as the Worker's `LASTFM_API_KEY`. Until the Cloudflare secrets exist, `deploy` says so and does nothing.
- **First host:** `listen.lullfm.workers.dev` (account subdomain `lullfm`, permanent) is listed and canonical in `hosts.json`, with its `autoVerify` filter in the app's manifest (`pathPrefix="/1/"`). The hub writes links on it; the app claims them from its next release. Its Worker serves `/.well-known/assetlinks.json`, copied from `packaging/android/assetlinks.json`.
- **A domain:** attach it to the Worker, add it to `hosts` and the manifest, then make it `canonical`. Never remove a host or rename the Worker or the account subdomain: every link copied names them.
- **Logs:** none. `observability` is off and wrangler sends no metrics (`wrangler.jsonc`).
