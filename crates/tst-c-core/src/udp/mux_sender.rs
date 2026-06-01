//! `TstUdpMuxSender` handle type and data-path entry points.
//!
//! Open a UDP-backed `MuxSender` with `tst_udp_mux_sender_open`.
//! Push encoded video/KLV/audio/subtitle with the `push_*` family.
//! Free with `tst_udp_mux_sender_close`.
//!
//! Pattern mirrors `crates/tst-c/src/rtp/mux_sender.rs` exactly — error
//! mapping, `ffi_catch` wrapping, `Handle::with_inner_ref`, and
//! `try_from_raw` trust-boundary validation are identical.
//!
//! **No cancel:** the UDP transport does not expose a `cancel_handle()`,
//! so there is no `tst_udp_mux_sender_cancel` entry point and no cancel /
//! `was_cancelled` side-channel. `_close` simply drops the handle. To
//! unblock a thread parked in a `_push_*` call, close the handle from the
//! same thread (or rely on the socket's send-side behavior).

use std::os::raw::c_char;

use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    AudioStreamHandle, KlvStreamHandle, SubtitleStreamHandle, VideoStreamHandle,
};
use tst_pipeline::MuxSender;
use tst_udp::{UdpTransport, UdpTransportBuilder};

use crate::config::TstMuxConfig;
use crate::error::{
    TstError, record_mux_error, record_not_available, record_not_found, record_shell_error,
    set_last_error, tst_get_last_error,
};
use crate::handle::{
    Handle, TstAudioStreamHandle, TstKlvStreamHandle, TstSubtitleStreamHandle, TstVideoStreamHandle,
};

// ---------------------------------------------------------------------------
// Handle type
// ---------------------------------------------------------------------------

/// Opaque handle for a UDP-backed mux sender.
///
/// Returned by [`tst_udp_mux_sender_open`]. Freed with
/// [`tst_udp_mux_sender_close`].
pub struct TstUdpMuxSender {
    pub(crate) inner: Handle<MuxSender<UdpTransport>>,
}

// ---------------------------------------------------------------------------
// Open
// ---------------------------------------------------------------------------

/// Open a UDP-backed `MuxSender` that muxes MPEG-TS in real time and
/// sends over UDP. `mux_cfg` must be a valid `tst_mux_config_t`
/// (constructed via `tst_mux_config_new`). Returns `NULL` on error.
///
/// The mux config is borrowed — the caller still owns it and must free
/// it. The returned handle is independent of the config after this call.
///
/// URL grammar:
/// - `udp://host:port` — unicast send
/// - `udp://group:port` (group ∈ 224.0.0.0/4 or ff00::/8) — multicast send
/// - Query params: `?ttl=N`, `?iface=eth0`, `?tos=0xb8`, `?sndbuf=2M`,
///   `?pkt_size=1316`, `?localaddr=...`
///
/// # Safety
///
/// `url` is a NUL-terminated C string. `mux_cfg` must be a non-null
/// pointer to a `tst_mux_config_t` valid for this call. The returned
/// handle must eventually be freed with `tst_udp_mux_sender_close`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_open(
    url: *const c_char,
    mux_cfg: *const TstMuxConfig,
) -> *mut TstUdpMuxSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let url_str = match unsafe { super::url::parse_url_str(url) } {
            Some(s) => s,
            None => return std::ptr::null_mut(),
        };
        let cfg_ref = match unsafe { mux_cfg.as_ref() } {
            Some(c) => c,
            None => {
                set_last_error(TstError::InvalidConfig, "mux_cfg is null");
                return std::ptr::null_mut();
            }
        };
        let built = match cfg_ref.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        let builder = match UdpTransportBuilder::from_url(url_str) {
            Ok(b) => b,
            Err(e) => {
                set_last_error(TstError::UdpConfig, &format!("udp url parse: {e}"));
                return std::ptr::null_mut();
            }
        };
        let transport = match builder.build() {
            Ok(t) => t,
            Err(e) => {
                let code = crate::error::udp_error_to_code(&e);
                set_last_error(code, &format!("udp build: {e}"));
                return std::ptr::null_mut();
            }
        };
        let mux_sender = match MuxSender::new(transport, built) {
            Ok(s) => s,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TstUdpMuxSender {
            inner: Handle::new(mux_sender),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close
// ---------------------------------------------------------------------------

/// Close and free a `tst_udp_mux_sender_t`.
///
/// Safe to call with `NULL` (no-op).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstUdpMuxSender` returned
/// by `tst_udp_mux_sender_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_close(p: *mut TstUdpMuxSender) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.inner.close();
        drop(boxed);
    });
}

