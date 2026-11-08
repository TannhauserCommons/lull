# ADR-0013: Switching the playing song to a better copy, lined up by ear

- Status: accepted
- Date: 2026-11-03

## Context

In a test call (2026-11-03), a song waited about 10 s for Soulseek and fell back to YouTube at the same point. "Pick another source" then showed better Soulseek copies, but nothing switched to them. The listener: "it doesn't automatically switch, so that's a problem". The tester: switch automatically once the copy is ready, but seamlessly, or "you'll hear the jump".

Until then:
- **Start fast** fetched Soulseek's copy beside YouTube's, with the rule that the current play never jumps.
- **A fallback** to YouTube, a copy below the streaming minimum, or a slower peer after a stall stayed lesser until the next play.
- **A manual pick** reopened the stream at the same time. That is seamless enough for a deliberate choice, but it is audible: the copies differ in where their audio starts (YouTube intros, encoder delay, other masters), so the same time is not the same sound.

## Decision

**The hub fetches the better copy beside the one playing, and offers it.**
- **When:** whenever the player streams a lesser copy (YouTube's, one below the streaming minimum, a peer fallen back to) and a better one is known, or nothing is known yet.
- **How:** an upgrade fetch of the streaming profile's best copy, in the map that keeps it out of the player's way. It searches at most once more, in the `Prefetch` lane of the search budget.
- **The offer:** `stream` events carry `upgrade { …, ready }`. `ready` means all of it is here, or it arrives 1.5 times faster than it plays.

**The player lines the copies up by ear, then crossfades.**
- **Measuring:** the copy plays silently on the second deck. Both decks are recorded in the same Web Audio render quanta, and a pure cross-correlation (`web/align.js`) measures the lag: the low band for confidence and the coarse lag, then the full rate and a parabola for the sample.
- **Correcting:** a seek puts the copy slightly ahead, and a DelayNode holds it back by the measured lag. A second measurement must find under 0.25 ms left, or there is no switch.
- **Switching:** at a quiet moment, the player asks the hub to adopt the copy (`POST /api/stream/{id}/switch`) and fades equal-power over 200 ms.
- **The timeline:** the deck keeps its offset, so positions and seeks are the song's.

**Honesty rules.**
- When the copies can't be clearly lined up (low confidence, or another lag nearly as good), the switch waits for the next play and Source details says so.
- A copy the listener picked is never switched away from.
- A switch never loses position, and never doubles audio: the old copy's response is let go right after the fade, and the hub stops that transfer once nothing reads it.
- Settings › Sources › "Switch to a better copy while a song plays", on by default, turns it off. Start fast's "never jumps" rule then holds as before.

## Alternatives

- **Switching at the same time position (what a manual pick does):** audible whenever the copies' starts differ, which is the common case between YouTube and Soulseek.
- **Seeking until aligned:** a media element's seek lands only to within tens of milliseconds, and each try costs a re-buffer. The delay line is exact to the sample once measured.
- **Nudging `playbackRate` until aligned:** it converges slowly and needs several rounds of measurement, and the delay line makes it unnecessary.
- **Decoding both files (`decodeAudioData`) and correlating offline:** it needs whole files (FLAC fetched in full, about 90 MB decoded) and doesn't measure what the decks actually play.
- **Onset matching:** it is robust to timbre, but coarser than waveform correlation, which already copes with other encodes once judged on the low band.
- **An AudioWorklet for recording:** it is the modern API, but needs a module file and an async load. The ScriptProcessor is deprecated but works everywhere lull runs, and records both decks in one callback.
- **The hub replacing the stream when the copy is ready:** the player would hear a cut, and a seek during the change could land on the wrong file.

## Consequences

- **Two transfers at once** while a lesser copy plays: the better copy runs beside it. It stops when the song is skipped, unless a save needs it.
- **Web Audio is on by default** (`switchToBetter` builds the graph at the first play), as it already was through ReplayGain normalisation.
- **The Android app's native output** has no `switchTo`, so a better copy plays from the next play there.
- **A switch takes 8-15 s** from the offer: recording, measuring, a quiet moment. Songs within 45 s of their end wait for the next play.
- **Measured live** (2026-11-03, against the homelab hub with a local copy offered in place of the hub's):
  - a re-encoded MP3 with a 1.234 s intro was found at 1.259 s (the intro plus the encoder's 25 ms) and left 0.010-0.014 ms apart;
  - an AAC copy was left 0.0005 ms apart;
  - another part of the song never switched;
  - the timeline moved 16.94 s in 16.95 s of wall time across the switch.

## Update 2026-11-08: better means better, and the switch shows

The owner on 2026-11-07, about a song that switched: "the source on the bottom left didn't update to let me know".
- **The switch was invisible.** The player dropped every event of the new copy's job until the crossfade ended. A copy that had all arrived sent no more, so the player bar's chips kept naming YouTube.
- **"Better" meant "passes the streaming minimum".** With a 320 kbps minimum, a 256 kbps or V0 copy never replaced YouTube's Opus.
- **A copy below the minimum was only ever upgraded to a copy already known,** so with none known it never upgraded.
- **The next song's offer was lost at a gapless change-over.** Its stream opens up to 90 s before it plays, and the player ignored its offer until then.

**Decision.**
- **Better means better in quality:** a copy that clearly beats the one playing (lossless over lossy, or at least a fifth more bitrate), or that passes the streaming minimum the one playing fails, within the streaming cap.
- **With none known,** the song is searched again once per play.
- **The switch answers with the new copy's stream,** and its events name the job they replace, so every surface shows the copy the moment the fade starts.
- **A quiet mark:** a small arrow in the quality chip, with a tip saying from what and when, instead of a toast.
- **Activity records the outcome:** "switched to <format> from <peer>", or why the player couldn't.
- **The next song's latest event is kept** and applied when it hands over.
- **Where the switch picks up is unchanged:** at the point the song has reached, lined up to the sample, never from the copy's start. Copies that can't be lined up wait for the next play, which plays the better copy from its start.

## Update 2026-11-08: one streaming policy decides the switch

The owner: better quality "is whatever is available fast that's on or above the minimum streaming bitrate, higher score if it's closer to maximum", "no tiny gains", and "we already have a score system". The update above still had its own idea of what is better:
- a copy below the minimum counted when it beat the one playing;
- crossing the minimum counted as a step on its own;
- the copy playing was a "lesser" one by its source (`Lesser::Youtube`).

**Decision.** The streaming policy (`soulseek::policy`, ADR-0012's update of the same day) decides the switch, for a copy of any source.
- **The target** (`policy::better`) is the best copy that passes the minimum and is within `maxKbps` (lossless counts as above any cap), by lull's score for how it sounds (`rank::sound`). The speed expected only breaks ties.
- **The score, for this:**
  - **Sound decides, speed doesn't.** It is the score's format factor alone (`rank::sound`): where the format sits in `LULL_PREFERRED_FORMATS`, then its bitrate. Speed, a free slot and the queue decide only what starts the song.
  - **The bitrate part scales toward the profile's `maxKbps`, not a fixed 320** (no cap: toward 320, higher is better). So under a 256 cap a 256 kbps copy earns the full points, and the copy nearer the cap scores higher.
  - **It is one function.** The start's score adds speed, slot and queue to that same format factor (`rank::factors`), wherever streaming ranks copies (`Sources::ordered` for streaming, the race's tie, `ask`'s choices). Search results and saving still rank with no cap.
- **The switch** (`policy::switch_to`) happens only for a clear step in sound over the copy playing: lossless over lossy, or at least a fifth more bitrate (`Quality::beats`). 192 to 256 and 256 to 320 kbps are steps; 300 to 320 and 192 to 224 aren't.
- **The target may be any source's.** Soulseek's known copies are candidates, and so is YouTube's while start fast makes YouTube a source. The upgrade fetches whichever the policy names; a YouTube copy downloads as YouTube's always do.
- **When it is looked for:**
  - Whenever the policy knows a target for the copy playing, whatever its source. YouTube's copy is looked up behind the listener's own lookups, and is mostly remembered from the race.
  - With none known, the song is searched again once per play when the copy playing fails the minimum.
  - Never for a copy the listener picked.
  - So a song started by `pick: fastest` on a fast copy moves to the best-sounding passing copy known, when that is a clear step; under `pick: highest` it usually started on it already. A replay of a cached copy fetches a clearly better one known beside it.
