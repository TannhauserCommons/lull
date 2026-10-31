# ADR-0011: One compose file, three network shapes chosen by profile

- Status: accepted
- Date: 2026-10-31

## Context

Other Jellyfin users should deploy lull on whatever network they have "with one compose file and one env file" (PROJECT.md). The three networks that matter:

- **Plain:** slskd on the host's own connection.
- **VPN:** slskd's traffic through a VPN, with a forwarded port so peers can reach it.
- **Existing:** the listener already runs slskd.

lull and slskd must share the transfers folder and must not run as root.

## Decision

`deploy/compose.yaml` holds all three shapes, and `COMPOSE_PROFILES` in `.env` picks one: `plain`, `vpn`, or empty for an existing slskd.

- **slskd's settings are written once,** as a YAML anchor shared by the plain and VPN services.
- **VPN:** slskd joins gluetun's network namespace. slskd's own gluetun integration (since slskd 0.24.4) reads the forwarded port and listens on it. gluetun carries the network alias `slskd`, so `LULL_SLSKD_URL` is the same in both shapes.
- **One value each for shared settings:**
  - slskd's administrator API key is `LULL_SLSKD_API_KEY`.
  - Sharing follows `LULL_SHARE` through a one-line wrapper around slskd's start command, since slskd reads shares only from its startup settings.
- **Non-root:** both containers run as `RUN_AS` (the owner of the music folder). A one-shot `init` service hands fresh, root-owned volumes to that user.
- **Keys without the `LULL_` prefix are kit settings.** lull ignores them, so one env file serves compose, slskd and lull.

Alternatives not taken:

- **One compose file per shape:** three copies of slskd's settings drift apart.
- **Feeding gluetun's port through its up-command or lull:** this needs slskd's remote configuration. The built-in integration does the same with no extra moving part.
- **Asking users to `mkdir` and `chown` folders before the first start:** an easy step to miss, and the failure it causes is confusing.
- **A separate env file for slskd:** that breaks the "one env file" goal.

## Consequences

- **Some secrets reach lull's environment.** lull receives the whole env file, including slskd's Soulseek password and VPN credentials, which it never reads. That's acceptable for one listener's own containers, and it keeps the key list in one place.
- **A sharing change needs a restart.** Changing `LULL_SHARE` takes effect when slskd restarts (`docker compose up -d`).
- **The pinned images move together.** The slskd and gluetun images are pinned; a bump is checked against the shapes with `docker compose config` and against `crates/soulseek/src/layout.rs`.
