//! Data-stream (`StreamSpec::Data` PES pass-through) config surface via the
//! C ABI: `tst_mux_config_add_data_stream` +
//! `tst_mux_config_set_stream_descriptors_for_data` +
//! `tst_mux_config_add_data_descriptor`.
//!
//! Config level only — the `tst_muxer_push_data[_to]` push surface is
//! exercised separately. The offline `tst_mux_config_*` / `tst_muxer_*`
//! surface is unconditional, so this module carries no feature gate
//! (matching `demuxer_offline.rs`).

use tstrans::config::{
    TstMuxConfig, TstProgramHandle, TstVideoCodec, tst_mux_config_add_data_descriptor,
    tst_mux_config_add_data_stream, tst_mux_config_add_program, tst_mux_config_add_video_stream,
    tst_mux_config_free, tst_mux_config_new, tst_mux_config_set_stream_descriptors_for_data,
};
use tstrans::error::{TstError, tst_get_last_error};
use tstrans::event::TstDescriptor;
use tstrans::handle::TST_INVALID_STREAM_HANDLE;
use tstrans::muxer::{tst_muxer_close, tst_muxer_open, tst_muxer_pull, tst_muxer_push_video_to};

const NAL_SPS: &[u8] = &[
    0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x1e, 0xda, 0x02, 0x80, 0xf6, 0xc0,
];

/// Open the config, push one video frame (forces PSI emission alongside the
/// payload), pull the TS output, and close. The config survives — `_open`
/// clones the inner — so callers can mutate + reopen the same `cfg`.
unsafe fn open_push_pull(cfg: *mut TstMuxConfig, h_video: u32) -> Vec<u8> {
    unsafe {
        let mux = tst_muxer_open(cfg);
        assert!(!mux.is_null(), "tst_muxer_open failed");
        let rc = tst_muxer_push_video_to(mux, h_video, NAL_SPS.as_ptr(), NAL_SPS.len(), 0, true);
        assert_eq!(rc, 0, "push_video_to failed");
        let mut buf = vec![0u8; 64 * 188];
        let n = tst_muxer_pull(mux, buf.as_mut_ptr(), buf.len());
        assert!(n > 0, "muxer produced no output");
        buf.truncate(n);
        tst_muxer_close(mux);
        buf
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ----------------------------------------------------------------------------
// Constructor — accept paths
// ----------------------------------------------------------------------------

#[test]
fn add_data_stream_returns_distinct_valid_handles() {
    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        let h0 = tst_mux_config_add_data_stream(cfg, prog, 0x1041, 0xF0, true);
        let h1 = tst_mux_config_add_data_stream(cfg, prog, 0x1042, 0xF1, false);
        assert_ne!(h0, TST_INVALID_STREAM_HANDLE);
        assert_ne!(h1, TST_INVALID_STREAM_HANDLE);
        assert_ne!(h0, h1, "two data streams must get distinct handles");
        tst_mux_config_free(cfg);
    }
}

// ----------------------------------------------------------------------------
// Constructor — reject paths
// ----------------------------------------------------------------------------

#[test]
fn add_data_stream_null_cfg_returns_sentinel() {
    unsafe {
        let h = tst_mux_config_add_data_stream(
            core::ptr::null_mut(),
            TstProgramHandle(0),
            0x1041,
            0xF0,
            true,
        );
        assert_eq!(h, TST_INVALID_STREAM_HANDLE);
        assert_eq!(tst_get_last_error(), TstError::InvalidConfig as i32);
    }
}

#[test]
fn add_data_stream_invalid_program_returns_sentinel() {
    unsafe {
        let cfg = tst_mux_config_new();
        // No programs added — TstProgramHandle(0) is invalid.
        let h = tst_mux_config_add_data_stream(cfg, TstProgramHandle(0), 0x1041, 0xF0, true);
        assert_eq!(h, TST_INVALID_STREAM_HANDLE);
        assert_eq!(tst_get_last_error(), TstError::InvalidUsage as i32);
        tst_mux_config_free(cfg);
    }
}

#[test]
fn add_data_stream_17th_exceeds_per_program_cap() {
    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        for i in 0..16u16 {
            let h = tst_mux_config_add_data_stream(cfg, prog, 0x1100 + i, 0xF0, true);
            assert_ne!(
                h, TST_INVALID_STREAM_HANDLE,
                "stream {i} should be accepted"
            );
        }
        let h = tst_mux_config_add_data_stream(cfg, prog, 0x1110, 0xF0, true);
        assert_eq!(
            h, TST_INVALID_STREAM_HANDLE,
            "17th data stream must be rejected"
        );
        assert_eq!(tst_get_last_error(), TstError::InvalidUsage as i32);
        tst_mux_config_free(cfg);
    }
}

