# ADR-0090: The app is the hub, a small Linux package serves the rest, and both update from any device

- Status: accepted
- Date: 2026-11-06

## Context

Listeners aren't expected to know Docker. Until now, a hub that stays on meant a container (`deploy/compose.yaml`) configured with a `.env` file. Its first setup code was printed only in the container's log. Updating it meant pulling a new image.

The desktop app (Windows and Linux) can already be the hub:
- it runs `lull_server::run` in its own process, with slskd, yt-dlp and Deno bundled (`app/build.rs`);
- it advertises itself on the local network and can set up `tailscale serve`;
- it updates itself with Tauri's signed updater.

It had no way to keep running with its window closed on a computer without a tray. It also lets the computer sleep under the hub.

The `lull` binary already serves headless, and the settings the web page changes are stored in the hub's database (`lull_core::settings`, `GET/PATCH /api/config`). So a server needs packaging, a few commands and self-update, not a second kind of hub.

## Decision

Three ways to run a hub, one hub program.

### 1. The desktop app is the hub

As before, plus **"Keep lull running when closed"** in Settings:
- lull starts at login with no window;
- Close hides it instead of quitting, with or without a tray;
- while the hub runs on mains power, the computer doesn't sleep when idle, and Settings says so. A closed laptop lid still sleeps.

### 2. A small Linux server package

`.deb`, `.rpm` and `.tar.gz`, for x64 and ARM64.
- **What it holds:** `lull`, slskd, yt-dlp and Deno (the versions the app pins) and their licences. Nothing else.
- **How it runs:** a systemd service under its own user, `lull`, with its data in `/var/lib/lull`.
- **The same hub as the app:** the hub runs its own slskd (`soulseekManaged`), advertises itself on the network, and uses the same code as the app for both.
  - The slskd supervisor, the network advert and the Tailscale helper live in `lull-server`.
  - The app and the package both use them.
  - Everything the app's hub does works the same: Soulseek, YouTube, discovery, sharing and share links, themes and sounds, and the library.
- **Commands:**
  - `sudo lull setup`: a few plain questions with good defaults. Where the music is (a folder, or an existing Jellyfin server's address and key). The Soulseek account. Whether to share the music. Then it prints:
    - the address to pair with and the setup code;
    - how to reach the hub from outside the home: Tailscale's serve if Tailscale is on this server, otherwise a reverse proxy or the listener's own domain, with `LULL_PUBLIC_URL`.
  - `sudo lull status`: the version, each part's state, the addresses and whether a device is paired.
  - `sudo lull pair`: a link that pairs a phone or computer.
  - The commands talk to the running hub over `<data>/admin.sock`, which only root and the hub's own user can open, through the routes the web page uses.
  - `sudo lull config get|set|unset KEY [value]`: one setting, validated as `PATCH /api/config` validates it. The hub then restarts its crates.
  - `sudo lull update`: the self-update below, from the command line.
- **One place per setting:**
  - Every setting the listener changes is stored where the web page stores it, the hub's database. `lull config`, `lull setup` and the web page all read and write it there.
  - The service file sets only the keys that describe how lull is run (`settings::ENV_ONLY`: the data folder, the listen address, the bundled programs).
  - Nothing needs editing by hand.
- **Jellyfin:**
  - `lull setup` and the web page can point lull at an existing Jellyfin server, or at the music folder Jellyfin already reads;
  - saves land where Jellyfin sees them (ADR-0003), so listeners keep their Jellyfin.
- **Reaching it:**
  - other devices on the network find it by themselves (`_lull._tcp`, as the app's hub);
  - it works behind a reverse proxy or on its own domain (`LULL_PUBLIC_URL`);
  - Tailscale's serve works where the server has Tailscale.

### 3. Docker, one option

`deploy/compose.yaml` stays, for NAS boxes and homelabs. It is never required.

### Updates from any paired device

`POST /api/updates/apply`: one action, no parameters. It means "update to lull's newest release": releases only, upward only.

| Install | What it does |
|---|---|
| Desktop app | its own updater, once nothing is playing on that computer; the app restarts |
| Linux package and Docker | the release's **hub kit**, as below |

The **hub kit** is `lull`, slskd, yt-dlp and Deno for that architecture. Its manifest names each file's SHA-256 and the kit format it needs. The manifest is signed with the key the app's updater uses.

How the hub updates from a kit:
1. **Download.** The hub downloads the kit into its data folder and checks it. It backs up the database with SQLite's online backup, then exits asking for a restart.
2. **Start.** The service's `lull` is its own launcher. It runs the newest kit that verifies, is newer than itself and isn't marked bad.
3. **The health window.** If the kit isn't healthy within 2 minutes, the launcher puts the database back and runs the previous version, and the page says so. A newer package or image installed by hand wins over older kits.
4. **Docker.** In Docker the kit's slskd isn't used, since compose pins slskd. A change to the image itself still needs `docker compose pull`.
5. **The service file or the launcher.** A change to either raises the kit format. The page then says "Install the new lull package once".

Security:
- the hub never runs as root, and writes only its data folder;
- every start checks the signature;
- `LULL_SELF_UPDATE=off` turns the button off.

### Proof in the queue

A queue check installs the `.deb` on Ubuntu and the `.rpm` on Fedora, and starts the service. It then runs the hub contract against it:
- setup and pairing;
- every part reporting in `/health`;
- slskd, yt-dlp and Deno running from the package;
- the network advert;
- the web UI and share pages served.

The same contract runs against the image, so the package hub and the others are held to one test.

## Rejected

- **Docker as the only server install:** listeners don't know Docker.
- **A settings file for the package** (`/etc/lull/lull.conf`): a second place for settings the web page also changes.
- **Updates through an apt or rpm repository:** each update needs root, and none can be pressed from a phone. It can be added later beside self-update.
- **Windows and macOS server installers:** the app is the hub on Windows. macOS is outside the project for now.

## Open

- Windows code signing, to drop "Windows protected your PC" on the installer.
- macOS: 99 USD a year to Apple, and someone with a Mac to try each release.
