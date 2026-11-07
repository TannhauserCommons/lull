# ADR-0034: lull links are pages on Cloudflare that chat apps show as cards

- Status: accepted
- Date: 2026-11-04 (the listener chose Cloudflare's free plan and asked that a link, and its cover, never break)
- Extends [ADR-0032](0032-lull-links-are-public-pages-with-ids.md) and replaces its link format for new links. Its open page stays for every link already copied.

## Context

The listener wants a lull link pasted into Discord, WhatsApp or Telegram to show a card: cover, title, artist, album. Tapped on Android, it should open the lull app at that song; anywhere else, a page saying what it is and how to open it. In their words, the cover must never break while any source lull tracks has it, and nothing else about a link may break either, however many people click it at once.

Today's link (ADR-0032) is `https://tannhausercommons.github.io/lull/open#lull?kind=song&artist=…`. Everything after `#` stays in the browser, so GitHub's server, and every chat app's crawler, sees only `/lull/open`. No card can be built from that: a card's title and picture come from tags in the page the server sends.

The listener chose Cloudflare's free plan to host the new page. A domain may come later, and must not break a link copied before it.

Measured or documented constraints (Cloudflare docs, 2026-11-04):
- **Workers Free:** 100,000 requests a day, 10 ms of CPU per request, 50 outside requests per request. Static assets served by a Worker are free and don't count.
- **The Cache API does nothing on `*.workers.dev`.** It works on a custom domain (and on `*.pages.dev`). Its contents stay in one data centre.
- **Durable Objects on Workers Free:** SQLite-backed only; 100,000 requests a day, 100,000 rows written a day, 5 GB in all.
- **The rate-limiting binding** counts per location, not worldwide.

## Decision

### For every lull, in every market

Share links serve every lull install, not one hub:
- **A link names only public catalogue facts**, so the Worker needs nothing from the hub that made it: any listener's hub writes links any page, card and lull can open.
- **Covers resolve wherever the link is opened.** A hub's iTunes ids come from its own store, which can be any country. The Worker looks an id up in the US store and then in the store of whoever opened the link (Cloudflare's country for the request), and searches names in that store. The sender's store is never put in the link: it would say where they are. Deezer ids, MBIDs and Last.fm names are the same everywhere.
- **Anyone may run a share host** from `share/` on their own Cloudflare account. Every lull reads `/1/` links by their shape on any https host, so those links open in every lull too. Only lull's own hosts get App Links and are named in `share/hosts.json`.

### One Worker, `share/`

- A Cloudflare Worker in the lull repository (`share/`), with static assets and two Durable Object classes, deployed by CI with `wrangler`.
- Its free address is `listen.<account subdomain>.workers.dev`. The account subdomain is set once when the Cloudflare account is made. It must say nothing about the owner (`tannhausercommons` does), and must never be renamed: renaming it breaks every link.
- The page itself needs no outside call. Only the cover does (below). So the title, artist and album in a card never depend on anyone else being up.

### The link

```
https://<host>/1/song/<artist>/<title>?al=<album>&aa=<album artist>&len=<s>&it=<iTunes track>&ic=<iTunes album>&dz=<Deezer album>&mb=<release group>
https://<host>/1/album/<artist>/<title>?it=<iTunes album>&dz=<Deezer album>&mb=<release group>
https://<host>/1/artist/<name>?dz=<Deezer artist>&mb=<MusicBrainz artist>
```

e.g. `https://listen.lullfm.workers.dev/1/song/Portishead/Glory%20Box?al=Dummy&it=1440823733&ic=1440823517`.

- **`1` is the version.** Unknown keys are ignored, so new ids can join version 1. `/2/` is only for a change that version 1 can't express, and the Worker answers every version it has ever handed out, forever.
- **Names are exact,** each one percent-encoded path segment, not slugs. lull's ids are hashes of names (ADR-0032), so the receiving lull needs the names exactly as sent. Browsers show them decoded.
- **Every key is optional except the names.** The hub writes what it knows, in this order, and leaves out the rest. Ids are checked for their form (iTunes and Deezer digits, MBIDs as UUIDs); a malformed one is dropped, not trusted. Names are capped at 200 characters. A path that names nothing gets a 404 page in lull's style, with the lull card.
- **The public page comes from the ids,** so it isn't carried: `song.link/i/<it>` for a song, `album.link/i/<it>` for an album, else Last.fm's page by name. Only those hosts are ever linked.
- **What a link never holds:** the hub's address, the listener's or sender's identity, the time, or anything about the owner. Only public music names and public catalogue ids.

### Privacy, compared with today's links

