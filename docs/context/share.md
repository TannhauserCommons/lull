# Share links on Cloudflare (`share/`)

lull links that chat apps show as cards, served by a Cloudflare Worker on the free plan. Why this shape: [ADR-0034](../adr/0034-lull-links-are-cards-served-from-cloudflare.md). The hub writes and reads the links (`crates/discovery/src/share_host.rs`, discovery.md#share-links).

## Files

| Path | What it is |
| --- | --- |
| `share/hosts.json` | The one list of lull's own share hosts, and the canonical one. The hub compiles it in and writes links on the canonical host; the Worker names it in `og:url` when serving a listed host (else its own); `test/hosts.test.mjs` checks the app's manifest claims each host |
| `share/src/index.js` | Routes: the page, its `…/cover`, the per-client limit. Everything else is a static file |
| `share/src/link.js` | Reading and writing `/1/<kind>/<names>?<ids>`; the album or artist a cover is looked up for (`subjectKey`) |
| `share/src/page.js` | The page: `site/open.html` with the card's tags and the item for `site/open.js`; the 404 and 429 pages |
| `share/src/sources.js` | The cover chain: iTunes (the US store, then the opener's: `request.cf.country`), Deezer, the Cover Art Archive, Last.fm, iTunes by name; Deezer and Wikidata for artists |
| `share/src/objects.js` | Durable Objects: `Cover` (one per album or artist: one lookup at a time, keeps the address found) and `Gate` (each service's budget, worldwide) |
| `share/build.mjs` | Gathers the static files into `share/public/` from where they live (`site/`, `web/`, `packaging/android/assetlinks.json`, `share/static/`), and writes `look.css`: lull's base tokens from `web/tokens.css`, the night block as a `prefers-color-scheme` rule. The page (`site/open.css`) is drawn with those tokens, its card as the app's card surface (`--cream`, `--ink`, `--link-on-cream`, `--line`), the same for everyone. Run by wrangler before every build, and by `pages.yml` |
| `share/test/` | `share.test.mjs` runs wrangler's own bundle in Miniflare with every outside service faked (`harness.mjs`); `hosts.test.mjs` |
| `.github/workflows/share.yml` | `test`; `deploy` on a dispatch on main |

## The link

For every lull install, in any market (ADR-0034, "For every lull"): links hold public facts only, never the sender's store or hub; every lull reads a `/1/` link on any https host, so a share host someone runs from `share/` works too.

`https://<host>/1/song/<artist>/<title>?al=&aa=&len=&it=&ic=&dz=&mb=`, `/1/album/<artist>/<title>?it=&dz=&mb=`, `/1/artist/<name>?dz=&mb=`.
- **Names** are exact, each one segment encoded as `encodeURIComponent` does (the hub's `share_host::encode` matches it), so the hub's link is the Worker's canonical one.
- **Ids:** `it` the item's iTunes id (a song's track, an album's collection), `ic` a song's album's iTunes id, `dz` the album's (or artist's) Deezer id, `mb` the release group's (or artist's) MBID; `al`, `aa`, `len` a song's album, album artist and length. Malformed ids are dropped on both sides.
- **The public page** is derived: `song.link/i/<it>`, `album.link/i/<it>`, else Last.fm by name.

## The cover

`…/cover` always answers an image, `cache-control` a week for a picture, an hour for lull's card (`web/assets/fallback-art-512.png`), a minute when the lookup ran out of its 6 s.
- **Order:** the edge cache (works on a custom domain, does nothing on `*.workers.dev`), the instance's memory (10 minutes), then the `Cover` object for the album, which runs `sources.find` once for everyone waiting.
- **Kept:** the picture's address, 30 days; "none" a day; a lookup a source couldn't answer, a minute. A kept address that stops answering is reported back (`broken`) and the chain runs again.
- **Budgets** (`Gate`): iTunes 10 a minute, Deezer 10 a second, MusicBrainz and the archive 1 a second, Last.fm 2, Wikidata 1. A turn free within a second is waited for; otherwise the step is skipped. A 429 or 403 from an API pauses it 10 minutes.
- **Per client:** 60 requests a minute per location (`CLIENTS`, the rate-limiting binding). Over it a page gets a 429 page, a cover lull's card.

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
