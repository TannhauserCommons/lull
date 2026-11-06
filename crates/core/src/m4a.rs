//! AAC frames written into an M4A (MP4) of their own, as they are: no re-encoding. A song cut from
//! a full-album upload is one (docs/context/youtube.md, Full-album uploads), so it is saved, tagged,
//! seeked and read like any YouTube AAC copy. One audio track in one chunk, its index (`moov`)
//! before the audio, so a player reading from the start knows the whole song at once.

/// One AAC frame and its length in samples (1024 for AAC-LC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub data: Vec<u8>,
    pub duration: u32,
}

/// The M4A of `frames`, whose AudioSpecificConfig is `asc` (from the source's `esds`), at `rate` Hz
/// with `channels`.
pub fn write(asc: &[u8], rate: u32, channels: u16, frames: &[Frame]) -> Vec<u8> {
    let ftyp = boxed(
        b"ftyp",
        &[b"M4A ".as_slice(), &0u32.to_be_bytes(), b"M4A ", b"mp42", b"isom"].concat(),
    );
    let audio: usize = frames.iter().map(|f| f.data.len()).sum();
    // The audio's offset is in the index, before it: measured once with any offset (its width is fixed).
    let at = ftyp.len() + moov(asc, rate, channels, frames, 0).len() + 8;
    let mut out = Vec::with_capacity(at + audio);
    out.extend(ftyp);
    out.extend(moov(asc, rate, channels, frames, u32::try_from(at).unwrap_or(u32::MAX)));
    out.extend(u32::try_from(audio + 8).unwrap_or(u32::MAX).to_be_bytes());
    out.extend(b"mdat");
    for f in frames {
        out.extend(&f.data);
    }
    out
}

fn moov(asc: &[u8], rate: u32, channels: u16, frames: &[Frame], audio_at: u32) -> Vec<u8> {
    let length: u64 = frames.iter().map(|f| u64::from(f.duration)).sum();
    let length = u32::try_from(length).unwrap_or(u32::MAX);
    let count = u32::try_from(frames.len()).unwrap_or(u32::MAX);
    let mvhd = full(
        b"mvhd",
        &[
            &[0u8; 8][..],
            &rate.to_be_bytes(),
            &length.to_be_bytes(),
            &0x0001_0000u32.to_be_bytes(),
            &0x0100u16.to_be_bytes(),
            &[0; 10],
            &MATRIX,
            &[0; 24],
            &2u32.to_be_bytes(),
        ]
        .concat(),
    );
    // flags 3: the track is enabled and in the movie
    let tkhd = boxed(
        b"tkhd",
        &[
            &[0, 0, 0, 3][..],
            &[0; 8],
            &1u32.to_be_bytes(),
            &[0; 4],
            &length.to_be_bytes(),
            &[0; 8],
            &[0; 4],
            &0x0100u16.to_be_bytes(),
            &[0; 2],
            &MATRIX,
            &[0; 8],
        ]
        .concat(),
    );
    // language "und", packed into three 5-bit letters
    let mdhd = full(
        b"mdhd",
        &[
            &[0u8; 8][..],
            &rate.to_be_bytes(),
            &length.to_be_bytes(),
            &0x55C4u16.to_be_bytes(),
            &[0; 2],
        ]
        .concat(),
    );
    let hdlr = full(b"hdlr", &[&[0u8; 4][..], b"soun", &[0; 12], b"SoundHandler\0"].concat());
    let smhd = full(b"smhd", &[0; 4]);
    let dinf = boxed(
        b"dinf",
        &full(
            b"dref",
            &[&1u32.to_be_bytes()[..], &boxed(b"url ", &[0, 0, 0, 1])].concat(),
        ),
    );
    let stbl = boxed(
        b"stbl",
        &[
            full(b"stsd", &[&1u32.to_be_bytes()[..], &mp4a(asc, rate, channels)].concat()),
            full(b"stts", &runs(frames)),
            // every frame in one chunk
            full(
                b"stsc",
                &[1u32, 1, count, 1]
                    .iter()
                    .flat_map(|n| n.to_be_bytes())
                    .collect::<Vec<_>>(),
            ),
            full(
                b"stsz",
                &[0u32, count]
                    .iter()
                    .copied()
                    .chain(frames.iter().map(|f| u32::try_from(f.data.len()).unwrap_or(u32::MAX)))
                    .flat_map(u32::to_be_bytes)
                    .collect::<Vec<_>>(),
            ),
            full(
                b"stco",
                &[1u32, audio_at]
                    .iter()
                    .flat_map(|n| n.to_be_bytes())
                    .collect::<Vec<_>>(),
            ),
        ]
        .concat(),
    );
    let minf = boxed(b"minf", &[smhd, dinf, stbl].concat());
    let mdia = boxed(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = boxed(b"trak", &[tkhd, mdia].concat());
    // an empty `udta` after the track, where a tag writer puts the tags, as in any M4A
    boxed(b"moov", &[mvhd, trak, boxed(b"udta", &[])].concat())
}

/// The sample entry: an AAC track (MPEG-4 audio, object type 0x40) and its configuration.
fn mp4a(asc: &[u8], rate: u32, channels: u16) -> Vec<u8> {
    let decoder_info = descriptor(0x05, asc);
    let decoder = descriptor(0x04, &[&[0x40u8, 0x15, 0, 0, 0][..], &[0; 8], &decoder_info].concat());
    let es = descriptor(0x03, &[&[0u8, 0, 0][..], &decoder, &descriptor(0x06, &[0x02])].concat());
    let esds = full(b"esds", &es);
    boxed(
        b"mp4a",
        &[
            &[0u8; 6][..],
            &1u16.to_be_bytes(),
            &[0; 8],
            &channels.to_be_bytes(),
            &16u16.to_be_bytes(),
            &[0; 4],
            &(rate << 16).to_be_bytes(),
            &esds,
        ]
        .concat(),
    )
}

/// `stts`: the frames' durations, as runs of equal ones.
fn runs(frames: &[Frame]) -> Vec<u8> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for f in frames {
        match runs.last_mut() {
            Some((n, d)) if *d == f.duration => *n += 1,
            _ => runs.push((1, f.duration)),
        }
    }
    let n = u32::try_from(runs.len()).unwrap_or(u32::MAX);
    n.to_be_bytes()
        .into_iter()
        .chain(
            runs.into_iter()
                .flat_map(|(n, d)| [n.to_be_bytes(), d.to_be_bytes()].concat()),
        )
        .collect()
}

