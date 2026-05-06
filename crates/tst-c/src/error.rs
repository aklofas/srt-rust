//! Thread-local last-error storage and the TST_E_* code enum.
//!
//! Mirrors libsrt's idiom: every fallible C function returns 0 on success
//! and a negative TST_E_* code on failure, with a thread-local detail
//! string available via tst_get_last_error_str(). The detail string is
//! stable until the next tst-c call on the same thread.

use std::cell::RefCell;
use std::ffi::CString;

/// Negative codes returned by every fallible tst-c entry point.
///
/// `Success = 0` is the only non-negative variant. Codes are stable
/// across tst-c versions; new codes append at the end.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TstError {
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
pub(crate) fn set_last_error(code: TstError, msg: &str) {
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
/// (`TST_E_SUCCESS`) if no error has been recorded on this thread yet.
/// The value is not cleared by successful calls; it reflects the most
/// recent failure on this thread (or `TST_E_SUCCESS` if there has been
/// none since thread start).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_last_error() -> libc::c_int {
    LAST_ERROR.with(|cell| cell.borrow().0)
}

/// Pointer to the most recent error message on this thread. Valid until
/// the next tst-c call on the same thread. Never NULL — empty string when
/// no error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_last_error_str() -> *const libc::c_char {
    LAST_ERROR.with(|cell| cell.borrow().1.as_ptr())
}

use tst_core::error::MuxError;
use tst_pipeline::{MuxSenderError, SenderError, TransportError};

/// Map a `MuxError` to a code + message.
#[allow(dead_code)]
pub(crate) fn record_mux_error(e: &MuxError) {
    let (code, msg) = match e {
        MuxError::InvalidConfig(s) => (TstError::InvalidConfig, (*s).to_string()),
        MuxError::InvalidNal => (
            TstError::InvalidNal,
            "video input is not Annex-B framed".into(),
        ),
        MuxError::BufferFull { capacity_packets } => (
            TstError::BufferFull,
            format!("muxer buffer full ({capacity_packets} packets)"),
        ),
        MuxError::KlvTooLarge { size, max } => (
            TstError::KlvTooLarge,
            format!("KLV blob is {size} bytes, max {max}"),
        ),
        MuxError::InvalidStreamHandle { kind, index } => (
            TstError::InvalidUsage,
            format!("invalid {kind} stream handle (index {index})"),
        ),
        MuxError::AmbiguousTarget { kind, count } => (
            TstError::InvalidUsage,
            format!(
                "ambiguous push: {count} {kind} streams configured \
                 — use tst_*_{kind}_to(handle, ...) to disambiguate"
            ),
        ),
        MuxError::NoKlvStreamsConfigured => (
            TstError::InvalidUsage,
            "no KLV streams configured; use tst_*_klv_to with a handle from klv_handles".into(),
        ),
        MuxError::NoAudioStreamsConfigured => (
            TstError::InvalidUsage,
            "no audio streams configured; use tst_*_audio_to with a handle from audio_handles"
                .into(),
        ),
        MuxError::NoSubtitleStreamsConfigured => (
            TstError::InvalidUsage,
            "no subtitle streams configured; use tst_*_subtitle_to with a handle from subtitle_handles"
                .into(),
        ),
        MuxError::TooManyVideoStreams { count, cap } => (
            TstError::InvalidConfig,
            format!("too many video streams: {count} configured, cap is {cap}"),
        ),
        MuxError::TooManyKlvStreams { count, cap } => (
            TstError::InvalidConfig,
            format!("too many klv streams: {count} configured, cap is {cap}"),
        ),
        MuxError::TooManyAudioStreams { count, cap } => (
            TstError::InvalidConfig,
            format!("too many audio streams: {count} configured, cap is {cap}"),
        ),
        MuxError::PmtTooLarge {
            used_bytes,
            max_bytes,
        } => (
            TstError::InvalidConfig,
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
            TstError::InvalidConfig,
            format!(
                "malformed descriptor for stream {stream_index} \
                 descriptor {descriptor_index}: {reason}"
            ),
        ),
        MuxError::TooManyPrograms { count, cap } => (
            TstError::InvalidConfig,
            format!("too many programs: {count} configured, cap is {cap}"),
        ),
        MuxError::EmptyProgram { program_number } => (
            TstError::InvalidConfig,
            format!("program {program_number} has no streams configured"),
        ),
        MuxError::DuplicateProgramNumber { program_number } => (
            TstError::InvalidConfig,
            format!("duplicate program_number {program_number} across programs"),
        ),
        MuxError::DuplicatePmtPid { pid, programs } => (
            TstError::InvalidConfig,
            format!("pmt_pid 0x{pid:04X} reused by programs {programs:?}"),
        ),
        MuxError::DuplicatePidAcrossPrograms { pid, programs } => (
            TstError::InvalidConfig,
            format!("stream PID 0x{pid:04X} used by programs {programs:?}"),
        ),
        MuxError::ProgramNotFound { program_number } => (
            TstError::InvalidUsage,
            format!("program {program_number} not found"),
        ),
        MuxError::PmtPidConflictsWithStream {
            pmt_pid,
            program_number,
        } => (
            TstError::InvalidConfig,
            format!(
                "pmt_pid 0x{pmt_pid:04X} of program {program_number} conflicts with a stream PID"
            ),
        ),
        MuxError::AudioTooLarge { size, max } => (
            TstError::InvalidUsage,
            format!("audio frames too large: {size} bytes, max {max}"),
        ),
        MuxError::TooManySubtitleStreams { count, cap } => (
            TstError::InvalidConfig,
            format!("too many subtitle streams: {count} configured, cap is {cap}"),
        ),
        MuxError::SubtitleTooLarge { size, max } => (
            TstError::InvalidUsage,
            format!("subtitle PES payload too large: {size} bytes (max {max})"),
        ),
        MuxError::SubtitlePidUsedAsPcrPid { pid } => (
            TstError::InvalidConfig,
            format!(
                "subtitle PID 0x{pid:04X} cannot be used as the PCR PID; \
                 subtitles are too sparse for PCR pacing"
            ),
        ),
        MuxError::InvalidLanguageCode { code } => (
            TstError::InvalidConfig,
            format!(
                "invalid ISO 639-2 language code: {code:02x?} (must be 3 lowercase ASCII bytes)"
            ),
        ),
        MuxError::InvalidTeletextField { field, value, max } => (
            TstError::InvalidConfig,
            format!("invalid DVB teletext {field}: {value} (max {max})"),
        ),
        MuxError::SubtitleOnlyProgram { program_number } => (
            TstError::InvalidConfig,
            format!(
                "program {program_number} contains only subtitle streams; \
                 PCR cannot be resolved (subtitles must not carry PCR per EN 300 472 §4.0)"
            ),
        ),
    };
    set_last_error(code, &msg);
}