// ----------------------------------------------------------------------------
// Descriptors — set / clear / add round-trip (observed through PMT bytes)
// ----------------------------------------------------------------------------

#[test]
fn set_stream_descriptors_for_data_set_then_clear_roundtrip() {
    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        let h_video = tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
        let h_data = tst_mux_config_add_data_stream(cfg, prog, 0x1041, 0xF0, true);
        assert_ne!(h_video, TST_INVALID_STREAM_HANDLE);
        assert_ne!(h_data, TST_INVALID_STREAM_HANDLE);

        // One user-private descriptor TLV (tag 0xA0, 4-byte body). A
        // user-private tag never trips the validate-time classify-Unknown
        // rule on data streams.
        let tlv: &[u8] = &[0xA0, 0x04, 0xDE, 0xAD, 0xBE, 0xEF];
        let rc =
            tst_mux_config_set_stream_descriptors_for_data(cfg, h_data, tlv.as_ptr(), tlv.len(), 1);
        assert_eq!(rc, 0);
        let ts = open_push_pull(cfg, h_video);
        assert!(
            contains(&ts, tlv),
            "PMT must carry the data-stream descriptor TLV"
        );

        // Clearing (len 0 / count 0) removes the descriptor on reopen.
        let rc =
            tst_mux_config_set_stream_descriptors_for_data(cfg, h_data, core::ptr::null(), 0, 0);
        assert_eq!(rc, 0);
        let ts = open_push_pull(cfg, h_video);
        assert!(
            !contains(&ts, tlv),
            "cleared descriptor must not appear in PMT"
        );

        tst_mux_config_free(cfg);
    }
}

#[test]
fn add_data_descriptor_accumulates() {
    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        let h_video = tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
        let h_data = tst_mux_config_add_data_stream(cfg, prog, 0x1041, 0xF0, true);
        assert_ne!(h_data, TST_INVALID_STREAM_HANDLE);

        let body_a: &[u8] = &[0x01, 0x02, 0x03];
        let desc_a = TstDescriptor {
            tag: 0xA1,
            _reserved: [0; 7],
            data: body_a.as_ptr(),
            data_len: body_a.len(),
        };
        let body_b: &[u8] = &[0x44];
        let desc_b = TstDescriptor {
            tag: 0xA2,
            _reserved: [0; 7],
            data: body_b.as_ptr(),
            data_len: body_b.len(),
        };
        assert_eq!(tst_mux_config_add_data_descriptor(cfg, h_data, &desc_a), 0);
        assert_eq!(tst_mux_config_add_data_descriptor(cfg, h_data, &desc_b), 0);

        let ts = open_push_pull(cfg, h_video);
        assert!(
            contains(&ts, &[0xA1, 0x03, 0x01, 0x02, 0x03]),
            "first added descriptor must appear in PMT"
        );
        assert!(
            contains(&ts, &[0xA2, 0x01, 0x44]),
            "second added descriptor must accumulate, not replace"
        );

        tst_mux_config_free(cfg);
    }
}

// ----------------------------------------------------------------------------
// Descriptors — forged-handle rejection (trust-boundary validation)
// ----------------------------------------------------------------------------

#[test]
fn descriptor_functions_reject_forged_high_bit_handle() {
    // Same threat model as `muxer_push_video_to_forged_high_bit_handle_*`
    // in multi_stream.rs: a raw handle with bits set above the canonical
    // 8-bit packed layout must be rejected — not silently aliased onto the
    // genuine stream.
    unsafe {
        let cfg = tst_mux_config_new();
        let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
        let h_data = tst_mux_config_add_data_stream(cfg, prog, 0x1041, 0xF0, true);
        assert_ne!(h_data, TST_INVALID_STREAM_HANDLE);
        let forged = h_data | 0x100;

        let tlv: &[u8] = &[0xA0, 0x01, 0x55];
        let rc =
            tst_mux_config_set_stream_descriptors_for_data(cfg, forged, tlv.as_ptr(), tlv.len(), 1);
        assert_eq!(rc, TstError::InvalidUsage as i32);

        let body: &[u8] = &[0x55];
        let desc = TstDescriptor {
            tag: 0xA0,
            _reserved: [0; 7],
            data: body.as_ptr(),
            data_len: body.len(),
        };
        let rc = tst_mux_config_add_data_descriptor(cfg, forged, &desc);
        assert_eq!(rc, TstError::InvalidUsage as i32);

        tst_mux_config_free(cfg);
    }
}
