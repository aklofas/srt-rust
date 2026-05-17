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
    /// Internal panic caught at the FFI boundary; the handle is now in
    /// an indeterminate state. Subsequent calls on the same handle will
    /// also fail (returning `Closed`). The caller should free the handle.
    PanicCaught = -11,
    /// Peer disconnected gracefully (received TCP-style FIN / SRT clean close).
    /// Distinguished from `Closed` (caller-side cancel/close) so receive loops
    /// can branch on the shutdown reason. After this code the handle is dead;
    /// subsequent calls return `Closed`.
    EndOfStream = -12,
    /// Requested data is not currently available — typically because a
    /// `tst_managed_*` handle has no live inner socket (mid-reconnect or
    /// after close). Distinct from `InvalidUsage` (which means the handle
    /// is in a fundamentally wrong state for the call) — `NotAvailable`
    /// is transient and may resolve on the next call.
    ///
    /// Returned today by the `tst_*_get_socket_stats` family when the
    /// inner transport's `socket_stats()` returns `None`.
    NotAvailable = -13,
    /// Requested PID is not known to this site — used by the
    /// `tst_*_get_stream_codec_stats` family when the caller asks for
    /// a PID that has never been observed on this handle. Distinct from
    /// `NotAvailable` (transient — managed handle is mid-reconnect)
    /// and from `InvalidUsage` (handle is in a fundamentally wrong
    /// state for the call).
    NotFound = -14,
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
    crate::panic::ffi_catch(TstError::Internal as i32, || {
        LAST_ERROR.with(|cell| cell.borrow().0)
    })
}

/// Pointer to the most recent error message on this thread. Valid until
/// the next tst-c call on the same thread. Never NULL — empty string when
/// no error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_get_last_error_str() -> *const libc::c_char {
    // The thread-local CString stays alive for the thread's lifetime, but
    // if `borrow()` panicked (reentrant Drop double-borrow), the happy-path
    // pointer is unreachable. Fall back to a static empty C string so the
    // never-NULL contract above is preserved.
    static EMPTY: &[u8] = b"\0";
    let fallback = EMPTY.as_ptr() as *const libc::c_char;
    crate::panic::ffi_catch(fallback, || {
        LAST_ERROR.with(|cell| cell.borrow().1.as_ptr())
    })
}

use tst_core::error::MuxError;
#[cfg(test)]
use tst_core::mpegts::mux::StreamKind;
use tst_pipeline::{ShellError, ShellErrorKind, TransportError};

/// Map a [`ShellErrorKind`] to its corresponding [`TstError`] code.
///
/// This is the single point of truth for the kind-to-code projection.
/// CI ratchet `scripts/check-shell-error-kind-coverage.sh` (Task 10)
/// will enforce every `ShellErrorKind` variant is matched explicitly here.
pub(crate) fn tst_error_from_kind(kind: ShellErrorKind) -> TstError {
    match kind {
        ShellErrorKind::ConfigInvalid => TstError::InvalidConfig,
        ShellErrorKind::InputMalformed => TstError::InvalidTs,
        ShellErrorKind::Backpressure => TstError::BufferFull,
        ShellErrorKind::TransportBroken => TstError::Transport,
        ShellErrorKind::Closed => TstError::Closed,
        ShellErrorKind::EndOfStream => TstError::EndOfStream,
        // Required by #[non_exhaustive]. CI ratchet
        // scripts/check-shell-error-kind-coverage.sh (Task 10) enforces
        // every ShellErrorKind variant is matched above before this arm.
        _ => TstError::Internal,
    }
}

/// Record a shell error to the per-thread last-error slot. Used by
/// every C ABI entry point's error path. Replaces the per-variant
/// `record_sender_error` / `record_ts_sender_error` functions from
/// pre-Wave-4 code. The standalone-muxer path still uses
/// `record_mux_error` (for raw `MuxError` values not wrapped in a shell),
/// and the connect/listen helper paths still use `record_transport_error`
/// (for raw `TransportError` from pre-shell-layer code).
///
/// Returns the negative TST_E_* code suitable for direct return from
/// the C entry point.
pub(crate) fn record_shell_error<E: ShellError>(e: &E) -> i32 {
    let code = tst_error_from_kind(e.kind());
    set_last_error(code, &e.to_string());
    code as i32
}

