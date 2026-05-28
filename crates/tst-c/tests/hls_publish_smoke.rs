//! Smoke test for the HLS publisher C ABI surface.
//!
//! Builds an HLS publisher on an ephemeral loopback port writing to a
//! `tempfile::tempdir()`, pushes ~100 NULL MPEG-TS packets through the
//! universal `tst_publisher_push_ts`, cuts a segment, finishes, and
//! asserts that a `.ts` segment + the `.m3u8` playlist landed on disk.
//!
//! Gated on `feature = "hls"` so it compiles only in builds that include
//! the HLS publisher. The lib name for `tst-c` is `tstrans` (see
//! `[lib] name` in Cargo.toml), so this references it as `tstrans`.
#![cfg(feature = "hls")]

use std::ffi::CString;

use tstrans::error::{TstError, tst_get_last_error};
use tstrans::hls::{
    TstPublisherKind, tst_hls_publisher_builder_bind, tst_hls_publisher_builder_build,
    tst_hls_publisher_builder_new, tst_hls_publisher_builder_output_dir,
    tst_hls_publisher_builder_segment_duration_ms, tst_publisher_cut_segment, tst_publisher_finish,
    tst_publisher_free, tst_publisher_kind, tst_publisher_push_ts,
};

const TS_PACKET_SIZE: usize = 188;

/// One MPEG-TS null packet (PID 0x1FFF, payload-only). Aligned to 188.
fn null_ts_packet() -> [u8; TS_PACKET_SIZE] {
    let mut p = [0xFFu8; TS_PACKET_SIZE];
    p[0] = 0x47; // sync byte
    p[1] = 0x1F; // null PID high 5 bits
    p[2] = 0xFF; // null PID low 8 bits
    p[3] = 0x10; // adaptation_field_control=01 (payload only)
    p
}

#[test]
fn hls_publish_round_trip() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let out_dir = dir.path().to_str().expect("tempdir path is utf8");

    // ── Build the publisher ────────────────────────────────────────────
    let builder = unsafe { tst_hls_publisher_builder_new() };
    assert!(!builder.is_null(), "builder_new returned null");

    let bind = CString::new("127.0.0.1:0").unwrap();
    assert_eq!(
        unsafe { tst_hls_publisher_builder_bind(builder, bind.as_ptr()) },
        0,
        "bind failed: {}",
        unsafe { tst_get_last_error() }
    );

    let out_c = CString::new(out_dir).unwrap();
    assert_eq!(
        unsafe { tst_hls_publisher_builder_output_dir(builder, out_c.as_ptr()) },
        0
    );

    assert_eq!(
        unsafe { tst_hls_publisher_builder_segment_duration_ms(builder, 1000) },
        0
    );

    let publisher = unsafe { tst_hls_publisher_builder_build(builder) };
    assert!(
        !publisher.is_null(),
        "build returned null: code {}",
        unsafe { tst_get_last_error() }
    );

    // Kind discriminator.
    assert_eq!(
        unsafe { tst_publisher_kind(publisher) },
        TstPublisherKind::Hls as u32
    );

    // ── Push ~100 TS packets ───────────────────────────────────────────
    let pkt = null_ts_packet();
    for i in 0..100 {
        let rc = unsafe { tst_publisher_push_ts(publisher, pkt.as_ptr(), pkt.len()) };
        assert_eq!(rc, 0, "push_ts[{i}] failed: code {}", unsafe {
            tst_get_last_error()
        });
    }

    // ── Cut a segment, then finish ─────────────────────────────────────
    assert_eq!(unsafe { tst_publisher_cut_segment(publisher) }, 0);
    assert_eq!(
        unsafe { tst_publisher_finish(publisher) },
        0,
        "finish failed: code {}",
        unsafe { tst_get_last_error() }
    );

    // After finish, push must report HLS_FINISHED.
    let rc = unsafe { tst_publisher_push_ts(publisher, pkt.as_ptr(), pkt.len()) };
    assert_eq!(rc, TstError::HlsFinished as i32);

    // ── Assert on-disk artifacts ───────────────────────────────────────
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read tempdir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    assert!(
        entries.iter().any(|n| n.ends_with(".ts")),
        "expected a .ts segment in {entries:?}"
    );
    assert!(
        entries.iter().any(|n| n.ends_with(".m3u8")),
        "expected an .m3u8 playlist in {entries:?}"
    );

    // ── Free ───────────────────────────────────────────────────────────
    unsafe { tst_publisher_free(publisher) };
}
