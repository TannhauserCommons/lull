//! Search led by the listener's own music (ADR-0036, behind the `personal-search` switch).

// The pieces land one at a time; `mod.rs` wires them (the commit that adds it drops this).
#![cfg_attr(not(test), allow(dead_code))]

pub mod index;
pub mod score;
