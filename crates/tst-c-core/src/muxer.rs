//! `tst_muxer_t` — standalone MPEG-TS muxer utility.
//!
//! Wraps `tst_core::mpegts::mux::Muxer`. No transport — push NALs and KLV,
//! pull TS bytes. The handle is internally synchronized; push_video,
//! push_klv, and pull may be called from different threads.

use crate::config::TstMuxConfig;
use crate::error::{TstError, record_mux_error, record_not_found, set_last_error};
use crate::handle::{
    Handle, TstAudioStreamHandle, TstKlvStreamHandle, TstSubtitleStreamHandle, TstVideoStreamHandle,
};
use alloc::boxed::Box;
use alloc::format;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, Muxer, SubtitleStreamHandle, VideoStreamHandle,
};

pub struct TstMuxer {
    inner: Handle<Muxer>,
}

/// Open a standalone muxer. Builds the config from `cfg` so the caller may
/// free it immediately after this returns. Returns NULL on failure with
/// last-error set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_open(cfg: *mut TstMuxConfig) -> *mut TstMuxer {
    crate::panic::ffi_catch(core::ptr::null_mut(), || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return core::ptr::null_mut();
        };
        let built = match cfg.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return core::ptr::null_mut();
            }
        };
        let muxer = match Muxer::new(built) {
            Ok(m) => m,
            Err(e) => {
                record_mux_error(&e);
                return core::ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TstMuxer {
            inner: Handle::new(muxer),
        }))
    })
}

