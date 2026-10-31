---
title: lull: goal and priorities
approved: 2026-10-31, by the user
---

# lull

Written with the user and changed only with their OK. If a request conflicts with this page, ask before acting.

## Goal

A complete, self-hosted music player for one listener who already runs Jellyfin. It should:
- find music on Soulseek and start playing it within seconds, while the transfer runs;
- save what they want to keep into Jellyfin, properly tagged;
- recommend niche music with reasons it can explain;
- scrobble to Last.fm.

It works the same in the browser, as an installed PWA and in a thin desktop wrapper, in five switchable designs. Other Jellyfin users can deploy it on whatever network they have, with one compose file and one env file.

## Non-goals

- Accounts or multi-user. One listener, with an optional shared token.
- Heavy ML or any external AI service. The taste model is small, on-device and explainable.
- Scraping sites whose terms forbid it (Rate Your Music included).
- Keeping music the listener didn't ask to save. The stream cache is temporary and capped.
- Logic in the desktop wrapper. It only hosts the web UI.

## Current priority

The first complete version.
- **Every feature in the brief works end to end** against a real Soulseek account, a real Jellyfin server and a real Last.fm account:
  - streaming, save, discovery, taste and Last.fm;
  - the full player: queue, playlists and M3U, lyrics, shortcuts and media keys;
  - Classic mode with Winamp skins;
  - the PWA and the desktop wrapper.
- **It runs on the user's homelab** at `https://lull.<tailnet>.ts.net`.

## Decisions

| Date | Decision | Why |
| --- | --- | --- |
| 2026-10-31 | Name: lull | Already drawn into every design, wordmark and icon |
| 2026-10-31 | All five designs ship (lull, skydeck, starlight, petal, glasshouse), day/night/system, switchable live; the choice is stored per device | User's wish; the design system was built for it |
| 2026-10-31 | Private repo for now, public later; the container image on GHCR is public | The homelab pulls public images with no registry auth; other Jellyfin users can install it once the repo is public |
| 2026-10-31 | Soulseek through an slskd sidecar | Mature, maintained, has an API; its growing download files allow progressive streaming (ADR in docs/adr) |
| 2026-10-31 | Deploy kit covers any network: plain, VPN (gluetun with a forwarded port), or an existing slskd | User's wish: easy setup for other people whatever their network |
| 2026-10-31 | The user's homelab runs slskd with no VPN and no inbound port, like its qBittorrent | Homelab rule: no router ports |

## Open questions for the user

- The licence to publish under when the repo goes public.
