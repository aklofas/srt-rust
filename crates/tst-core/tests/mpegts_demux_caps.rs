//! Demuxer resource-cap regression tests.
//!
//! Phase 0: validates that hostile input cannot grow `Demuxer::sync_buf`
//! past the 4 MiB hard ceiling. A peer feeding bytes with no 0x47 sync
//! byte previously grew the buffer unboundedly — `extend_from_slice` ran
//! before the `SYNC_SEARCH_WINDOW` check, so a single multi-GB `feed` call
//! could OOM the host before the loop got a chance to bail.

use tst_core::error::DemuxError;
use tst_core::mpegts::demux::Demuxer;

#[test]
fn demux_rejects_unbounded_sync_buf_growth() {
    let mut dx = Demuxer::new();
    // 8 MiB of 0xFF (no sync byte 0x47 anywhere). Twice the 4 MiB cap so
    // the very first `feed` call must trip the ceiling.
    let garbage = vec![0xFFu8; 8 * 1024 * 1024];
    let result = dx.feed(&garbage);
    assert!(
        matches!(result, Err(DemuxError::SyncBufExhausted { .. })),
        "expected SyncBufExhausted, got {result:?}"
    );
}