Today the item rides after `#`, which no server sees. That is exactly why no card can be built. In the new link it is in the path, so:
- **Cloudflare** sees each opened link's item and the opener's IP address, as any web host does. The Worker logs nothing (Workers logs and traces off, no analytics script, no cookies), but Cloudflare's own aggregate metrics exist.
- **Each chat app's crawler** reads the item to build the card. The chat app already holds the message, so it learns nothing new.
- **Search engines** are kept out: `X-Robots-Tag: noindex` and `<meta name="robots" content="noindex">`. `robots.txt` doesn't disallow anything, because some card crawlers obey it.
- **What doesn't change:** the link names a song, as before, and nothing in it leads to the hub, the listener or the owner. The Cloudflare account is the owner's, but nothing public shows that, provided the account subdomain is neutral.
- **Old links keep their promise:** the GitHub page isn't changed to forward its `#` part to Cloudflare.

### The card

The page's `<head>` comes first and the whole page is a few KB, because WhatsApp reads only the start of a page. It holds:
- `og:site_name` `lull`, `og:type` (`music.song`, `music.album`, `profile`), `og:title` (the title, or the artist's name), `og:description` (`Portishead · Dummy`, `Album by Portishead`, `Artist`), and `og:url` (the canonical link, below).
- `og:image`, the Worker's own cover address for the link (`…/cover`, same ids), with `og:image:width`/`height` 600 and `og:image:alt`.
- `twitter:card` `summary_large_image`, plus `twitter:title`, `twitter:description` and `twitter:image`. Discord and Telegram read Open Graph and use these as fallbacks.
- `theme-color` (lull's accent), which Discord uses for the card's edge. And `music:duration` for a song.

Every name is HTML-escaped. The page has the same strict CSP, `no-referrer` and lack of outside requests as ADR-0032's page.

### The cover never breaks

`GET …/cover` always answers with an image. Each step's answer is checked: status 200, an image content type, a non-empty body under 2 MB. A failure moves on to the next step.

A song's cover is its album's. Covers are keyed by the album (its best id, else its normalised album artist and title), so every song on an album shares one lookup.

**Albums and songs:**
1. iTunes `lookup` by `ic` (a song) or `it` (an album), in the US store, then in the opener's store. The store's cover at 600 px.
2. Deezer `album/<dz>`. Its cover at 600 px.
3. The Cover Art Archive, `release-group/<mb>/front-500`. Exact, but slow from some places, so it comes after the fast sources.
4. Last.fm `album.getInfo` by album artist and album. Uses a Worker secret, `LASTFM_API_KEY`.
5. iTunes search by names in the opener's store, taking only a listing whose artist and album title are the same as the link's once case, accents and punctuation are set aside. Last, because a name can find the wrong album. The hub's looser rule (`share::best`) isn't copied into the Worker: a cover that might be another album's is worse than lull's card.

**Artists:**
1. Deezer `artist/<dz>`.
2. Wikidata's picture (P18, through Commons) by `mb`.
3. Deezer artist search, used only when exactly one artist has that name.

**Nothing found:** lull's card, a static 600 px PNG (lull's icon on its own background) shipped with the Worker. Chat apps print the title and artist beside it anyway.

**Keeping what was found:** the Worker keeps the *address* of the picture found, never its bytes.
- Rechecked after 30 days. A "none" is asked again after a day, and a step that failed (timeout, 429, 5xx) after a minute, since that is often a moment's slowness. The gate keeps those retries polite.
- When the address it kept stops answering, that request runs the chain again instead of failing.
- Nothing is stored that could grow without bound: one small row per album or artist that someone opened.

**Time:** each step gets 3 s and the chain gets 6 s, because crawlers give up. Past that, the request gets lull's card with a one-minute cache, and the lookup finishes for the next request. A lookup that a source couldn't answer gets the same minute, since it is asked again then.

### A burst of clicks costs one lookup

- **The page** is computed from its address alone: no outside call, under a millisecond of CPU, `Cache-Control: public, max-age=86400`.
- **The cover** is answered from:
  1. the Worker's memory (what this instance already found);
  2. Cloudflare's edge cache, once a domain makes the Cache API work (until then that step does nothing, and the next one carries the load);
  3. the album's Durable Object.
  
  The Durable Object is one object per album or artist, worldwide, and it runs one request at a time. So a thousand people opening the same new link at once wait on one lookup, and one row records the answer.
- **The bytes** are fetched from the source's CDN (mzstatic, Deezer's, archive.org, Last.fm's), which are built for this. They're served with `Cache-Control: public, max-age=604800`, so Discord's and Telegram's image proxies and browsers keep them. Lull's card is served with a one-hour cache, so a later cover replaces it.
- **Polite to every upstream, whatever the traffic.** One `Gate` Durable Object hands out turns per service, worldwide. Over budget, a step is skipped (the chain moves on) rather than queued:
  - iTunes 10 a minute. Apple counts about 20 a minute per IP address, and Cloudflare's outgoing addresses are shared.
  - Deezer 10 a second.
  - MusicBrainz and the Cover Art Archive 1 a second.
  - Last.fm 2 a second.
  - Wikidata 1 a second.
  
  A 429 or 403 from a service pauses it for 10 minutes.
- **The Worker fetches only these services,** by id or name, never an address taken from a link. So it can't be used to fetch anything else.
- **Abuse:** Cloudflare's rate-limiting binding allows each client (by IP address) 60 requests a minute per location; over it, a 429 page in lull's style says to wait a minute. Its docs don't mark it paid-only. If the free plan refuses it at deploy, the binding comes out, and this ADR and the 429 page's text say so. The gate above protects the upstreams either way. The remaining risk is deliberate traffic using up the 100,000 requests a day, which would stop links for the rest of that day (UTC).

### Android App Links

- **The Worker serves `/.well-known/assetlinks.json`** on every host it answers, copied at build from `packaging/android/assetlinks.json` (one file, the release key). A host it serves is verified at its own root, so no second repository is needed.
- **The app's manifest gains an `autoVerify` filter** for each share host, `pathPrefix="/1/"`. The `tannhausercommons.github.io` `/lull/open` filter stays. Its root already serves the same `assetlinks.json` (checked 2026-11-04). That matters: on Android 11 and older, an app is verified only when every host in its `autoVerify` filters is.
- **`MainActivity`** takes a share host's https link as it takes the open page's: the whole URL goes to `SharedLinks`, then to the web UI's `#/open`, then to the hub's `/api/open`.
- **The app isn't installed:** Android opens the page in the browser. The page offers what ADR-0032's does:
  - **Open in the lull app** (an intent URL with `S.browser_fallback_url`, for an unverified or debug build);
  - **Open in lull in the browser** (the hub's address, kept in that browser only);
  - **Listen on every music app;**
  - **Get lull.**
  
  An in-app browser (Discord's) may not hand App Links to the app; that button is for it.
- **iOS** isn't in scope.

### Add to queue (2026-11-05, asked for by a listener)

- **For a song or an album the page offers Add to queue beside Open,** through the same paths: the app's intent URL or `lull://open`, or the listener's lull in the browser, each with `do=queue` beside the link. The shared link itself never carries it.
- **lull adds it to next up without playing or leaving the screen it showed;** with nothing loaded, the first song waits paused. Started by the link (the app wasn't running), it opens on Home with the song queued.
- **It joins the queue of whichever of the listener's devices is playing:** the queue lives in each device's player, so when the device the link opened on isn't playing and another is, lull sends it there (lull Connect's `enqueue` command), with a toast naming that device.
- **The page no longer acts on its own for a song or an album:** a remembered lull isn't opened after a moment, and a computer isn't asked "Open lull?" on load, since either would take the choice away. An artist, with nothing to queue, opens as before.

### The Discord card (2026-11-07, asked for by the listener: "like Spotify's")

Discord lets a site replace its link preview with a layout of Discord's own components (a component embed, 2026). lull's card in Discord is one:
- **What it shows:** "Song shared from lull" (or Album, Artist; "lull" links to lull's repository), the title, the artist, the album and length; the cover beside them; link buttons **Open in lull** (the page) and **Every music app** (song.link or album.link), else **Last.fm**. Only those hosts, as above.
- **Its edge is the cover's colour,** as Spotify's card is filled with one. The colour is read once, in the album's `Cover` object, when its picture is found: from a 128 px copy of the picture, by the averages a JPEG keeps for each 8×8 block, so nothing is decoded in full (a millisecond or two). Only the colour is kept, beside the picture's address. A picture that isn't a JPEG has none, and the edge is lull's colour.
- **The page stays made from its address alone.** It links the card at `…/card`, which Discord fetches like the cover. The card waits up to 2.5 s for the colour, which starts the cover's lookup for the cover Discord asks for next; past that it has lull's colour. Discord has 10 s for the page, the card and the cover together.
- **Discord's limits** (3,000 bytes, 512-character button addresses) are checked when the page is made: a card that can't fit isn't linked, and Discord shows the Open Graph card, which every other app keeps showing.
- **What a site's card can't have:** a player, a preview or any audio, a card filled with colour, or a logo beside the title. Spotify's card is Discord's own integration.

Alternatives:
- **The colour in the link** (written by the hub). Every lull would have to read the cover's colour before sharing, and links copied before it would never have one.
- **The card inline in the page,** asking the `Cover` object for the colour on every page view. The page would depend on the lookup, and every view would cost a Durable Object request.
- **A per-song picture** drawn with the cover and the words (as some sites' cards are). A rasterizer doesn't fit the free plan's 10 ms (Workers Paid, $5 a month, would allow it), and Discord already lays out the words.

### One page script, two hosts

- `site/open.js` stays the one script. It reads the item from the `#lull?` part (old links) or from a JSON block the Worker writes into the page (new links).
- `site/open.css`, the icon and the font are copied into the Worker's assets at build, as `pages.yml` copies them for GitHub Pages.
- The link a lull opens is the page's address without `app=none`. Today's `linkOf`, which drops the whole query, changes with it.

### Hosts, and a domain later

- **`share/hosts.json` is the one list of share hosts,** and names which is canonical.
  - Until the owner has picked the account subdomain and the Worker is deployed, it lists no host and names none canonical. The hub then keeps writing ADR-0032's links, and the manifest has no new filter: an `autoVerify` host that can't verify would, on Android 11 and older, stop the open page's App Link verifying too.
  - Tests use a placeholder host, `lull.share-host.invalid`.
  - The hub compiles it in, and `share.rs` writes links on the canonical host. `open.rs` reads `/1/` links on any https host (above).
  - The Worker puts the canonical host in `og:url` when it serves one of these hosts; a share host someone runs for themselves names its own.
  - A test checks that the Android manifest lists every host.
- **A domain is added, never swapped:** add it to the list and the manifest, attach it to the Worker, then make it canonical.
- **The `workers.dev` address keeps serving every link forever,** and isn't redirected. If a domain lapses, links on it fail, but every link on `workers.dev` still opens and the canonical host can go back.
- **The Cache API starts working** on the domain with no code change.

### Old links

- `tannhausercommons.github.io/lull/open#lull?…` keeps working unchanged:
  - `site/` and `pages.yml` stay;
  - `open.rs` keeps reading `#lull?`;
  - the Android filter and the root `assetlinks.json` stay.
- The GitHub page doesn't forward to Cloudflare (Privacy, above). Old links don't become cards; every link copied from now on does.

### Cost

Everything above fits Workers Free:
- one Worker, with static assets;
- two SQLite Durable Object classes, a row per album or artist, written at most monthly;
- no KV, no R2, no paid add-ons.

The deploy job runs on GitHub Actions with `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` (from Bitwarden's `lull/` items, set as repository secrets by the integrator). `LASTFM_API_KEY` is put as a Worker secret from `lull/LASTFM_API_KEY`.

## Alternatives

- **Cloudflare Pages with Functions** (`<project>.pages.dev`). Its Cache API works on the free address, and the address doesn't need an account subdomain. But Pages can't define Durable Objects, so the single-flight store would be a second Worker and a second deploy, and Cloudflare now steers new projects to Workers with static assets. The Worker gets the edge cache when a domain arrives.
- **A short opaque id** (`/s/<hash>`) mapping to stored metadata. The links are shorter, but every hub would have to write to the Worker. Hubs can't hold a shared secret (lull is open source), so anyone could fill the store, and a lost store breaks every link. Self-describing links never depend on stored state.
- **Only an iTunes id in the link** (`/1/s/<id>`). Shortest, but the card's title then needs Apple to be up and to list the song, and many songs lull plays aren't in the store.
- **Keeping the cover's bytes** in Durable Object storage. It would survive every source going away, but storage then grows with whatever anyone crafts, and the 5 GB would need eviction. Keeping only the found address, and running the chain again when it fails, meets "never broken while one source has it" without that.
- **Redirecting to the source's CDN** instead of passing the bytes through. Cheaper, but a picture that later vanishes would be cached as broken by the chat app's proxy, under our address.
- **Drawing a per-song card** (title and artist rendered into the PNG). 10 ms of CPU on the free plan doesn't fit a rasterizer, and chat apps print the names anyway.
- **The hub warming the cover** when "Copy link" is pressed. It would make the first card faster, but the hub would then contact the share host, tying a hub's address to its links in Cloudflare's logs. The share host knows nothing of hubs.
- **Forwarding old links to the new page.** Old links would get the new page (still no card: the crawler never saw the `#` part), at the cost of sending what their senders were told stays in the browser.

## Consequences

- **Links now pass through a service.** If Cloudflare is down, new links don't open, though any lull still opens them pasted into search. Old links depend on GitHub Pages as before.
- **The account subdomain and the Worker's name are part of every link,** like `OPEN_PAGE` before. Neither is ever renamed.
- **`/api/share`'s `link` changes shape;** its `url` (the public page) and `artwork` don't. The web UI's "Copy link" needs no change. `open.rs` reads both shapes.
- **The app's manifest changes,** so App Links for the new host reach phones with the next app release. Until then, a tap opens the page, whose button opens the app.
- **New things to keep working:** the `share/` Worker with its tests (wrangler's local runtime, run in CI), its deploy job, and the gate's budgets when a service changes its limits.
- **Costs per new album shared:** at most one call to each source in the chain, inside each source's budget. Repeat opens cost no outside call, except the image bytes from a CDN for viewers whose app or browser hasn't cached them.
