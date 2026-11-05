# Context index

Curated pages for what is expensive to rediscover: boundaries, contracts, invariants. Code and tests outrank these pages. A page marked *(owner writes)* is created by its area's owner with the first change there.

| Task | Read | Then open |
| --- | --- | --- |
| Anything (who owns what, seams, parallel work) | [workspace.md](workspace.md) | `Cargo.toml`, `crates/server/src/lib.rs` |
| Call or add an endpoint or event | [api.md](api.md) | the owning crate's `src/lib.rs` |
| Config keys, settings in the app | [api.md](api.md), `deploy/.env.example`, ADR-0002 | `crates/core/src/config.rs`, `crates/core/src/settings.rs` |
| Hub: pairing, devices, Connect, restart | [hub.md](hub.md) *(owner writes)*, ADR-0002 | `crates/server/`, `crates/core/src/devices.rs` |
| Soulseek: slskd, search, ranking | [soulseek.md](soulseek.md) *(owner writes)* | `crates/soulseek/` |
| Progressive streaming, peer fallback, switching to a better copy mid-song | [streaming.md](streaming.md) *(owner writes)*, ADR-0013 | `crates/soulseek/`, `web/align.js` |
| Stream cache and eviction | [cache.md](cache.md) *(owner writes)* | `crates/soulseek/` |
| YouTube as a second source: modes, matching, yt-dlp | [youtube.md](youtube.md), ADR-0012 | `crates/youtube/`, `crates/soulseek/src/fetch.rs` |
| Deploying (plain, VPN, existing slskd) | [deploy.md](deploy.md) *(owner writes)* | `deploy/` |
| Library sources (folder, Jellyfin), switching, saved-track streaming | [library.md](library.md) *(owner writes)*, ADR-0003 | `crates/library/` |
| Save flow (match, cover, tags, naming, rescan) | [save-flow.md](save-flow.md) *(owner writes)* | `crates/library/` |
| Discovery sources and their limits | [discovery.md](discovery.md) *(owner writes)* | `crates/discovery/` |
| Share links: the card pages on Cloudflare, covers, App Links | [share.md](share.md), ADR-0034 | `share/`, `crates/discovery/src/share_host.rs` |
| Taste model | [taste.md](taste.md) *(owner writes)* | `crates/discovery/` |
| Scrobbling rules, Last.fm auth | [scrobbling.md](scrobbling.md) *(owner writes)* | `crates/lastfm/` |
| Likes, playlists, M3U, history, stats, settings, sync, imports | [collection.md](collection.md) *(owner writes)*, ADR-0005 | `crates/collection/` |
| Subsonic API | [subsonic.md](subsonic.md) *(owner writes)* | `crates/subsonic/` |
| Themes: format, hooks, Theme Studio | [../themes.md](../themes.md) *(owner writes)*, ADR-0004 | `web/designs/`, `crates/themes/` |
| Offline downloads, outbox, lull Connect (UI) | [web.md](web.md) *(owner writes)*, ADR-0005 | `web/` |
| Visual tests, performance budgets, asset pipeline | [quality.md](quality.md) *(owner writes)* | `tests/ui/` |
| UI sounds, sound packs, easter eggs | [../themes.md](../themes.md) section 13, ADR-0081, [web.md](web.md) | `web/sounds.js`, `web/eggs.js`, `web/packs/`, `scripts/sounds/`, `crates/themes/src/pack.rs` |
| Classic mode skins | [skins.md](skins.md) *(owner writes)* | `crates/skins/`, `web/screens/skins.js`, `web/screens/classic.js` |
| Web UI: structure, data layer, player engine | [web.md](web.md) *(owner writes)*, `design/DESIGN.md` | `web/index.html`, `web/app.js` |
| Windows and Linux app | [desktop.md](desktop.md) *(owner writes)*, `design/desktop/DESKTOP.md`, ADR-0006 | `app/` |
| Android app, native player, F-Droid | [android.md](android.md) *(owner writes)*, ADR-0006, ADR-0061 | `app/src/mobile/`, `app/plugins/player/`, `packaging/fdroid/` |
| Why something is the way it is | [../adr/](../adr/) | the ADR |
