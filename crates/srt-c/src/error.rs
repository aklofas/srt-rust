//! Thread-local last-error storage and the SRTC_E_* code enum.
//!
//! Mirrors libsrt's idiom: every fallible C function returns 0 on success
//! and a negative SRTC_E_* code on failure, with a thread-local detail
//! string available via srtc_get_last_error_str(). The detail string is
//! stable until the next srt-c call on the same thread.

use std::cell::RefCell;
use std::ffi::CString;

/// Negative codes returned by every fallible srt-c entry point.
///
/// `Success = 0` is the only non-negative variant. Codes are stable
/// across srt-c versions; new codes append at the end.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtcError {
    Success = 0,
    InvalidConfig = -1,
    InvalidNal = -2,
    InvalidTs = -3,
    BufferFull = -4,
    KlvTooLarge = -5,
    TooLarge = -6,
    Closed = -7,
    Transport = -8,
    InvalidUsage = -9,
    Internal = -10,
}

thread_local! {
    static LAST_ERROR: RefCell<(i32, CString)> = RefCell::new((0, CString::new("").unwrap()));
}

/// Set the per-thread last-error code + message. Internal helper used by
/// every fallible entry point on its error path.
pub(crate) fn set_last_error(code: SrtcError, msg: &str) {
    let cstr = CString::new(msg).unwrap_or_else(|_| CString::new("<message had nul>").unwrap());
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = (code as i32, cstr);
    });
}

#[cfg(test)]
pub(crate) fn clear_last_error_for_test() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = (0, CString::new("").unwrap());
    });
}

/// Read the most recent error code on this thread. Returns `0`
/// (`SRTC_E_SUCCESS`) if no error has been recorded on this thread yet.
/// The value is not cleared by successful calls; it reflects the most
/// recent failure on this thread (or `SRTC_E_SUCCESS` if there has been
/// none since thread start).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_get_last_error() -> libc::c_int {
    LAST_ERROR.with(|cell| cell.borrow().0)
}

/// Pointer to the most recent error message on this thread. Valid until
/// the next srt-c call on the same thread. Never NULL — empty string when
/// no error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_get_last_error_str() -> *const libc::c_char {
    LAST_ERROR.with(|cell| cell.borrow().1.as_ptr())
}

use srt_core::error::MuxError;
use srt_core::pipeline::{SenderError, TransportError, TsSenderError};

/// Map a `MuxError` to a code + message.
#[allow(dead_code)]
pub(crate) fn record_mux_error(e: &MuxError) {
    let (code, msg) = match e {
        MuxError::InvalidConfig(s) => (SrtcError::InvalidConfig, (*s).to_string()),
        MuxError::InvalidNal => (
            SrtcError::InvalidNal,
            "video input is not Annex-B framed".into(),
        ),
        MuxError::BufferFull { capacity_packets } => (
            SrtcError::BufferFull,
            format!("muxer buffer full ({capacity_packets} packets)"),
        ),
        MuxError::KlvTooLarge { size, max } => (
            SrtcError::KlvTooLarge,
            format!("KLV blob is {size} bytes, max {max}"),
        ),
        MuxError::InvalidStreamHandle { kind, index } => (
            SrtcError::InvalidUsage,
            format!("invalid {kind} stream handle (index {index})"),
        ),
        MuxError::AmbiguousTarget { kind, count } => (
            SrtcError::InvalidUsage,
            format!(
                "ambiguous push: {count} {kind} streams configured \
                 — use srtc_*_{kind}_to(handle, ...) to disambiguate"
            ),
        ),
        MuxError::TooManyVideoStreams { count, cap } => (
            SrtcError::InvalidConfig,
            format!("too many video streams: {count} configured, cap is {cap}"),
        ),
        MuxError::TooManyKlvStreams { count, cap } => (
            SrtcError::InvalidConfig,
            format!("too many klv streams: {count} configured, cap is {cap}"),
        ),
        MuxError::TooManyAudioStreams { count, cap } => (
            SrtcError::InvalidConfig,
            format!("too many audio streams: {count} configured, cap is {cap}"),
        ),
        MuxError::PmtTooLarge {
            used_bytes,
            max_bytes,
        } => (
            SrtcError::InvalidConfig,
            format!(
                "PMT too large: {used_bytes} bytes used, {max_bytes} max \
                 (drop some user-supplied descriptors or shorten their payloads)"
            ),
        ),
        MuxError::MalformedDescriptor {
            stream_index,
            descriptor_index,
            reason,
        } => (
            SrtcError::InvalidConfig,
            format!(
                "malformed descriptor for stream {stream_index} \
                 descriptor {descriptor_index}: {reason}"
            ),
        ),
        MuxError::TooManyPrograms { count, cap } => (
            SrtcError::InvalidConfig,
            format!("too many programs: {count} configured, cap is {cap}"),
        ),
        MuxError::EmptyProgram { program_number } => (
            SrtcError::InvalidConfig,
            format!("program {program_number} has no streams configured"),
        ),
        MuxError::DuplicateProgramNumber { program_number } => (
            SrtcError::InvalidConfig,
            format!("duplicate program_number {program_number} across programs"),
        ),
        MuxError::DuplicatePmtPid { pid, programs } => (
            SrtcError::InvalidConfig,
            format!("pmt_pid 0x{pid:04X} reused by programs {programs:?}"),
        ),
        MuxError::DuplicatePidAcrossPrograms { pid, programs } => (
            SrtcError::InvalidConfig,
            format!("stream PID 0x{pid:04X} used by programs {programs:?}"),
        ),
        MuxError::ProgramNotFound { program_number } => (
            SrtcError::InvalidUsage,
            format!("program {program_number} not found"),
        ),
        MuxError::PmtPidConflictsWithStream {
            pmt_pid,
            program_number,
        } => (
            SrtcError::InvalidConfig,
            format!(
                "pmt_pid 0x{pmt_pid:04X} of program {program_number} conflicts with a stream PID"
            ),
        ),
        MuxError::AudioTooLarge { size, max } => (
            SrtcError::InvalidUsage,
            format!("audio frames too large: {size} bytes, max {max}"),
        ),
        MuxError::TooManySubtitleStreams { count, cap } => (
            SrtcError::InvalidConfig,
            format!("too many subtitle streams: {count} configured, cap is {cap}"),
        ),
        MuxError::SubtitleTooLarge { size, max } => (
            SrtcError::InvalidUsage,
            format!("subtitle PES payload too large: {size} bytes (max {max})"),
        ),
        MuxError::SubtitlePidUsedAsPcrPid { pid } => (
            SrtcError::InvalidConfig,
            format!(
                "subtitle PID 0x{pid:04X} cannot be used as the PCR PID; \
                 subtitles are too sparse for PCR pacing"
            ),
        ),
        MuxError::InvalidLanguageCode { code } => (
            SrtcError::InvalidConfig,
            format!(
                "invalid ISO 639-2 language code: {code:02x?} (must be 3 lowercase ASCII bytes)"
            ),
        ),
        MuxError::InvalidTeletextField { field, value, max } => (
            SrtcError::InvalidConfig,
            format!("invalid DVB teletext {field}: {value} (max {max})"),
        ),
    };
    set_last_error(code, &msg);
}

