//! `tst_demux_receiver_*` stats accessors.
//!
//! Plain (non-managed) receiver stats surface: aggregate
//! `_get_stats`, wire-level `_get_socket_stats`, per-PID
//! `_get_stream_codec_stats` + `_get_stream_stats`, and a
//! counter-reset `_reset_stats`. Managed sibling lives in
//! `managed.rs`.

use super::TstDemuxReceiver;
use crate::error::{TstError, record_not_available, record_not_found, set_last_error};

/// Snapshot stats for a `tst_demux_receiver_t` into `*out`.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer
/// is null, `TST_E_CLOSED` if the receiver has been closed.
///
/// NOTE: per-PID counters are NOT included on this struct — call
/// `tst_demux_receiver_get_stream_stats` to retrieve them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stats(
    p: *mut TstDemuxReceiver,
    out: *mut crate::stats::TstDemuxReceiverStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = crate::stats::TstDemuxReceiverStats::from(&rx.stats());
        unsafe { *out = stats };
        0
    })
}

/// Read wire-level transport stats for the underlying libsrt socket.
/// See [`tst_mux_sender_get_socket_stats`](crate::sender::mux_sender::tst_mux_sender_get_socket_stats)
/// for full semantics — same shape, different handle type.
///
/// # Safety
///
/// Caller MUST ensure `p` is a valid `*mut TstDemuxReceiver` opened via
/// `tst_demux_receiver_open` and `out` points to a writable
/// `TstSocketStats`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_socket_stats(
    p: *mut TstDemuxReceiver,
    out: *mut crate::stats::TstSocketStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    unsafe { *out = crate::stats::TstSocketStats::default() };
    handle.inner.with_inner_ref(|rx| match rx.socket_stats() {
        Some(stats) => {
            unsafe { *out = (&stats).into() };
            0
        }
        None => record_not_available(
            "demux receiver socket stats unavailable (transport not connected or closed)",
        ),
    })
}

/// Snapshot codec-specific stats for one PID on a `tst_demux_receiver_t`
/// into `*out`.
///
/// The returned struct is a tagged union — read `out->kind` first, then
/// the matching `out->u.<arm>` field. See `tst_stream_codec_stats_t` in
/// `tstrans.h` for the discriminator constants (`TST_CODEC_KIND_*`).
///
/// # Errors
///
/// * `TST_E_INVALID_CONFIG` — `p` or `out` is null
/// * `TST_E_CLOSED` — handle was closed via `tst_demux_receiver_close`
/// * `TST_E_NOT_FOUND` — `pid` has never been observed on this handle
/// * `TST_E_INTERNAL` — internal panic caught at the FFI boundary
///
/// # Safety
///
/// `p` must be a valid pointer obtained from `tst_demux_receiver_open`;
/// `out` must be a writable `tst_stream_codec_stats_t`. The pointee is
/// fully written on `TST_OK` and untouched on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stream_codec_stats(
    p: *mut TstDemuxReceiver,
    pid: u16,
    out: *mut crate::stats::TstStreamCodecStats,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out.is_null() {
        set_last_error(TstError::InvalidConfig, "null out pointer");
        return TstError::InvalidConfig as i32;
    }
    handle
        .inner
        .with_inner_ref(|rx| match rx.stream_codec_stats(pid) {
            Some(stats) => {
                unsafe { *out = crate::stats::codec_stats_to_c(stats) };
                0
            }
            None => record_not_found(&format!(
                "codec stats not available for pid 0x{pid:04x} (pid has never been observed on this demux receiver)"
            )),
        })
}

