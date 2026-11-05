# ADR-0096: Unfinished work lands on main switched off

- Status: accepted
- Date: 2026-11-04

## Context

The 0.2.2 release waited on features that were half done. Work that takes days either sits on a topic branch, where it drifts from main and conflicts, or lands half-finished, where the listener sees it. With main moving only through the queue (ADR-0095) and releases cut from its tip (ADR-0097), main must be releasable at every commit, unfinished work included.

## Decision

### One list of switches, in core

`lull_core::flags` holds every switch as a constant list: its name (`kebab-case`), one line on what it turns on, the session or area that owns it, and the release area it belongs to (ADR-0098). Each is **off unless turned on**. The list is the single source of truth: the hub, the web UI and the tests all read it.

- **The hub** asks `ctx.config.flags.on(&flags::NAME)`: a constant, so a typo doesn't compile.
- **Turning one on** is the setting `LULL_FLAGS` (a comma-separated list of names), set in the hub's environment or stored through `PATCH /api/config` like any other `LULL_*` key (ADR-0002); Settings shows no control for it. A name that isn't a switch stops the hub starting, as an unknown `LULL_*` key does.
- **The web UI** gets the names that are on in `/health`, the first answer it already reads at start, so a switch costs no request. It asks `App.flag('name')`.
- **Tests** run with every switch off, which is what ships. A feature's own tests turn its switch on (the fixture hub takes the same `LULL_FLAGS`).

### The rule

- **Big work lands dark.** Work that won't be finished in the commit that starts it lands behind a switch, in small commits through the queue. The switch is added in the first of them.
- **Off must be exactly today's lull.** With the switch off, no new request, element, setting or cost appears, so the UI gates' goldens and budgets don't move.
- **Finishing a feature removes its switch:** the code paths for off, the list entry and the tests that turned it on. A switch is never flipped on by default and left in place. `grep -rn "flags.on(\|App.flag(" crates web` lists what is still dark.

## Rejected

- **Long-lived topic branches:** they drift, conflict, and are never tested with everyone else's work.
- **Compile-time Cargo features:** the web UI can't read them, the hub image would need a build per combination, and the owner couldn't try a dark feature on their hub.
- **A flag service or a config file of its own:** a second place to look, for a one-listener app.
- **Per-device switches in the browser:** a feature that touches the hub and the UI must be on for both at once.

## Consequences

- **A release never waits for a feature;** whatever is unfinished ships switched off.
- **The owner can try a dark feature on their own hub** by setting `LULL_FLAGS`, and turn it off again, without a release.
- **Dark code is code:** clippy, tests and review hold it to the same standard, and it costs a reader something until its switch is removed. Removing the switch is part of finishing the feature, and its commit says so.