#[allow(dead_code)]
pub(crate) fn record_transport_error(e: &TransportError) {
    let (code, msg) = match e {
        TransportError::Backpressure(s) => (SrtcError::Transport, format!("backpressure: {s}")),
        TransportError::Broken(s) => (SrtcError::Transport, format!("broken: {s}")),
        TransportError::Closed => (SrtcError::Closed, "transport closed".into()),
        TransportError::TooLarge { len, max } => (
            SrtcError::TooLarge,
            format!("message {len} bytes exceeds payload cap {max}"),
        ),
    };
    set_last_error(code, &msg);
}

#[allow(dead_code)]
pub(crate) fn record_sender_error(e: &SenderError) {
    match e {
        SenderError::Mux(m) => record_mux_error(m),
        SenderError::Transport(t) => record_transport_error(t),
    }
}

#[allow(dead_code)]
pub(crate) fn record_ts_sender_error(e: &TsSenderError) {
    match e {
        TsSenderError::Transport(t) => record_transport_error(t),
        TsSenderError::Framing(f) => set_last_error(SrtcError::InvalidTs, &f.to_string()),
    }
}

/// Helper for entry points that catch panics or Mutex poison.
#[allow(dead_code)]
pub(crate) fn record_internal(detail: &str) {
    set_last_error(SrtcError::Internal, &format!("internal error: {detail}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_roundtrips() {
        set_last_error(SrtcError::InvalidConfig, "bad pid");
        assert_eq!(
            unsafe { srtc_get_last_error() },
            SrtcError::InvalidConfig as i32
        );
        let s_ptr = unsafe { srtc_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert_eq!(s.to_str().unwrap(), "bad pid");
    }

    #[test]
    fn default_is_success_with_empty_string() {
        clear_last_error_for_test();
        assert_eq!(unsafe { srtc_get_last_error() }, 0);
        let s_ptr = unsafe { srtc_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert_eq!(s.to_str().unwrap(), "");
    }

    #[test]
    fn ambiguous_target_message_points_to_to_siblings() {
        let e = MuxError::AmbiguousTarget {
            kind: "video",
            count: 2,
        };
        record_mux_error(&e);
        let s_ptr = unsafe { srtc_get_last_error_str() };
        let msg = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
        assert!(msg.contains("srtc_*_video_to"), "got: {msg}");
        assert!(!msg.contains("deferred"), "got: {msg}");
    }
}
