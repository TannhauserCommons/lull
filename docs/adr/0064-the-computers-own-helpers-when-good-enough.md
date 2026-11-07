# ADR-0064: lull uses the computer's own yt-dlp, Deno and slskd when they're good enough

- Status: accepted (owner, 2026-11-07: "go ahead do booth"; on a newer slskd series: "Use it, say so")
- Date: 2026-11-07

## Context

lull runs three helper programs: yt-dlp (YouTube), Deno (which yt-dlp needs) and slskd (Soulseek). The desktop app
always ran the copies it ships. On Arch those are files pacman doesn't own and that go stale between lull releases,
while Arch has the same programs, kept current (`yt-dlp` and `deno` in its repositories, `slskd-bin` in the AUR). The
owner asked for one rule for every system, not Arch-only logic.

## Decision

1. **Order:** the listener's override (`LULL_YTDLP`, `LULL_DENO`, `LULL_SLSKD`) > a copy on this computer that passes
   its check > lull's own copy. **Use lull's own copies** (Settings › App) puts lull's own first.
2. **Where lull looks:** the directories in `PATH`, then `/usr/bin` and `/usr/local/bin` on Linux (a desktop
   launcher's `PATH` can be shorter than a terminal's); never lull's own folder.
3. **The check, at each start of the hub:** `--version` answers within 5 s with at least the version lull is tested
   with (`lull_core::helpers`; the app's bundled pins match it, tested). yt-dlp: the newer of this computer's and lull's
   own, since YouTube breaks old yt-dlp first. slskd: a series lull's contract hasn't passed is still used, and Settings
   says lull hasn't been tested with it; if lull then can't use it (Soulseek down), Settings says so and points to
   lull's own copy where there is one.
4. **Settings › App** says in one line which copy each helper is, its version, and any note.
5. **Packaging:** the AUR `lull-bin` depends on `yt-dlp`, `deno` and `slskd-bin` and ships no copies, from the first
   release that carries this rule. Windows, .deb, .rpm and the AppImage keep bundling (those systems don't reliably have
   the programs) and still prefer a computer's copy that passes.
6. **Where it applies:** the desktop app's own hub. The Linux server package and the image set the paths themselves
   (overrides), so they are unchanged until their owners adopt `lull_core::helpers::choose`.

## Alternatives considered

- **Always the bundled copies:** files pacman doesn't own, stale between releases.
- **Always the computer's copy, whatever its version:** an old distro yt-dlp breaks YouTube; an untested slskd series
  can break Soulseek. Rejected for the version check.
- **Refusing a slskd series lull hasn't tested:** an AUR install has no copy of its own to fall back to. The owner chose
  to use it and say so.
- **Arch-only logic:** the owner asked for one rule.

## Consequences

- One `--version` run per helper at the hub's start.
- On Arch, `slskd-bin` brings its own (disabled) `slskd.service` and `slskd` system user; lull doesn't use them.
- The slskd series the contract has passed is a list in code (`SLSKD_TESTED_SERIES`), extended when it passes a new one.
- CI: the app, as its own hub, signs in to the slskd contract's stand-in Soulseek server with lull's own copies and
  with copies installed on the computer (graphics.yml); the AUR check does the same with Arch's packages.