/// Read the Unix-epoch microsecond timestamp of the last item observed
/// on `pid` (video/KLV/audio/PSI — any elementary stream tracked in
/// per-stream stats) into `*out_epoch_micros`.
///
/// `*out_epoch_micros` is `0` when `pid` has never been observed on this
/// handle — unknown pid and "never seen" are indistinguishable (both mean
/// "nothing to report yet") and this getter never errors on either case.
/// This lets a caller poll per-PID staleness (e.g. a watchdog checking
/// "has this stream gone quiet") with a single lock/lookup instead of
/// holding a `_get_stream_stats` snapshot open.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if either pointer is
/// null, `TST_E_CLOSED` if the receiver has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stream_last_seen_micros(
    p: *mut TstDemuxReceiver,
    pid: u16,
    out_epoch_micros: *mut u64,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out_epoch_micros.is_null() {
        set_last_error(TstError::InvalidConfig, "null out_epoch_micros pointer");
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = rx.stats();
        let micros = stats
            .per_stream
            .get(&pid)
            .map(|ss| crate::stats::last_seen_epoch_micros(ss.last_seen))
            .unwrap_or(0);
        unsafe { *out_epoch_micros = micros };
        0
    })
}

/// Reset stats counters for a `tst_demux_receiver_t` to zero.
/// Also invalidates the borrowed `_get_stream_stats` snapshot
/// (design §4.5).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` if the pointer is
/// null, `TST_E_CLOSED` if the receiver has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_reset_stats(p: *mut TstDemuxReceiver) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    // Clear the stream_stats_buf so any borrowed snapshot becomes
    // a dangling pointer — caller contract documents this as the
    // invalidation moment.
    if let Ok(mut buf) = handle.stream_stats_buf.lock() {
        buf.clear();
    }
    handle.inner.with_inner_mut(|rx| {
        rx.reset_stats();
        0
    })
}

/// Snapshot per-PID stats for a `tst_demux_receiver_t` into the
/// handle's internal buffer; return a `(*const TstStreamStats, size_t)`
/// pair borrowing that buffer.
///
/// **Borrowed buffer lifetime (design §4.5):** `*out_array` is valid
/// until the next `_get_stream_stats` / `_reset_stats` / `_close`
/// call on the same handle. Callers wanting longer lifetime memcpy
/// the array out.
///
/// Capped at `TST_STATS_MAX_STREAMS = 64` entries (BTreeMap ordering
/// preserved by ascending PID); excess streams are silently dropped.
/// `program_number` field is `0` for now — populated once `StreamStats`
/// surfaces it (currently absent from `tst_core::mpegts::stats::StreamStats`).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on any null pointer
/// arg, or `TST_E_CLOSED` if the receiver has been closed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_receiver_get_stream_stats(
    p: *mut TstDemuxReceiver,
    out_array: *mut *const crate::stats::TstStreamStats,
    out_count: *mut libc::size_t,
) -> libc::c_int {
    let Some(handle) = (unsafe { p.as_ref() }) else {
        set_last_error(TstError::InvalidConfig, "null receiver pointer");
        return TstError::InvalidConfig as i32;
    };
    if out_array.is_null() || out_count.is_null() {
        set_last_error(
            TstError::InvalidConfig,
            "null out_array or out_count pointer",
        );
        return TstError::InvalidConfig as i32;
    }
    handle.inner.with_inner_ref(|rx| {
        let stats = rx.stats();
        let mut buf = handle
            .stream_stats_buf
            .lock()
            .expect("stream_stats_buf Mutex poisoned");
        buf.clear();
        let cap = crate::stats::TST_STATS_MAX_STREAMS;
        for (pid, ss) in stats.per_stream.iter().take(cap) {
            let mut c_ss = crate::stats::TstStreamStats {
                pid: *pid,
                ..Default::default()
            };
            crate::stats::fill_stream_stats(&mut c_ss, ss);
            buf.push(c_ss);
        }
        // SAFETY: out_array / out_count non-null per guard above.
        // The returned pointer borrows from buf, which lives on the
        // handle until the next _get_stream_stats / _reset_stats /
        // _close call (caller contract per design §4.5).
        unsafe {
            *out_array = buf.as_ptr();
            *out_count = buf.len();
        }
        0
    })
}
