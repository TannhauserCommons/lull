# ADR-0050: Settings reach other crates through a `settings` event

- Status: accepted
- Date: 2026-10-31

## Context

Settings > Sources, Sharing and Discovery change how other crates behave:
- soulseek: ranking preset, minimum bitrate, peer queue limit, stall timeout, upload slots and speed;
- discovery: adventurousness, which sources it uses.

Collection owns `/api/settings` and stores them. Domain crates can't depend on each other, and a crate's tables are its own.

## Decision

- **Collection publishes the stored settings as a `settings` event,** once at start-up and again after every change.
- **Crates that honour a setting subscribe in their `start()`.** Collection starts after every other crate (server/src/main.rs), so the start-up copy reaches them. Until then they use the documented defaults, the same values collection returns for a fresh install.

## Alternatives

- **A `Settings` seam in core,** implemented by collection and handed to others. It is the cleanest read-on-demand, but it needs a core trait and a change to the wiring in `main.rs` (integrator). It would also make every reader ask collection on each use.
- **Settings in a core table every crate reads.** One more shared table, written by one crate. That reverses "each crate owns its tables" for little gain.
- **Each crate stores its own settings behind its own endpoint** (as lastfm does for its switches). That works for one crate's switches, but it would split one Settings screen across many APIs.

## Consequences

- **The start-up copy depends on wiring order.** If `main.rs` ever starts collection before a reader, that reader gets no start-up copy. This is recorded in `api.md#events` and here; moving to a seam is the fix if it ever matters.
- **Readers keep the latest copy in memory.** There is no request path to collection.
