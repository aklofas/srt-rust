#![no_main]

//! Fuzz target — end-to-end demuxer panic-freedom, now with:
//!
//! * **StrictMode coverage**: first input byte selects one of the 4 modes
//!   (Off / TimingOnly / DescriptorsOnly / Full) via bits [1:0].
//! * **cfi_tolerance**: first input byte bit [2] sets the flag.
//! * **flush coverage**: `flush()` is called after the feed loop and its
//!   output is drained, exercising `drain_partial` → `parse_complete` →
//!   `handle_complete_pes`.
//!
//! # Input layout
//!
//! ```text
//! [0]     selector byte
//!           bits [1:0] → StrictMode: 0=Off 1=TimingOnly 2=DescriptorsOnly 3=Full
//!           bit  [2]   → cfi_tolerance: 0=false 1=true
//! [1..]   MPEG-TS packet bytes forwarded to Demuxer::feed
//! ```
//!
//! StrictMode::Full causes `feed` to return early `Err(StrictRejection)` on
//! non-conformant input — that is normal and is treated as panic-freedom
//! (discarded). Only panics count as failures.

use libfuzzer_sys::fuzz_target;
use tst_core::mpegts::demux::{Demuxer, DemuxerConfig, StrictMode};

fuzz_target!(|data: &[u8]| {
    // Need at least the selector byte.
    if data.is_empty() {
        return;
    }

    let selector = data[0];
    let payload = &data[1..];

    // Derive StrictMode from bits [1:0] — all 4 variants reachable.
    let strict = match selector & 0b11 {
        0 => StrictMode::Off,
        1 => StrictMode::TimingOnly,
        2 => StrictMode::DescriptorsOnly,
        _ => StrictMode::Full, // 3
    };

    // Derive cfi_tolerance from bit [2].
    let cfi_tolerance = (selector >> 2) & 1 == 1;

    let cfg = DemuxerConfig::builder()
        .strict(strict)
        .cfi_tolerance(cfi_tolerance)
        .build();
    let mut d = Demuxer::with_config(cfg);

    // Feed arbitrary bytes; StrictRejection is a normal outcome, not a panic.
    let _ = d.feed(payload);
    while d.next_event().is_some() {}

    // Call flush() to exercise drain_partial → parse_complete →
    // handle_complete_pes on any partial PES that was buffered during feed.
    // flush() is infallible — it must never panic.
    d.flush();
    while d.next_event().is_some() {}
});