/// Push one Annex-B-framed video access unit. Returns 0 on success or a
/// negative TST_E_* code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_video(
    p: *mut TstMuxer,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(nal, len, "nal") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_mut(|m| match m.push_video(slice, pts, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                // Find the matching TST_E_* code via the recorded last-error.
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one KLV blob onto the muxer's single KLV stream.
///
/// `klv` must point to **raw MISB Local Set bytes**. For streams configured
/// as `TST_KLV_STREAM_TYPE_SYNCHRONOUS_METADATA`, the muxer prepends a
/// 5-byte `Metadata_AU_cell` header per ITU-T H.222.0 V9 §2.12.4.2 before
/// emitting. **Do not pre-wrap the AU cell on the caller side** —
/// double-wrapping produces metadata that receivers cannot parse. For
/// streams configured as `TST_KLV_STREAM_TYPE_PRIVATE_DATA`, the payload
/// is emitted as-is.
///
/// `pts_90khz` is the presentation timestamp in 90 kHz ticks, lives in
/// the PES header, and is the same value pulled by demux-side consumers.
///
/// Single-stream form: the muxer must have exactly one KLV stream
/// configured. Multi-stream callers use `tst_muxer_push_klv_to` with an
/// explicit `TstKlvStreamHandle`.
///
/// # Errors
///
/// - `TST_E_INVALID_USAGE` — no KLV stream configured (`NoKlvStreamsConfigured`).
/// - `TST_E_INVALID_USAGE` — more than one KLV stream configured
///   (`AmbiguousTarget` — caller must use `_to` variant).
/// - `TST_E_KLV_TOO_LARGE` — payload exceeds the per-frame KLV size limit
///   (`KlvTooLarge`).
/// - `TST_E_INVALID_CONFIG` — `klv` is null with non-zero `len`.
///
/// # C ABI
///
/// `tst_muxer_push_klv` — see `crates/tst-c/include/tstrans.h`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_klv(
    p: *mut TstMuxer,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(klv, len, "klv") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle.inner.with_inner_mut(|m| {
        match m.push_klv(
            slice, pts,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        }
    })
}

/// Push one Annex-B NAL targeting a specific video elementary stream.
///
/// `handle` is obtained from `tst_mux_config_add_video_stream` at config
/// time and is stable across managed-sender reconnects. Out-of-range
/// handles surface as `TST_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream muxer, prefer `tst_muxer_push_video` — it has the
/// same effect and doesn't require a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_video_to(
    p: *mut TstMuxer,
    handle: TstVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> crate::c_types::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(nal, len, "nal") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Trust-boundary validation: reject any caller-provided u32 that has
    // bits set outside the canonical 8-bit packed layout. Without this,
    // `valid.raw() | 0x100` would mask down to the valid low byte and
    // alias the wrong elementary stream — the push-time range check
    // sees only the masked indices and cannot distinguish.
    let stream = match VideoStreamHandle::try_from_raw(handle) {
        Ok(h) => h,
        Err(e) => {
            record_mux_error(&e);
            return unsafe { crate::error::tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    h.inner
        .with_inner_mut(|m| match m.push_video_to(stream, slice, pts, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one pre-built KLV blob targeting a specific KLV elementary stream.
///
/// `handle` is obtained from `tst_mux_config_add_klv_stream`. Same
/// semantics as `tst_muxer_push_video_to`.
///
/// For `KlvStreamType::SynchronousMetadata` streams, the muxer auto-wraps
/// the caller's bytes in a `Metadata_AU_cell` header per ITU-T H.222.0
/// V9 § 2.12.4.2 (5 bytes prepended; PTS surfaced in the PES header).
/// For `KlvStreamType::PrivateData` streams, the caller's bytes pass
/// through unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_klv_to(
    p: *mut TstMuxer,
    handle: TstKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(klv, len, "klv") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Trust-boundary validation — see VideoStreamHandle::try_from_raw rationale
    // in tst_muxer_push_video_to above.
    let stream = match KlvStreamHandle::try_from_raw(handle) {
        Ok(h) => h,
        Err(e) => {
            record_mux_error(&e);
            return unsafe { crate::error::tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    h.inner.with_inner_mut(|m| {
        match m.push_klv_to(
            stream, slice, pts,
            // C ABI receiver-surface plan will expose metadata_service_id;
            // today defaults to 0x00 per ST 1402.2 App. B Table 2.
            0x00,
        ) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        }
    })
}

/// Push one audio frame buffer (single-stream shorthand).
///
/// Resolves only when exactly one audio stream is configured across all
/// programs. Otherwise rejects with `TST_E_INVALID_USAGE` (carrying
/// `MuxError::AmbiguousTarget` or `MuxError::NoAudioStreamsConfigured`).
///
/// `frames` is one or more pre-framed audio frames concatenated by the
/// caller (e.g. one ADTS frame for AAC, one MP2 frame, one AC-3 frame).
/// PTS is required — audio always carries PTS.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_audio(
    p: *mut TstMuxer,
    frames: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(frames, len, "frames") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_mut(|m| match m.push_audio(slice, pts) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one audio frame buffer targeting a specific audio elementary stream.
///
/// `handle` is obtained from `tst_mux_config_add_audio_stream` /
/// `tst_mux_config_add_audio_stream_with_language` at config time and is
/// stable across the config→open boundary. Out-of-range handles surface
/// as `TST_E_INVALID_USAGE` (carrying `MuxError::InvalidStreamHandle`).
///
/// On a single-stream muxer, prefer `tst_muxer_push_audio` — it has the
/// same effect and doesn't require a handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_audio_to(
    p: *mut TstMuxer,
    handle: TstAudioStreamHandle,
    frames: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(frames, len, "frames") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Trust-boundary validation — see VideoStreamHandle::try_from_raw rationale
    // in tst_muxer_push_video_to above.
    let stream = match AudioStreamHandle::try_from_raw(handle) {
        Ok(h) => h,
        Err(e) => {
            record_mux_error(&e);
            return unsafe { crate::error::tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    h.inner
        .with_inner_mut(|m| match m.push_audio_to(stream, pts, slice) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one subtitle PES unit (single-stream shorthand).
///
/// Resolves only when exactly one subtitle stream is configured.
/// Otherwise rejects with `TST_E_INVALID_USAGE` (carrying
/// `MuxError::AmbiguousTarget` or `MuxError::NoSubtitleStreamsConfigured`).
///
/// `payload` is one complete logical subtitle unit (DVB-sub composition
/// page, teletext data field, CEA-708 service block, or WebVTT cue);
/// fragmentation across PES is not used. PTS is required — subtitles
/// are rendered at presentation time and never reordered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_subtitle(
    p: *mut TstMuxer,
    payload: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(payload, len, "payload") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_mut(|m| match m.push_subtitle(pts, slice) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Push one subtitle PES unit targeting a specific subtitle elementary stream.
///
/// `handle` is obtained from one of the four
/// `tst_mux_config_add_subtitle_stream_*` constructors. Out-of-range
/// handles surface as `TST_E_INVALID_USAGE` (carrying
/// `MuxError::InvalidStreamHandle`).
///
/// On a single-stream muxer, prefer `tst_muxer_push_subtitle` — same
/// effect, no handle required.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_push_subtitle_to(
    p: *mut TstMuxer,
    handle: TstSubtitleStreamHandle,
    payload: *const u8,
    len: usize,
    pts_90khz: i64,
) -> crate::c_types::c_int {
    let Some(h) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(payload, len, "payload") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Trust-boundary validation — see VideoStreamHandle::try_from_raw rationale
    // in tst_muxer_push_video_to above.
    let stream = match SubtitleStreamHandle::try_from_raw(handle) {
        Ok(h) => h,
        Err(e) => {
            record_mux_error(&e);
            return unsafe { crate::error::tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    h.inner
        .with_inner_mut(|m| match m.push_subtitle_to(stream, pts, slice) {
            Ok(()) => 0,
            Err(e) => {
                record_mux_error(&e);
                unsafe { crate::error::tst_get_last_error() }
            }
        })
}

/// Drain TS bytes into `out_buf` (capacity `out_cap`). Returns the number
/// of bytes written; 0 means nothing was ready or the buffer was too
/// small for the next chunk. Never sets last-error — 0 is a normal return
/// value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_pull(
    p: *mut TstMuxer,
    out_buf: *mut u8,
    out_cap: usize,
) -> usize {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        return 0;
    };
    if out_buf.is_null() || out_cap == 0 {
        return 0;
    }
    let slice = unsafe { core::slice::from_raw_parts_mut(out_buf, out_cap) };
    let mut n = 0usize;
    let _rc = handle.inner.with_inner_mut(|m| {
        n = m.pull(slice);
        0
    });
    n
}

/// Snapshot stats for a `tst_muxer_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the muxer has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_get_stats(
    p: *mut TstMuxer,
    out: *mut crate::stats::TstMuxerStats,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|m| {
        let stats = m.stats();
        let mut per_stream =
            [crate::stats::TstStreamStats::default(); crate::stats::TST_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::TstMuxerStats {
            ts_packets_emitted: stats.ts_packets_emitted,
            ts_bytes_emitted: stats.ts_bytes_emitted,
            programs_configured: stats.programs_configured,
            per_stream_count,
            per_stream_truncated: if truncated { 1 } else { 0 },
            per_stream,
        };
        unsafe { *out = dst };
        0
    })
}

/// Snapshot codec-specific stats for one PID on a `tst_muxer_t` into `*out`.
///
/// The returned struct is a tagged union — read `out->kind` first, then
/// the matching `out->u.<arm>` field. See `tst_stream_codec_stats_t` in
/// `tstrans.h` for the discriminator constants (`TST_CODEC_KIND_*`).
///
/// # Errors
///
/// * `TST_E_INVALID_CONFIG` — `p` or `out` is null
/// * `TST_E_CLOSED` — handle was closed via `tst_muxer_close`
/// * `TST_E_NOT_FOUND` — `pid` has never been observed on this handle
/// * `TST_E_INTERNAL` — internal panic caught at the FFI boundary
///
/// # Safety
///
/// `p` must be a valid pointer obtained from `tst_muxer_open`; `out`
/// must be a writable `tst_stream_codec_stats_t` of size at least
/// `sizeof(tst_stream_codec_stats_t)`. The pointee is fully written on
/// `TST_OK` and untouched on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_get_stream_codec_stats(
    p: *mut TstMuxer,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle
        .inner
        .with_inner_ref(|m| match m.stream_codec_stats(pid) {
            Some(stats) => {
                unsafe { *out = crate::stats::codec_stats_to_c(stats) };
                0
            }
            None => record_not_found(&format!(
                "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this muxer)"
            )),
        })
}

/// Reset stats counters for a `tst_muxer_t` to zero.
///
/// Per-stream entries are preserved (identity fields remain); only flow
/// counters (`items`, `bytes`, `discontinuities`) are zeroed.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, or `TST_E_CLOSED` if the muxer has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_reset_stats(p: *mut TstMuxer) -> crate::c_types::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null muxer pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_mut(|m| {
        m.reset_stats();
        0
    })
}

/// Close and free the muxer.
///
/// Safe to call with NULL (no-op). After this call the pointer is
/// invalid; passing the same non-null pointer twice is undefined
/// behavior (use-after-free on the consumed `Box`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_muxer_close(p: *mut TstMuxer) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.inner.close();
        drop(boxed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn open_with_default_config_succeeds() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            assert!(!m.is_null());
            tst_muxer_close(m);
            tst_mux_config_free(cfg);
        }
    }

    #[test]
    fn push_then_pull_emits_bytes() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            // Single-stream muxer: push_video (no handle) is unambiguous.
            let hv = tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            tst_mux_config_free(cfg);

            // Annex-B IDR NAL.
            let nal: [u8; 9] = [0, 0, 0, 1, 0x65, 0xAA, 0xAA, 0xAA, 0xAA];
            let rc = tst_muxer_push_video_to(m, hv, nal.as_ptr(), nal.len(), 0, true);
            assert_eq!(rc, 0);

            let mut buf = vec![0u8; 4096];
            let n = tst_muxer_pull(m, buf.as_mut_ptr(), buf.len());
            assert!(n > 0, "expected TS bytes to be pulled");
            assert_eq!(buf[0], 0x47, "first byte of TS packet must be 0x47");

            tst_muxer_close(m);
        }
    }

    #[test]
    fn push_video_with_invalid_nal_returns_invalid_nal() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            tst_mux_config_add_klv_stream(cfg, prog, 0x1031, TstKlvStreamType::PrivateData, false);
            let m = tst_muxer_open(cfg);
            tst_mux_config_free(cfg);

            let bad = [0xAB, 0xCD];
            let rc = tst_muxer_push_video(m, bad.as_ptr(), bad.len(), 0, false);
            assert_eq!(rc, TstError::InvalidNal as i32);

            tst_muxer_close(m);
        }
    }

    #[test]
    fn null_pointer_close_is_safe() {
        unsafe { tst_muxer_close(core::ptr::null_mut()) };
    }
}
