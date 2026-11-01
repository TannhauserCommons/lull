---
title: lull: goal and priorities
approved: 2026-11-01, by the user (lull 2 plan)
---

# lull

Written with the user and changed only with their OK. If a request conflicts with this page, ask before acting.

## Goal

A music app anyone can download and use like Spotify, hosted by the listener themselves. It should:
- find music on Soulseek and start playing it within seconds, while the transfer runs;
- save what they want to keep, properly tagged, into a music folder, or into Jellyfin when they use it;
- recommend niche music with reasons it can explain;
- scrobble to Last.fm;
- show the same history, likes and playlists on every device, with no account and no login.

**Where it runs.** Real apps for Windows, Linux and Android, plus the installed web app.
- On Windows and Linux, the app can be the listener's **hub**: the server, Soulseek and their library.
- Phones and other computers **pair** with the hub once by QR code.
- The hub also ships as a headless binary and a Docker image, for people who run a server.

**Look and feel.** It looks exactly like the design package on every screen size: five switchable designs, plus themes anyone can write in plain CSS. It's fast without giving up motion or features.

## Non-goals

- **Accounts or multi-user.** One listener per hub. Devices pair; nobody logs in.
- **A hosted relay or cloud service.** Remote access is the listener's own Tailscale.
- **App-store distribution.** Stores would likely refuse a Soulseek client.
- **Heavy ML or any external AI service.** The taste model is small, on-device and explainable.
- **Scraping sites whose terms forbid it** (Rate Your Music included).
- **Keeping music the listener didn't ask to save.** The stream cache is temporary and capped.
- **Logic in the app shells.** They host the web UI and the hub, and add only what a platform needs: tray, native audio, downloads to device storage.

## Current priority

lull 2 (plan approved 2026-11-01):
- **Hub and devices:** pairing, settings in the app, the desktop app as hub.
- **Library:** folder mode and Jellyfin mode, switchable with nothing lost.
- **The Android app:** a native background player and offline downloads.
- **Collection and devices:** the offline outbox, and lull Connect.
- **Subsonic API.**
- **Imports:** Spotify (Exportify CSV), Last.fm and ListenBrainz.
- **Theme packages and Theme Studio.**
- **Quality:** visual-regression and performance budgets in CI.

It is done when every item passes end to end on the homelab hub and on the Windows and Android apps.

## Decisions

| Date | Decision | Why |
| --- | --- | --- |
| 2026-10-31 | Name: lull | Already drawn into every design, wordmark and icon |
| 2026-10-31 | All five designs ship (lull, skydeck, starlight, petal, glasshouse), day/night/system, switchable live; the choice is stored per device | User's wish; the design system was built for it |
| 2026-10-31 | Private repo for now, public later; the container image on GHCR is public | The homelab pulls public images with no registry auth |
| 2026-10-31 | Soulseek through an slskd sidecar | Mature, maintained, has an API; its growing download files allow progressive streaming (ADR-0010) |
| 2026-10-31 | Deploy kit covers any network: plain, VPN (gluetun with a forwarded port), or an existing slskd | User's wish: easy setup whatever the network |
| 2026-10-31 | The user's homelab runs slskd with no VPN and no inbound port, like its qBittorrent | Homelab rule: no router ports |
| 2026-11-01 | One hub per listener; devices pair by QR, no accounts (ADR-0002) | Soulseek allows one login per account, and slskd can't run on a phone |
| 2026-11-01 | Jellyfin is optional: folder mode or Jellyfin mode, switchable, keyed by content-derived ids (ADR-0003) | User's wish; ids already don't depend on the source |
| 2026-11-01 | Themes are plain CSS packages; the five designs are written in the same format (ADR-0004) | Full freedom for theme authors, proven by the built-ins |
| 2026-11-01 | Offline downloads with an outbox; lull Connect between devices (ADR-0005) | Spotify-like use away from home |
| 2026-11-01 | One Tauri v2 app for Windows, Linux and Android; native audio on Android (ADR-0006) | User's wish: real apps, built on Rust; no macOS or iOS |
| 2026-11-01 | OpenSubsonic API, and Spotify / Last.fm / ListenBrainz import | Existing native apps work with lull; new listeners start with their music and taste |
| 2026-11-01 | Remote access through the listener's Tailscale; `tailscale serve` provides HTTPS | No relay to host; browsers need HTTPS to install the app |
| 2026-11-01 | Layouts reveal more as the window grows; resizable, collapsible panels and a reorderable Home, remembered per device (ADR-0007) | User's wish: big and odd window sizes used well, not stretched cards |

## Open questions for the user

- The licence to publish under when the repo goes public. AGPL-3.0 fits next to slskd (AGPL) and Jellyfin (GPL).
- Code signing for Windows (a certificate) before a public launch. Without one, SmartScreen warns on first install.
