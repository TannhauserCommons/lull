//! The OpenSubsonic API at `/rest/*`, over the `Library` and `Collection` seams, so existing
//! Subsonic apps work with lull (docs/context/subsonic.md, ADR-0002 for app passwords).
//! Built by the Subsonic session, which also adds the crate's `start` and `router` and wires
//! them in `crates/server/src/lib.rs` once `collection` implements its seam.

use lull_core::db::Migrations;

pub const MIGRATIONS: Migrations = Migrations {
    owner: "subsonic",
    steps: &[],
};
