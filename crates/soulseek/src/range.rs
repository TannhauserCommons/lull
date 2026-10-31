//! HTTP byte ranges against a file that is still growing. The file's full size is known from the
//! search result, so ranges resolve against it; what can be sent *now* is bounded by the bytes
//! received so far (docs/context/streaming.md).

/// Half-open byte span `[start, end)` of the full file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl Span {
    pub fn len(&self) -> u64 {
        self.end - self.start
    }

    /// `Content-Range` value for a 206 answer.
    pub fn content_range(&self, size: u64) -> String {
        format!("bytes {}-{}/{size}", self.start, self.end.saturating_sub(1))
    }
}

/// What the client asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// No usable `Range` header: the whole file, answered 200.
    Whole(Span),
    /// One satisfiable range, answered 206.
    Part(Span),
    /// A range that starts at or past the end of the file: 416.
    Unsatisfiable,
}

/// Resolves a `Range` header against the full size. Headers that are malformed or ask for
/// several ranges are ignored, as RFC 9110 allows, and the whole file is sent.
pub fn ask(header: Option<&str>, size: u64) -> Ask {
    let whole = Ask::Whole(Span { start: 0, end: size });
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return whole;
    };
    if spec.contains(',') {
        return whole;
    }
    let Some((from, to)) = spec.split_once('-') else {
        return whole;
    };
    let (from, to) = (from.trim(), to.trim());
    match (from.parse::<u64>(), to.parse::<u64>()) {
        // bytes=-N: the last N bytes.
        (Err(_), Ok(n)) if from.is_empty() => match n.min(size) {
            0 => Ask::Unsatisfiable,
            n => Ask::Part(Span {
                start: size - n,
                end: size,
            }),
        },
        (Ok(start), _) if start >= size => Ask::Unsatisfiable,
        // bytes=A-
        (Ok(start), Err(_)) if to.is_empty() => Ask::Part(Span { start, end: size }),
        // bytes=A-B (inclusive B, clamped to the file)
        (Ok(start), Ok(last)) if last >= start => Ask::Part(Span {
            start,
            end: last.saturating_add(1).min(size),
        }),
        _ => whole,
    }
}

/// Bytes of `span` that can be sent from `pos` now, given `received` bytes on disk.
pub fn sendable(span: Span, pos: u64, received: u64) -> u64 {
    span.end.min(received).saturating_sub(pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1_000_000;

    #[test]
    fn ranges_resolve_against_the_full_size_not_what_has_arrived() {
        let size = 30 * MB;
        assert_eq!(ask(None, size), Ask::Whole(Span { start: 0, end: size }));
        assert_eq!(ask(Some("bytes=0-"), size), Ask::Part(Span { start: 0, end: size }));
        assert_eq!(
            ask(Some("bytes=100-199"), size),
            Ask::Part(Span { start: 100, end: 200 })
        );
        // An end past the file is clamped; a start past it is unsatisfiable.
        assert_eq!(
            ask(Some("bytes=29999990-40000000"), size),
            Ask::Part(Span {
                start: 29_999_990,
                end: size
            })
        );
        assert_eq!(ask(Some("bytes=30000000-"), size), Ask::Unsatisfiable);
        // Suffix ranges: the tail some players read first for metadata.
        assert_eq!(
            ask(Some("bytes=-128"), size),
            Ask::Part(Span {
                start: size - 128,
                end: size
            })
        );
        assert_eq!(ask(Some("bytes=-0"), size), Ask::Unsatisfiable);
        // Malformed and multi-range headers fall back to the whole file.
        for odd in ["bytes=5-2", "bytes=abc", "items=0-1", "bytes=0-1,4-5"] {
            assert_eq!(ask(Some(odd), size), Ask::Whole(Span { start: 0, end: size }), "{odd}");
        }
    }

    #[test]
    fn only_received_bytes_are_sendable_on_a_growing_file() {
        let span = Span {
            start: 1_000,
            end: 5_000,
        };
        // Nothing of the span has arrived yet: the caller waits.
        assert_eq!(sendable(span, 1_000, 800), 0);
        // Part of it has: send up to what's on disk, never beyond.
        assert_eq!(sendable(span, 1_000, 3_000), 2_000);
        assert_eq!(sendable(span, 2_500, 3_000), 500);
        // The file grew past the span: stop at the span's end.
        assert_eq!(sendable(span, 4_000, 9_000), 1_000);
        assert_eq!(sendable(span, 5_000, 9_000), 0);
    }
}
