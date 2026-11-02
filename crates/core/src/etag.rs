//! Entity tags for answers made from bytes in memory (covers), so a client that already has
//! the bytes gets a bodyless 304: the hub answers `If-None-Match` for every route that sets an
//! `ETag` (`server::conditional`).

use std::hash::{DefaultHasher, Hasher};

/// A strong ETag of `bytes`, quoted. The hash may change with a Rust release, which costs one
/// download per picture, never a wrong 304 within a running hub.
pub fn of(bytes: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    h.write(bytes);
    format!("\"{:016x}-{:x}\"", h.finish(), bytes.len())
}
