//! C ABI: Annex B <-> length-prefixed conversion + parameter-set
//! extraction (Task 8, ABI 21).
//!
//! Exercises `tst_annexb_to_length_prefixed` / `tst_param_sets_*`
//! through the public `tstrans::codec_framing` re-export — the same
//! crate-external-caller path a real C consumer goes through (as
//! opposed to `bindings/c/core/src/codec_framing.rs`'s own `#[cfg(test)]`
//! unit tests, if any, which run in-crate). Unconditional module (no
//! `srt`/`rtp` feature gate needed), so this file carries no
//! `#![cfg(feature = ...)]`.

use std::ffi::CStr;

use tstrans::codec_framing::{
    tst_annexb_to_length_prefixed, tst_param_sets_count, tst_param_sets_extract,
    tst_param_sets_free, tst_param_sets_get,
};
use tstrans::error::{TstError, tst_get_last_error, tst_get_last_error_str};

const TST_VIDEO_CODEC_H264: i32 = 0;

fn last_error_msg() -> String {
    unsafe {
        CStr::from_ptr(tst_get_last_error_str())
            .to_str()
            .unwrap()
            .to_owned()
    }
}

/// Hand-built Annex B access unit: SPS (0x67) + PPS (0x68) + IDR (0x65),
/// each delimited by a 4-byte start code.
fn sps_pps_idr_annexb() -> Vec<u8> {
    vec![
        0x00, 0x00, 0x00, 0x01, // start code
        0x67, 0xAA, 0xBB, 0xCC, 0xDD, // SPS: header + 4 payload bytes (len 5)
        0x00, 0x00, 0x00, 0x01, // start code
        0x68, 0xEE, 0xFF, // PPS: header + 2 payload bytes (len 3)
        0x00, 0x00, 0x00, 0x01, // start code
        0x65, 0x11, 0x22, 0x33, // IDR slice: header + 3 payload bytes (len 4)
    ]
}

#[test]
fn annexb_to_length_prefixed_buffer_too_small_reports_needed_size_without_writing() {
    let annexb = sps_pps_idr_annexb();
    // Expected output: 3 NALs, each a 4-byte BE length + NAL bytes:
    // (4+5) + (4+3) + (4+4) = 9 + 7 + 8 = 24 bytes.
    let expected_needed: usize = 24;

    unsafe {
        let mut out_len: usize = 0xDEAD_BEEF; // sentinel — must be overwritten
        let mut out_buf = [0u8; 4]; // deliberately too small
        let rc = tst_annexb_to_length_prefixed(
            annexb.as_ptr(),
            annexb.len(),
            4,
            out_buf.as_mut_ptr(),
            out_buf.len(),
            &mut out_len,
        );
        assert_eq!(rc, TstError::BufferFull as i32, "{}", last_error_msg());
        assert_eq!(out_len, expected_needed);
        // Buffer must be untouched on the query/too-small path.
        assert_eq!(out_buf, [0u8; 4]);

        // NULL out with a nonzero out_cap also takes the query path.
        let mut out_len2: usize = 0;
        let rc2 = tst_annexb_to_length_prefixed(
            annexb.as_ptr(),
            annexb.len(),
            4,
            std::ptr::null_mut(),
            1000,
            &mut out_len2,
        );
        assert_eq!(rc2, TstError::BufferFull as i32);
        assert_eq!(out_len2, expected_needed);
    }
}

#[test]
fn annexb_to_length_prefixed_succeeds_with_right_sized_buffer() {
    let annexb = sps_pps_idr_annexb();

    unsafe {
        let mut out_len: usize = 0;
        let mut out_buf = [0u8; 64];
        let rc = tst_annexb_to_length_prefixed(
            annexb.as_ptr(),
            annexb.len(),
            4,
            out_buf.as_mut_ptr(),
            out_buf.len(),
            &mut out_len,
        );
        assert_eq!(rc, 0, "{}", last_error_msg());
        assert_eq!(out_len, 24);

        // First NAL: 4-byte BE length of the SPS NAL (5 bytes), then the
        // SPS bytes themselves (header 0x67 first).
        assert_eq!(&out_buf[0..4], &[0x00, 0x00, 0x00, 0x05]);
        assert_eq!(out_buf[4], 0x67);
        assert_eq!(&out_buf[4..9], &[0x67, 0xAA, 0xBB, 0xCC, 0xDD]);

        // Second NAL (PPS): length 3, header 0x68.
        assert_eq!(&out_buf[9..13], &[0x00, 0x00, 0x00, 0x03]);
        assert_eq!(out_buf[13], 0x68);

        // Third NAL (IDR): length 4, header 0x65.
        assert_eq!(&out_buf[16..20], &[0x00, 0x00, 0x00, 0x04]);
        assert_eq!(out_buf[20], 0x65);
    }
}

