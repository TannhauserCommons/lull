# Context index

Curated pages for what is expensive to rediscover: boundaries, contracts, invariants. Code and tests outrank these pages. A page marked *(owner writes)* is created by its area's owner with the first change there.

| Task | Read | Then open |
| --- | --- | --- |
| Anything (who owns what, seams, parallel work) | [workspace.md](workspace.md) | `Cargo.toml`, `crates/server/src/main.rs` |
| Call or add an endpoint or event | [api.md](api.md) | the owning crate's `src/lib.rs` |
| Config keys | [api.md](api.md), `deploy/.env.example` | `crates/core/src/config.rs` |
| Soulseek: slskd, search, ranking | [soulseek.md](soulseek.md) *(owner writes)* | `crates/soulseek/` |
| Progressive streaming, peer fallback | [streaming.md](streaming.md) *(owner writes)* | `crates/soulseek/` |
| Stream cache and eviction | [cache.md](cache.md) *(owner writes)* | `crates/soulseek/` |
| Deploying (plain, VPN, existing slskd) | [deploy.md](deploy.md) *(owner writes)* | `deploy/` |
| Jellyfin library, saved-track streaming | [library.md](library.md) *(owner writes)* | `crates/library/` |
| Save flow (match, cover, tags, naming, rescan) | [save-flow.md](save-flow.md) *(owner writes)* | `crates/library/` |
| Discovery sources and their limits | [discovery.md](discovery.md) *(owner writes)* | `crates/discovery/` |
| Taste model | [taste.md](taste.md) *(owner writes)* | `crates/discovery/` |
| Scrobbling rules, Last.fm auth | [scrobbling.md](scrobbling.md) *(owner writes)* | `crates/lastfm/` |
| Likes, playlists, M3U, history, stats, settings | [collection.md](collection.md) *(owner writes)* | `crates/collection/` |
| Classic mode skins | [skins.md](skins.md) *(owner writes)* | `crates/skins/`, `web/screens/classic.js` |
| Web UI: structure, data layer, player engine | [web.md](web.md) *(owner writes)*, `design/DESIGN.md` | `web/index.html`, `web/app.js` |
| Desktop wrapper | [desktop.md](desktop.md) *(owner writes)*, `design/desktop/DESKTOP.md` | `desktop/` |
| Why something is the way it is | [../adr/](../adr/) | the ADR |
