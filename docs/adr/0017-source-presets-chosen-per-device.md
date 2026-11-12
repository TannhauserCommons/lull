# ADR-0017: Source presets, chosen per device

- Status: accepted (owner, 2026-11-12: "For the per device quality, it's just a source preset. We could just have like a different setting that we have for sources as like a customizable like preset and we could have the default preset for a phone so that it chooses lower quality.")
- Date: 2026-11-12

## Context

A listener on an iPhone, on mobile data, got FLAC. He asked for a mobile preset that puts the fastest copy first and caps quality, and a PC preset where FLAC can stay on.

Until now there was one streaming profile for the whole hub (`sources.stream`, ADR-0008): its minimum (lossless only, bitrate, formats), its cap (`maxKbps`, ADR-0012's update "the cap is a maximum"), which copy first (`pick`) and what happens when no copy is good enough (`below`). Every device got what that one profile chose. A listener who switched FLAC off for their phone switched it off for their desktop too, and the other way round.

The hub already knows which device asks: every request carries a paired device (ADR-0002). What it doesn't know is what kind of device that is: a device is a browser, the app or a Subsonic client, and "the app" is Windows, Linux and Android alike.

One fetch per song is shared by everyone who wants it (streaming.md#fetches), so a phone and a desktop asking for the same song met in one fetch, run by one profile.

## Decision

**A source preset is a named streaming profile, and each device uses one.**

- **What a preset holds:** the whole streaming profile as it is today (`lull_core::quality::StreamProfile`): lossless only, minimum bitrate, maximum bitrate, formats, which copy first (`pick`: fastest or highest quality) and what happens when none is good enough (`below`). Settings › Sources › Streaming edits the preset selected there.
- **What stays the hub's:** the saving profile (a saved file is the library's, whichever device asked), the peer limits, "Switch to a better copy" and YouTube's settings. YouTube's mode decides how the hub looks for every song, saves included, and is not part of a preset.
- **Two built-ins always exist.** "Desktop" is the profile the hub had: an existing setup's `sources.stream` becomes it, so nothing changes for anyone until they choose otherwise. "Phone" is Desktop with fastest first, a 320 kbps cap and the lossless formats (FLAC, WAV, AIFF) off; a Desktop that was lossless only isn't on the phone. Built-ins can be edited, not renamed or deleted. Listeners make, rename, edit and delete their own (a copy of the preset selected, up to 20).
- **Each device uses one preset,** chosen in Settings › Sources ("This device uses"), stored on the hub against the device's id. A device that hasn't chosen uses the default for its kind, worked out from the request that asks:
  - **Phone** when its `User-Agent` names Android, iPhone, iPad, iPod or Mobile, when it sends `Sec-CH-UA-Mobile: ?1`, or when it is the Android app's own player (`lull-app`). That covers the web app on a phone or tablet and the Android app.
  - **Desktop** otherwise: a computer's browser, the Windows and Linux app, an iPad whose Safari asks as a Mac.
  - Choosing a preset overrides the default; choosing "the default for this device" again goes back to it. Deleting a preset sends the devices that used it back to their default.
- **The hub applies the requesting device's preset to everything it fetches for that device:** the start, start fast's race, a better copy and the switch to it, preloads (each device's list with its own preset), later plays from the cache, the source picker's order and the YouTube copy's format.
- **Shared fetches** (streaming.md#fetches, "Presets"): fetches are kept per song *and* streaming profile. Devices whose presets hold the same profile share one fetch exactly as before. A device with another profile joins a running fetch only when the copy it has suits that profile: an allowed format, within its cap and passing its minimum. Otherwise it gets its own fetch, which never takes a copy another fetch of the song is transferring, so one copy never downloads twice at once. The cache may then hold a copy for each, and each later play takes the best its own preset allows. A phone is never served a copy its preset excludes unless its own `below` rule allows it (the cap's last resort, `playBest`), or the listener picked that copy.
- **Stored** with the rest of the settings (`sources.presets`, `sources.devices`), published in the `settings` event, changed only through `/api/sources/presets` and `/api/sources/device` (api.md).

## Rejected

- **A per-device override of single settings** (a cap per device, a format list per device): two places would decide one copy, and a listener couldn't tell which one won. A preset is the whole profile, so the one in use says everything.
- **Classifying a device once, when it pairs:** an app or browser can't say at pairing what it is, a device's own player (Android's) asks with another user agent than its pages, and a stored guess goes stale. The request says it every time.
- **Telling the device's kind from screen size in the web app:** the audio a phone streams is fetched by an `<audio>` element or Android's own player, which send no such hint; only headers reach the hub on every request.
- **One fetch per song, run by the latest asker's profile:** a desktop playing FLAC would be cut off by a phone opening the same song, or the phone would get the FLAC.
- **YouTube's mode in the preset:** it decides how saves and every device's songs are looked for, and the race and fallback would need a second set of rules per song. Fastest first is what the phone preset changes.

## Consequences

- A listener with one preset in use sees and gets exactly what they had.
- Two devices with different presets playing the same song at the same moment may each fetch a copy; the bytes cross the network twice only then.
- A preset changed while a song plays applies from the next song: the fetch running keeps the profile it started with.
- `PATCH /api/settings` no longer takes `sources.stream`; an older UI's change is refused with the endpoint to use.