#[allow(dead_code)]
pub(crate) fn record_transport_error(e: &TransportError) {
    let (code, msg) = match e {
        TransportError::Backpressure(s) => (TstError::Transport, format!("backpressure: {s}")),
        TransportError::Broken(s) => (TstError::Transport, format!("broken: {s}")),
        TransportError::Closed => (TstError::Closed, "transport closed".into()),
        TransportError::TooLarge { len, max } => (
            TstError::TooLarge,
            format!("message {len} bytes exceeds payload cap {max}"),
        ),
    };
    set_last_error(code, &msg);
}

#[allow(dead_code)]
pub(crate) fn record_sender_error(e: &MuxSenderError) {
    match e {
        MuxSenderError::Mux(m) => record_mux_error(m),
        MuxSenderError::Transport(t) => record_transport_error(t),
    }
}

#[allow(dead_code)]
pub(crate) fn record_ts_sender_error(e: &SenderError) {
    match e {
        SenderError::Transport(t) => record_transport_error(t),
        SenderError::Framing(f) => set_last_error(TstError::InvalidTs, &f.to_string()),
    }
}

/// Helper for entry points that catch panics or Mutex poison.
#[allow(dead_code)]
pub(crate) fn record_internal(detail: &str) {
    set_last_error(TstError::Internal, &format!("internal error: {detail}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_then_get_roundtrips() {
        set_last_error(TstError::InvalidConfig, "bad pid");
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::InvalidConfig as i32
        );
        let s_ptr = unsafe { tst_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert_eq!(s.to_str().unwrap(), "bad pid");
    }

    #[test]
    fn default_is_success_with_empty_string() {
        clear_last_error_for_test();
        assert_eq!(unsafe { tst_get_last_error() }, 0);
        let s_ptr = unsafe { tst_get_last_error_str() };
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
        let s_ptr = unsafe { tst_get_last_error_str() };
        let msg = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
        assert!(msg.contains("tst_*_video_to"), "got: {msg}");
        assert!(!msg.contains("deferred"), "got: {msg}");
    }
}
