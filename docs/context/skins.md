# Skins: Winamp Skin Museum proxy, `.wsz` cache, favourites

`crates/skins`, for Classic mode (DESIGN.md §12). Endpoints: [api.md](api.md#collection-cratescollection-cratesskins).

## The museum API (checked 2026-10-31)

- **Endpoint.** `POST https://api.webamp.org/graphql`: public, no key, CORS open.
- **Browse.** `skins(first, offset, sort: MUSEUM)` returns `{ count, nodes }`.
  - **The design's `skins(sort: MUSEUM, filter: APPROVED)` is now refused** ("Combining `sort: MUSEUM` with `filter` is not supported").
  - lull sorts by `MUSEUM` without a filter and drops `nsfw: true` skins itself. In the museum order NSFW skins are rare (none in the first 200).
- **Search.** `search_classic_skins(query, first, offset, nsfw)` returns a list with no total, so lull asks for 31 rows to learn whether there's a next page.
- **Lookup.** `fetch_skin_by_md5(md5)` returns the skin with `download_url`, or `null`. lull uses it for downloads and favourites.
- **Fields used:** `md5`, `filename`, `screenshot_url` (275x348 PNG on `r2.webampskins.org`), `museum_url`, `nsfw`, `download_url`. There is no display name; lull shows the file name without `.wsz`, with `_` as spaces.
- **Files.** `.wsz` files are served from `r2.webampskins.org/skins/<md5>.wsz` with no CORS headers, which is why they are proxied. The md5 is the file's own md5; verified on a download.
- **Terms.** No terms of use, rate limits or API policy are published: not at skins.webamp.org/about, and not in the skin-database package. Webamp is MIT (Jordan Eldredge).
  - The about page credits the collection to the Internet Archive and asks nothing in return.
  - The skin-database README calls the GraphQL server "not currently used by anything", so the API comes with no stability promise.
  - lull behaves as below to stay polite, and keeps the design's attribution: "Skins from the Winamp Skin Museum" plus a per-skin `museumUrl`.

## Politeness

- **Debouncing** search keystrokes (300 ms) is the UI's job.
- **Pages are cached in memory for an hour,** up to 200 pages, oldest dropped first.
- **On a failure, the museum gets no calls for 30 s;** requests in that window get 503. If a cached page exists, even an expired one, it is served instead.
- **The shared HTTP client** adds timeouts and bounded retries with backoff (it honours `Retry-After`).
- **A `.wsz` is downloaded once,** then served from disk forever: it is named by its md5 and never changes.

## `.wsz` validation and cache

- **What counts as a skin:** the zip's central directory has an entry named `main.bmp`, in any folder and any case (Webamp searches the same way). Nothing is unpacked. Anything else (an HTML error page, a truncated zip, a modern `.wal`) is answered with 502 and is not cached.
- **Size cap: 16 MiB.** Classic skins are a few hundred KB.
- **Storage.** Files are cached at `<LULL_DATA_DIR>/skins/<md5>.wsz`, written to a `.part` file then renamed so a reader never sees half a file. The cache is not capped: it only grows by skins the listener opened.
- **Favourites** (`skins_favourites`) store the museum's metadata when added, so the favourites list renders offline. `downloaded` in any list means the file is in the cache, for the drawer's offline view.