#[test]
fn annexb_to_length_prefixed_rejects_invalid_length_size() {
    let annexb = sps_pps_idr_annexb();
    unsafe {
        let mut out_len: usize = 0;
        let mut out_buf = [0u8; 64];
        let rc = tst_annexb_to_length_prefixed(
            annexb.as_ptr(),
            annexb.len(),
            3, // only 1/2/4 are valid
            out_buf.as_mut_ptr(),
            out_buf.len(),
            &mut out_len,
        );
        assert_eq!(rc, TstError::InvalidConfig as i32);
        assert_eq!(tst_get_last_error(), TstError::InvalidConfig as i32);
    }
}

#[test]
fn annexb_to_length_prefixed_null_out_len_is_rejected() {
    let annexb = sps_pps_idr_annexb();
    unsafe {
        let mut out_buf = [0u8; 64];
        let rc = tst_annexb_to_length_prefixed(
            annexb.as_ptr(),
            annexb.len(),
            4,
            out_buf.as_mut_ptr(),
            out_buf.len(),
            std::ptr::null_mut(),
        );
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }
}

#[test]
fn param_sets_extract_h264_counts_and_gets_sps_with_header_byte() {
    let annexb = sps_pps_idr_annexb();

    unsafe {
        let p = tst_param_sets_extract(annexb.as_ptr(), annexb.len(), TST_VIDEO_CODEC_H264);
        assert!(!p.is_null(), "{}", last_error_msg());

        assert_eq!(tst_param_sets_count(p, 0 /* vps */), 0);
        assert_eq!(tst_param_sets_count(p, 1 /* sps */), 1);
        assert_eq!(tst_param_sets_count(p, 2 /* pps */), 1);

        let mut out_ptr: *const u8 = std::ptr::null();
        let mut out_len: usize = 0;
        let rc = tst_param_sets_get(p, 1, 0, &mut out_ptr, &mut out_len);
        assert_eq!(rc, 0, "{}", last_error_msg());
        assert_eq!(out_len, 5);
        let sps_bytes = std::slice::from_raw_parts(out_ptr, out_len);
        assert_eq!(sps_bytes, &[0x67, 0xAA, 0xBB, 0xCC, 0xDD]);

        // idx out of range for a valid bucket -> NOT_FOUND.
        let rc_oob = tst_param_sets_get(p, 1, 5, &mut out_ptr, &mut out_len);
        assert_eq!(rc_oob, TstError::NotFound as i32);

        // which out of range -> INVALID_CONFIG (distinct from idx-range).
        let rc_bad_which = tst_param_sets_get(p, 7, 0, &mut out_ptr, &mut out_len);
        assert_eq!(rc_bad_which, TstError::InvalidConfig as i32);

        tst_param_sets_free(p);
    }
}

#[test]
fn param_sets_extract_rejects_unrecognized_codec() {
    let annexb = sps_pps_idr_annexb();
    unsafe {
        let p = tst_param_sets_extract(annexb.as_ptr(), annexb.len(), 99);
        assert!(p.is_null());
        assert_eq!(tst_get_last_error(), TstError::InvalidConfig as i32);
    }
}

#[test]
fn param_sets_count_out_of_range_which_returns_zero_no_error() {
    let annexb = sps_pps_idr_annexb();
    unsafe {
        let p = tst_param_sets_extract(annexb.as_ptr(), annexb.len(), TST_VIDEO_CODEC_H264);
        assert!(!p.is_null());
        assert_eq!(tst_param_sets_count(p, 7), 0);
        assert_eq!(tst_param_sets_count(std::ptr::null(), 1), 0);
        tst_param_sets_free(p);
    }
}

#[test]
fn param_sets_free_is_null_safe() {
    unsafe {
        tst_param_sets_free(std::ptr::null_mut());
    }
}