/// Map a `MuxError` to a code + message.
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
            "no audio streams configured (audio carriage is supported via the Rust API; the C ABI sender surface does not currently expose audio send entries — see docs/deferred-features.md)"
                .into(),
        ),
        MuxError::NoSubtitleStreamsConfigured => (
            TstError::InvalidUsage,
            "no subtitle streams configured (subtitle carriage is supported via the Rust API; the C ABI sender surface does not currently expose subtitle send entries — see docs/deferred-features.md)"
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
        MuxError::KlvPidUsedAsPcrPid { pid } => (
            TstError::InvalidConfig,
            format!(
                "PCR PID 0x{pid:04X} resolves to a KLV stream — KLV cadence is too sparse for PCR \
                 (ETSI TR 101 290 §5.6.1); add a video stream or pin pcr_pid to a faster-cadence stream"
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
        MuxError::DescriptorIndexOutOfRange {
            kind,
            index,
            program_number,
        } => (
            TstError::InvalidUsage,
            format!(
                "descriptor index {index} out of range for {kind} streams in program \
                 {program_number} (call after the corresponding add_{kind})"
            ),
        ),
        MuxError::AbsIndexOutOfRange {
            abs_idx,
            len,
            program_number,
        } => (
            TstError::InvalidUsage,
            format!(
                "abs_idx {abs_idx} out of range for program {program_number} (has {len} streams)"
            ),
        ),
        MuxError::ConfigInvalid { reason } => {
            // Maps to the same TstError::InvalidConfig as the
            // flat-string MuxError::InvalidConfig variant — same
            // semantic ("muxer config is invalid, here's why"), just
            // with a richer reason. Bindings discriminating on the
            // numeric code see no change; bindings reading the
            // last-error string get the formatted diagnostic.
            (TstError::InvalidConfig, reason.clone())
        }
        _ => {
            // Required by #[non_exhaustive]. CI ratchet
            // scripts/check-tst-c-error-coverage.sh enforces that every
            // upstream MuxError variant is explicitly matched above; if
            // this arm fires at runtime, the ratchet failed (or was
            // bypassed). The Debug format names the unmapped variant so
            // last-error-str carries actionable diagnostics.
            (TstError::InvalidConfig, format!("unhandled MuxError variant: {e:?}"))
        }
    };
    set_last_error(code, &msg);
}

pub(crate) fn record_transport_error(e: &TransportError) {
    let (code, msg) = match e {
        TransportError::Backpressure(s) => (TstError::Transport, format!("backpressure: {s}")),
        TransportError::Broken(s) => (TstError::Transport, format!("broken: {s}")),
        TransportError::Closed => (TstError::Closed, "transport closed".into()),
        TransportError::TooLarge { len, max } => (
            TstError::TooLarge,
            format!("message {len} bytes exceeds payload cap {max}"),
        ),
        _ => {
            // Required by #[non_exhaustive]. See scripts/check-tst-c-error-coverage.sh
            // for the CI ratchet that prevents this arm from firing.
            (
                TstError::Transport,
                format!("unhandled TransportError variant: {e:?}"),
            )
        }
    };
    set_last_error(code, &msg);
}

/// Helper for entry points that catch panics or Mutex poison.
pub(crate) fn record_internal(detail: &str) {
    set_last_error(TstError::Internal, &format!("internal error: {detail}"));
}

/// Helper for the `catch_unwind` arm of `Handle::with_inner_*`. Records
/// a `PanicCaught` last-error with a useful detail message extracted
/// from the panic payload.
pub(crate) fn record_panic_caught(detail: &str) {
    set_last_error(
        TstError::PanicCaught,
        &format!("panic caught at FFI boundary: {detail}"),
    );
}

/// Record an end-of-stream condition. Used by receivers when the transport
/// reports a graceful peer close and the call was not caller-initiated.
pub(crate) fn record_eos() {
    set_last_error(TstError::EndOfStream, "end of stream (peer disconnected)");
}

/// Expose `record_shell_error` to integration tests that cannot access
/// `pub(crate)` items. Integration tests in `crates/tst-c/tests/` are
/// separate crates that can only reach `pub` items on the rlib.
///
/// These functions are NOT `extern "C"` and therefore do NOT appear in the
/// cbindgen-generated C header (`tstrans.h`). They are only reachable from
/// Rust tests that link the rlib. Named with a `test_` prefix so call sites
/// are self-documenting about their test-only status.
pub fn test_record_shell_error<E: ShellError>(e: &E) -> i32 {
    record_shell_error(e)
}

/// Read the thread-local last-error code for test assertions. Equivalent to
/// `tst_get_last_error()` but callable without `unsafe`. Not `extern "C"`;
/// does not appear in the C header.
pub fn test_last_error_code() -> i32 {
    LAST_ERROR.with(|cell| cell.borrow().0)
}

/// Read the thread-local last-error message string for test assertions. Not
/// `extern "C"`; does not appear in the C header.
pub fn test_last_error_msg() -> String {
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .1
            .to_str()
            .unwrap_or("<invalid utf8>")
            .to_owned()
    })
}