const MATRIX: [u8; 36] = [
    0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0,
];

fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let size = u32::try_from(body.len() + 8).unwrap_or(u32::MAX);
    [&size.to_be_bytes()[..], kind, body].concat()
}

/// A box with a version and flags, both 0.
fn full(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    boxed(kind, &[&[0u8; 4][..], body].concat())
}

/// An MPEG-4 descriptor, its length in the four-byte form.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let n = u32::try_from(body.len()).unwrap_or(0x0FFF_FFFF);
    let len = [
        0x80 | (n >> 21) as u8 & 0x7F,
        0x80 | (n >> 14) as u8 & 0x7F,
        0x80 | (n >> 7) as u8 & 0x7F,
        n as u8 & 0x7F,
    ];
    [&[tag][..], &len, body].concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_comes_first_and_points_at_the_audio() -> Result<(), &'static str> {
        let frames: Vec<Frame> = (0..3u8)
            .map(|i| Frame {
                data: vec![i; 5 + usize::from(i)],
                duration: 1024,
            })
            .collect();
        let out = write(&[0x12, 0x10], 44_100, 2, &frames);
        let kinds: Vec<&[u8]> = {
            let (mut at, mut kinds) = (0, Vec::new());
            while at + 8 <= out.len() {
                let size = u32::from_be_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]]) as usize;
                kinds.push(&out[at + 4..at + 8]);
                at += size;
            }
            assert_eq!(at, out.len(), "the boxes fill the file");
            kinds
        };
        assert_eq!(kinds, [b"ftyp".as_slice(), b"moov", b"mdat"]);
        // past the box's kind, its version and flags, and its entry count
        let stco = out.windows(4).position(|w| w == b"stco").ok_or("no stco")? + 4 + 4 + 4;
        let at = u32::from_be_bytes([out[stco], out[stco + 1], out[stco + 2], out[stco + 3]]) as usize;
        assert_eq!(&out[at..], [vec![0u8; 5], vec![1; 6], vec![2; 7]].concat());
        Ok(())
    }
}
