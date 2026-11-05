# Fingerprint trial

The measurements behind [ADR-0016](../../docs/adr/0016-checking-a-copy-by-its-sound.md): is a YouTube upload the recording the listener asked for, judged by its sound? Not part of the hub; run by hand on a workstation, never against a listener's hub or Soulseek account.

- `src/main.rs` (`fptrial <reference> <candidate>...`): decodes each file (symphonia: AAC/M4A, MP3, FLAC, Vorbis, ALAC, WAV; Opus in WebM through ruopus), fingerprints it with rusty-chromaprint (Chromaprint's algorithm 2, the one AcoustID uses), and slides the reference over the candidate for the offset with the fewest differing bits. One JSON line per candidate: `ber` (bit error rate at the best offset: 0 is identical, about 0.5 is unrelated), `offset_s` (where the reference sits in the candidate), timings.
- `scripts/fpcands.py <dir> <songs.txt>`: per song (`artist | title | length_s`), Apple's 30 s preview as the reference and YouTube uploads found by searching, labelled by their titles (the song, fan uploads, live, cover, sped-up, slowed, karaoke, instrumental, remaster, radio edit, acoustic, other artists' songs of the title). `LIGHT=1` asks for fewer. The labels are guesses: correct them by hand in `<dir>/labels.json` (`{ "<file>": { "kind", "expect": "pass"|"refuse"|"either" } }`) before counting.
- `scripts/fprefs.py <dir>`: picks each song's Apple track strictly (artist and title equal) and records its length.
- `scripts/fprun.py <dir>`: downloads each upload (AAC 140, one at a time, 4 s apart) and scores it against the references.
- `scripts/fpanalyze.py <dir>[,<dir>...] [ber] [length share]`: per kind, wrong accepted and right refused under the sound alone, today's words-and-length rule, and the proposed three zones.
- `scripts/fpagree.py <dir>`: for songs without a reference, how alike their uploads are to each other.
- `scripts/qforms2.py <songs.txt> <out.json> US,DE,JP,BR`: YouTube Music's song search with the artist quoted or not, per storefront (the LENA fix).
- `scripts/storefronts.py <songs.txt> US,JP,DE,BR,KR <out.json>`: whether Apple has the song with a preview, per storefront.

`songs-varied.txt` is a set across languages and scripts. A listener's own songs make a better set (likes, library and history); keep that list out of the repository.

```sh
cargo build --release   # in this folder; set CARGO_TARGET_DIR outside a synced or protected folder
export FPTRIAL=$PWD/target/release/fptrial YTDLP=yt-dlp
python3 scripts/fpcands.py data songs.txt && python3 scripts/fprefs.py data && python3 scripts/fprun.py data
python3 scripts/fpanalyze.py data 0.2 0.25
```