/// Clear the thread-local last-error for test isolation. Not `extern "C"`;
/// does not appear in the C header.
pub fn test_clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = (0, CString::new("").unwrap());
    });
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
            kind: StreamKind::Video,
            count: 2,
        };
        record_mux_error(&e);
        let s_ptr = unsafe { tst_get_last_error_str() };
        let msg = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
        assert!(msg.contains("tst_*_video_to"), "got: {msg}");
        assert!(!msg.contains("deferred"), "got: {msg}");
    }

    #[test]
    fn end_of_stream_code_is_negative_twelve() {
        assert_eq!(TstError::EndOfStream as i32, -12);
    }

    #[test]
    fn not_available_code_is_negative_thirteen() {
        assert_eq!(TstError::NotAvailable as i32, -13);
    }

    #[test]
    fn end_of_stream_records_distinct_from_closed() {
        clear_last_error_for_test();
        super::record_eos();
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::EndOfStream as i32
        );
        let s_ptr = unsafe { tst_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert!(s.to_str().unwrap().contains("end of stream"));
    }

    /// Helper: read the thread-local last-error string and assert it does
    /// NOT begin with `"unhandled "`. That prefix is uniquely produced by
    /// the Debug-format wildcard arms in `record_*_error`; its presence
    /// means a known variant fell through to the wildcard. Belt-and-
    /// suspenders with the per-variant exact-code assertion.
    fn assert_not_unhandled_wildcard() {
        let s_ptr = unsafe { tst_get_last_error_str() };
        let msg = unsafe { std::ffi::CStr::from_ptr(s_ptr) }.to_str().unwrap();
        assert!(
            !msg.starts_with("unhandled "),
            "wildcard arm fired for a known variant: {msg}"
        );
    }

    #[test]
    fn every_known_mux_error_variant_maps_to_expected_code() {
        use tst_core::mpegts::mux::{StreamKind, TeletextField};

        // (variant, expected TstError code). Cover every variant of MuxError.
        // Expected codes come from reading record_mux_error's explicit match
        // arms above.
        let cases: Vec<(MuxError, TstError)> = vec![
            (MuxError::InvalidConfig("test"), TstError::InvalidConfig),
            (
                MuxError::ConfigInvalid {
                    reason: "test".into(),
                },
                TstError::InvalidConfig,
            ),
            (MuxError::InvalidNal, TstError::InvalidNal),
            (
                MuxError::BufferFull {
                    capacity_packets: 1,
                },
                TstError::BufferFull,
            ),
            (
                MuxError::KlvTooLarge { size: 100, max: 50 },
                TstError::KlvTooLarge,
            ),
            (
                MuxError::InvalidStreamHandle {
                    kind: StreamKind::Video,
                    index: 0,
                },
                TstError::InvalidUsage,
            ),
            (
                MuxError::AmbiguousTarget {
                    kind: StreamKind::Video,
                    count: 2,
                },
                TstError::InvalidUsage,
            ),
            (MuxError::NoKlvStreamsConfigured, TstError::InvalidUsage),
            (MuxError::NoAudioStreamsConfigured, TstError::InvalidUsage),
            (
                MuxError::NoSubtitleStreamsConfigured,
                TstError::InvalidUsage,
            ),
            (
                MuxError::TooManyVideoStreams { count: 17, cap: 16 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::TooManyKlvStreams { count: 17, cap: 16 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::TooManyAudioStreams { count: 17, cap: 16 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::PmtTooLarge {
                    used_bytes: 200,
                    max_bytes: 183,
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::MalformedDescriptor {
                    stream_index: 0,
                    descriptor_index: 0,
                    reason: "test",
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::TooManyPrograms { count: 17, cap: 16 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::EmptyProgram { program_number: 1 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::DuplicateProgramNumber { program_number: 1 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::DuplicatePmtPid {
                    pid: 0x100,
                    programs: [1, 2],
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::DuplicatePidAcrossPrograms {
                    pid: 0x100,
                    programs: [1, 2],
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::ProgramNotFound { program_number: 1 },
                TstError::InvalidUsage,
            ),
            (
                MuxError::PmtPidConflictsWithStream {
                    pmt_pid: 0x100,
                    program_number: 1,
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::AudioTooLarge { size: 100, max: 50 },
                TstError::InvalidUsage,
            ),
            (
                MuxError::TooManySubtitleStreams { count: 17, cap: 16 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::SubtitleTooLarge { size: 100, max: 50 },
                TstError::InvalidUsage,
            ),
            (
                MuxError::SubtitlePidUsedAsPcrPid { pid: 0x100 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::KlvPidUsedAsPcrPid { pid: 0x100 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::InvalidLanguageCode {
                    code: [b'X', b'X', b'X'],
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::InvalidTeletextField {
                    field: TeletextField::MagazineNumber,
                    value: 99,
                    max: 7,
                },
                TstError::InvalidConfig,
            ),
            (
                MuxError::SubtitleOnlyProgram { program_number: 1 },
                TstError::InvalidConfig,
            ),
            (
                MuxError::DescriptorIndexOutOfRange {
                    kind: StreamKind::Video,
                    index: 5,
                    program_number: 1,
                },
                TstError::InvalidUsage,
            ),
            (
                MuxError::AbsIndexOutOfRange {
                    abs_idx: 99,
                    len: 3,
                    program_number: 1,
                },
                TstError::InvalidUsage,
            ),
        ];

        for (case, expected) in cases {
            clear_last_error_for_test();
            record_mux_error(&case);
            let code = unsafe { tst_get_last_error() };
            assert_eq!(
                code, expected as i32,
                "MuxError variant mapped to wrong code: {case:?} -> got {code}, expected {}",
                expected as i32
            );
            assert_not_unhandled_wildcard();
        }
    }

    #[test]
    fn every_known_transport_error_variant_maps_to_expected_code() {
        let cases: Vec<(TransportError, TstError)> = vec![
            (
                TransportError::Backpressure("test".into()),
                TstError::Transport,
            ),
            (TransportError::Broken("test".into()), TstError::Transport),
            (TransportError::Closed, TstError::Closed),
            (
                TransportError::TooLarge { len: 100, max: 50 },
                TstError::TooLarge,
            ),
        ];

        for (case, expected) in cases {
            clear_last_error_for_test();
            record_transport_error(&case);
            let code = unsafe { tst_get_last_error() };
            assert_eq!(
                code, expected as i32,
                "TransportError variant mapped to wrong code: {case:?} -> got {code}, expected {}",
                expected as i32
            );
            assert_not_unhandled_wildcard();
        }
    }
}