// ---------------------------------------------------------------------------
// Push — single-stream variants
// ---------------------------------------------------------------------------

/// Push one Annex-B NAL through the muxer's single video stream and
/// out the UDP transport (single-stream shorthand).
///
/// `nal` must point to `len` bytes of Annex-B NAL data. `pts_90khz` is
/// the presentation timestamp in 90 kHz ticks. `key_frame` is `true`
/// for IDR / key frames (used to set the random-access indicator in the
/// MPEG-TS adaptation field).
///
/// Resolves only when exactly one video stream is configured; otherwise
/// rejects with `TST_E_INVALID_USAGE`.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `nal` must be
/// readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_video(
    p: *mut TstUdpMuxSender,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(nal, len, "nal") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_ref(|s| match s.send_video(slice, pts, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one raw KLV blob through the muxer's single KLV stream and out
/// the UDP transport (single-stream shorthand).
///
/// `klv` must point to **raw MISB Local Set bytes**. For streams
/// configured as `TST_KLV_STREAM_TYPE_SYNCHRONOUS_METADATA`, the muxer
/// prepends a 5-byte `Metadata_AU_cell` header per ITU-T H.222.0 V9
/// §2.12.4.2. **Do not pre-wrap the AU cell on the caller side.**
/// `pts_90khz` is the presentation timestamp in 90 kHz ticks.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `klv` must be
/// readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_klv(
    p: *mut TstUdpMuxSender,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(klv, len, "klv") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_ref(|s| match s.send_klv(slice, pts, 0x00) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one audio frame buffer through the muxer's single audio stream
/// and out the UDP transport (single-stream shorthand).
///
/// `frames` must point to `len` bytes of pre-framed audio data (one or
/// more ADTS frames or MPEG audio frames concatenated). `pts_90khz` is
/// the presentation timestamp in 90 kHz ticks.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `frames` must
/// be readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_audio(
    p: *mut TstUdpMuxSender,
    frames: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(frames, len, "frames") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_ref(|s| match s.send_audio(slice, pts) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one subtitle PES unit through the muxer's single subtitle stream
/// and out the UDP transport (single-stream shorthand).
///
/// `payload` is one complete logical subtitle unit. `pts_90khz` is the
/// presentation timestamp in 90 kHz ticks.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `payload` must
/// be readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_subtitle(
    p: *mut TstUdpMuxSender,
    payload: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(payload, len, "payload") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let pts = Pts90khz::new(pts_90khz);
    handle
        .inner
        .with_inner_ref(|s| match s.send_subtitle(slice, pts) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

// ---------------------------------------------------------------------------
// Push — multi-stream (_to) variants
// ---------------------------------------------------------------------------

/// Push one Annex-B NAL targeting a specific video elementary stream.
///
/// `stream_handle` is obtained from `tst_mux_config_add_video_stream` at
/// config time and is stable across the config→open boundary. Out-of-range
/// handles surface as `TST_E_INVALID_USAGE`.
///
/// On a single-stream sender, prefer `tst_udp_mux_sender_push_video` —
/// same effect, no handle required.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `nal` must be
/// readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_video_to(
    p: *mut TstUdpMuxSender,
    stream_handle: TstVideoStreamHandle,
    nal: *const u8,
    len: usize,
    pts_90khz: i64,
    key_frame: bool,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(nal, len, "nal") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // Trust-boundary validation — forged stream_handle values are rejected
    // before they reach the push-time range check (which only sees masked
    // indices and can't detect high-byte contamination).
    let stream = match VideoStreamHandle::try_from_raw(stream_handle) {
        Ok(h) => h,
        Err(e) => {
            crate::error::record_mux_error(&e);
            return unsafe { tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_video_to(stream, slice, pts, key_frame) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one KLV blob targeting a specific KLV elementary stream.
///
/// For `KlvStreamType::SynchronousMetadata` streams the muxer auto-wraps
/// the caller's bytes in a `Metadata_AU_cell` header (do not pre-wrap).
/// On a single-stream sender, prefer `tst_udp_mux_sender_push_klv`.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `klv` must be
/// readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_klv_to(
    p: *mut TstUdpMuxSender,
    stream_handle: TstKlvStreamHandle,
    klv: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(klv, len, "klv") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let stream = match KlvStreamHandle::try_from_raw(stream_handle) {
        Ok(h) => h,
        Err(e) => {
            crate::error::record_mux_error(&e);
            return unsafe { tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_klv_to(stream, slice, pts, 0x00) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one audio frame buffer targeting a specific audio elementary stream.
///
/// On a single-stream sender, prefer `tst_udp_mux_sender_push_audio`.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `frames` must
/// be readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_audio_to(
    p: *mut TstUdpMuxSender,
    stream_handle: TstAudioStreamHandle,
    frames: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(frames, len, "frames") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let stream = match AudioStreamHandle::try_from_raw(stream_handle) {
        Ok(h) => h,
        Err(e) => {
            crate::error::record_mux_error(&e);
            return unsafe { tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_audio_to(stream, slice, pts) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

/// Push one subtitle PES unit targeting a specific subtitle elementary stream.
///
/// On a single-stream sender, prefer `tst_udp_mux_sender_push_subtitle`.
///
/// # Safety
///
/// `p` must be a valid non-freed `*mut TstUdpMuxSender`. `payload` must
/// be readable for `len` bytes.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_push_subtitle_to(
    p: *mut TstUdpMuxSender,
    stream_handle: TstSubtitleStreamHandle,
    payload: *const u8,
    len: usize,
    pts_90khz: i64,
) -> libc::c_int {
    let Some(wrapper) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    let slice = match unsafe { crate::ffi_slice::ffi_slice(payload, len, "payload") } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let stream = match SubtitleStreamHandle::try_from_raw(stream_handle) {
        Ok(h) => h,
        Err(e) => {
            crate::error::record_mux_error(&e);
            return unsafe { tst_get_last_error() };
        }
    };
    let pts = Pts90khz::new(pts_90khz);
    wrapper
        .inner
        .with_inner_ref(|s| match s.send_subtitle_to(stream, slice, pts) {
            Ok(()) => 0,
            Err(e) => {
                record_shell_error(&e);
                unsafe { tst_get_last_error() }
            }
        })
}

// ---------------------------------------------------------------------------
// Stats
// ---------------------------------------------------------------------------

/// Snapshot mux-sender-level stats for a `tst_udp_mux_sender_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, or `TST_E_CLOSED` if the sender has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpMuxSender` opened via
/// `tst_udp_mux_sender_open`. `out` must point to a writable
/// `TstMuxSenderStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_get_mux_sender_stats(
    p: *mut TstUdpMuxSender,
    out: *mut crate::stats::TstMuxSenderStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| {
        let stats = s.stats();
        let mut per_stream =
            [crate::stats::TstStreamStats::default(); crate::stats::TST_STATS_MAX_STREAMS];
        let (per_stream_count, truncated) =
            crate::stats::fill_per_stream(&mut per_stream, &stats.per_stream);
        let dst = crate::stats::TstMuxSenderStats {
            bytes_sent: stats.bytes_sent,
            packets_sent: stats.packets_sent,
            pending_bytes_queued: stats.pending_bytes_queued,
            pending_chunks_queued: stats.pending_chunks_queued,
            programs_configured: stats.programs_configured,
            per_stream_count,
            per_stream_truncated: if truncated { 1 } else { 0 },
            per_stream,
        };
        unsafe { *out = dst };
        0
    })
}

/// Read wire-level transport stats for the underlying UDP socket.
///
/// `out` MUST point to a writable `TstSocketStats`; the function zeros
/// the struct on failure.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is null,
/// `TST_E_NOT_AVAILABLE` if no live stats are available, or
/// `TST_E_CLOSED` if the handle was closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpMuxSender` opened via
/// `tst_udp_mux_sender_open`. `out` must point to a writable
/// `TstSocketStats`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_get_socket_stats(
    p: *mut TstUdpMuxSender,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    unsafe { *out = crate::stats::TstSocketStats::default() };
    handle.inner.with_inner_ref(|s| match s.socket_stats() {
        Some(stats) => {
            unsafe { *out = (&stats).into() };
            0
        }
        None => record_not_available(
            "udp mux sender socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Snapshot codec-specific stats for one PID on a `tst_udp_mux_sender_t`.
///
/// The returned struct is a tagged union — read `out->kind` first, then
/// the matching `out->u.<arm>` field.
///
/// # Errors
///
/// * `TST_E_INVALID_CONFIG` — `p` or `out` is null
/// * `TST_E_CLOSED` — handle was closed
/// * `TST_E_NOT_FOUND` — `pid` has never been observed on this handle
/// * `TST_E_INTERNAL` — internal panic caught at the FFI boundary
///
/// # Safety
///
/// `p` must be a valid pointer obtained from `tst_udp_mux_sender_open`.
/// `out` must be a writable `tst_stream_codec_stats_t`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_get_stream_codec_stats(
    p: *mut TstUdpMuxSender,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|s| match s.stream_codec_stats(pid) {
        Some(stats) => {
            unsafe { *out = crate::stats::codec_stats_to_c(stats) };
            0
        }
        None => record_not_found(&format!(
            "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this udp mux sender)"
        )),
    })
}

/// Reset stats counters for a `tst_udp_mux_sender_t` to zero.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is null,
/// or `TST_E_CLOSED` if the sender has been closed.
///
/// # Safety
///
/// `p` must be a valid `*mut TstUdpMuxSender` opened via
/// `tst_udp_mux_sender_open`.
#[cfg(feature = "udp")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_udp_mux_sender_reset_stats(p: *mut TstUdpMuxSender) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null udp mux sender pointer");
        return TstError::InvalidConfig as i32;
    };
    handle.inner.with_inner_ref(|s| {
        s.reset_stats();
        0
    })
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    #[test]
    fn null_close_is_safe() {
        unsafe { tst_udp_mux_sender_close(std::ptr::null_mut()) };
    }

    #[test]
    fn null_push_video_returns_invalid_config() {
        let nal = [0u8; 4];
        let rc = unsafe {
            tst_udp_mux_sender_push_video(std::ptr::null_mut(), nal.as_ptr(), nal.len(), 0, false)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_push_klv_returns_invalid_config() {
        let klv = [0u8; 4];
        let rc = unsafe {
            tst_udp_mux_sender_push_klv(std::ptr::null_mut(), klv.as_ptr(), klv.len(), 0)
        };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_get_mux_sender_stats_returns_invalid_config() {
        let mut stats = crate::stats::TstMuxSenderStats::default();
        let rc =
            unsafe { tst_udp_mux_sender_get_mux_sender_stats(std::ptr::null_mut(), &mut stats) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn null_reset_stats_returns_invalid_config() {
        let rc = unsafe { tst_udp_mux_sender_reset_stats(std::ptr::null_mut()) };
        assert_eq!(rc, TstError::InvalidConfig as i32);
    }

    #[test]
    fn open_with_null_url_returns_null() {
        unsafe {
            let cfg = tst_mux_config_new();
            let prog = tst_mux_config_add_program(cfg, 1, 0x1000);
            tst_mux_config_add_video_stream(cfg, prog, 0x1011, TstVideoCodec::H264);
            let p = tst_udp_mux_sender_open(std::ptr::null(), cfg as *const _);
            assert!(p.is_null());
            tst_mux_config_free(cfg);
        }
    }

    #[test]
    fn open_with_null_config_returns_null() {
        let url = std::ffi::CString::new("udp://127.0.0.1:54322").unwrap();
        let p = unsafe { tst_udp_mux_sender_open(url.as_ptr(), std::ptr::null()) };
        assert!(p.is_null());
    }
}
