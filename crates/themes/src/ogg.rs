//! Just enough of Ogg (RFC 3533) to tell a pack's sound is Ogg Opus or Ogg Vorbis and how long it
//! plays, without decoding it (ADR-0081): every page is walked and its checksum checked, the first
//! packet names the codec, and the last page's granule position is the length in samples.

/// What a sound file holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sound {
    pub codec: Codec,
    pub seconds: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Opus,
    Vorbis,
}

const CAPTURE: &[u8; 4] = b"OggS";
const HEADER: usize = 27;
/// Page flags: the first page of a stream.
const BEGINS: u8 = 0x02;
/// Opus counts its granule positions at 48 kHz, whatever rate the sound was made at.
const OPUS_RATE: f64 = 48_000.0;

/// Reads `bytes` as one Ogg stream of Opus or Vorbis. The error completes "the file ...".
pub fn probe(bytes: &[u8]) -> Result<Sound, String> {
    let pages = pages(bytes)?;
    let first = pages.first().ok_or("is empty")?;
    if first.flags & BEGINS == 0 || pages.iter().any(|p| p.serial != first.serial) {
        return Err("is not one Ogg stream (it starts mid-stream or holds several)".into());
    }
    let packet = first_packet(first);
    // -1 marks a page on which no packet ends; the last one that ends a packet tells the length
    let granule = pages.iter().rev().map(|p| p.granule).find(|g| *g != -1).unwrap_or(0) as f64;
    let (codec, seconds) = if let Some(head) = packet.strip_prefix(b"OpusHead") {
        // version, channels, then the pre-skip: samples the decoder drops at the start
        let pre_skip = head.get(2..4).ok_or("has a cut-off OpusHead")?;
        let pre_skip = f64::from(u16::from_le_bytes([pre_skip[0], pre_skip[1]]));
        (Codec::Opus, (granule - pre_skip) / OPUS_RATE)
    } else if let Some(head) = packet.strip_prefix(b"\x01vorbis") {
        // version (4 bytes), channels, then the sample rate
        let rate = head.get(5..9).ok_or("has a cut-off Vorbis header")?;
        let rate = u32::from_le_bytes([rate[0], rate[1], rate[2], rate[3]]);
        if rate == 0 {
            return Err("has a Vorbis header with no sample rate".into());
        }
        (Codec::Vorbis, granule / f64::from(rate))
    } else {
        return Err("is Ogg, but not Opus or Vorbis".into());
    };
    if seconds <= 0.0 {
        return Err("holds no sound".into());
    }
    Ok(Sound { codec, seconds })
}

struct Page<'a> {
    flags: u8,
    granule: i64,
    serial: u32,
    lacing: &'a [u8],
    body: &'a [u8],
}

fn pages(bytes: &[u8]) -> Result<Vec<Page<'_>>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes.get(at..at + 4) != Some(CAPTURE.as_slice()) {
            return Err(if at == 0 {
                "is not an Ogg file".to_owned()
            } else {
                format!("has something that is not an Ogg page at byte {at}")
            });
        }
        let header = bytes.get(at..at + HEADER).ok_or("is cut off inside an Ogg page")?;
        if header[4] != 0 {
            return Err("uses an Ogg version this lull doesn't know".into());
        }
        let segments = usize::from(header[26]);
        let lacing = bytes
            .get(at + HEADER..at + HEADER + segments)
            .ok_or("is cut off inside an Ogg page")?;
        let end = at + HEADER + segments + lacing.iter().map(|&b| usize::from(b)).sum::<usize>();
        let page = bytes.get(at..end).ok_or("is cut off inside an Ogg page")?;
        let stored = u32::from_le_bytes([header[22], header[23], header[24], header[25]]);
        if checksum(page) != stored {
            return Err(format!("is damaged (the Ogg page at byte {at} fails its checksum)"));
        }
        let mut granule = [0; 8];
        granule.copy_from_slice(&header[6..14]);
        out.push(Page {
            flags: header[5],
            granule: i64::from_le_bytes(granule),
            serial: u32::from_le_bytes([header[14], header[15], header[16], header[17]]),
            lacing,
            body: &page[HEADER + segments..],
        });
        at = end;
    }
    Ok(out)
}

/// The packet the stream starts with: its segments up to the first one shorter than 255 bytes.
fn first_packet<'a>(page: &Page<'a>) -> &'a [u8] {
    let mut len = 0;
    for &segment in page.lacing {
        len += usize::from(segment);
        if segment < 255 {
            break;
        }
    }
    &page.body[..len.min(page.body.len())]
}

/// Ogg's CRC-32: polynomial 0x04c11db7, not reflected, starting at 0, over the page with its own
/// checksum field read as zeros.
pub(crate) fn checksum(page: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut r = (i as u32) << 24;
            let mut bit = 0;
            while bit < 8 {
                r = if r & 0x8000_0000 != 0 {
                    (r << 1) ^ 0x04c1_1db7
                } else {
                    r << 1
                };
                bit += 1;
            }
            table[i] = r;
            i += 1;
        }
        table
    };
    page.iter().enumerate().fold(0u32, |crc, (i, &b)| {
        let b = if (22..26).contains(&i) { 0 } else { b };
        (crc << 8) ^ TABLE[usize::from((crc >> 24) as u8 ^ b)]
    })
}

/// Ogg files made in code for this crate's tests.
#[cfg(test)]
pub(crate) mod fixture {
    /// One page with one packet (under 255 bytes), its checksum filled in.
    pub fn page(flags: u8, granule: i64, sequence: u32, packet: &[u8]) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.extend([0, flags]);
        page.extend(granule.to_le_bytes());
        page.extend(7u32.to_le_bytes());
        page.extend(sequence.to_le_bytes());
        page.extend([0; 4]);
        page.extend([1, packet.len() as u8]);
        page.extend(packet);
        let crc = super::checksum(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        page
    }

    /// An Ogg Opus file that plays for `seconds`: its OpusHead page (pre-skip 312), then one last
    /// page whose granule position says where the sound ends.
    pub fn opus(seconds: f64) -> Vec<u8> {
        let mut head = b"OpusHead".to_vec();
        head.extend([1, 2]);
        head.extend(312u16.to_le_bytes());
        head.extend(48_000u32.to_le_bytes());
        head.extend([0, 0, 0]);
        let mut file = page(0x02, 0, 0, &head);
        file.extend(page(0x04, 312 + (seconds * 48_000.0) as i64, 1, &[0xfc, 0xff, 0xfe]));
        file
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_opus_length_comes_from_the_last_granule_less_the_pre_skip() {
        let sound = probe(&fixture::opus(1.5));
        assert_eq!(
            sound,
            Ok(Sound {
                codec: Codec::Opus,
                seconds: 1.5
            })
        );
        let mut damaged = fixture::opus(1.5);
        let last = damaged.len() - 1;
        damaged[last] ^= 1;
        assert!(probe(&damaged).is_err_and(|e| e.contains("checksum")));
        assert_eq!(probe(b"RIFF\0\0\0\0WAVE"), Err("is not an Ogg file".to_owned()));
    }
}
