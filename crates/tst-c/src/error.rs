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
    /// (-13) Resource temporarily unavailable; retry later.
    ///
    /// Returned by stats/socket_stats accessors on a managed handle while
    /// the underlying transport is reconnecting. The same call may
    /// succeed once reconnect completes — bindings should expose this as
    /// a transient signal that does not require user intervention.
    ///
    /// Returned today by the `tst_*_get_socket_stats` family when the
    /// inner transport's `socket_stats()` returns `None` (mid-reconnect
    /// or after close).
    ///
    /// **Contract:** transient. The next call on the same handle may
    /// succeed.
    ///
    /// See [`TstError::NotFound`] for the persistent counterpart, and
    /// [`TstError::InvalidUsage`] for the wrong-handle-state case.
    NotAvailable = -13,
    /// (-14) Resource not found; the request will not succeed on this handle.
    ///
    /// Returned by per-PID accessors (codec stats, stream info) when the
    /// PID has never been observed on this stream. Distinct from
    /// `NotAvailable` (which is transient — same call may later succeed)
    /// and from `InvalidUsage` (which means the handle is in a
    /// fundamentally wrong state for the call entirely).
    ///
    /// Returned today by the `tst_*_get_stream_codec_stats` family when
    /// the caller asks for a PID that has never been observed on this
    /// handle.
    ///
    /// **Contract:** persistent. The next call on the same handle with
    /// the same key will return the same error. Retry is futile unless
    /// the caller knows the key has since started being observed.
    ///
    /// See [`TstError::NotAvailable`] for the transient counterpart.
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

/// Clears the thread-local last-error slot, resetting it to
/// `(TST_E_SUCCESS, "")`.
///
/// Most callers should NOT need this — every fallible `tst_*` function
/// returns its result code directly (0 on success, negative on failure),
/// so checking the return value is the idiomatic pattern. The
/// thread-local last-error slot is a side-channel for the **message
/// string** corresponding to the most recent failure, useful for
/// logging and diagnostics.
///
/// Use this function when:
///
/// 1. Chaining checks through code that doesn't propagate return values
///    (e.g., a series of `tst_mux_config_add_*_stream` calls in a
///    higher-level helper that returns a single combined status).
/// 2. Discriminating "the most recent call succeeded" from "the most
///    recent call failed and set an error" using `tst_get_last_error()
///    == 0` as the post-call check.
///
/// **Thread-locality:** clears only the calling thread's slot. Other
/// threads' last-error values are unaffected. Matches the libsrt
/// `srt_clearlasterror()` semantic.
///
/// # Safety
///
/// Sound under any caller invocation — no pointer arguments, no
/// mutating shared state (the thread-local is per-thread by definition),
/// no internal locks. The `unsafe extern "C"` annotation matches the
/// convention of every other `tst_*` entry point for consistency.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_clear_last_error() {
    crate::panic::ffi_catch((), || {
        set_last_error(TstError::Success, "");
    });
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

/// Map a `MuxError` to a code + message via the inner-tier
/// `MuxSenderErrorKind` category.
///
/// The code projection routes through `MuxError::kind()` for the
/// `ConfigInvalid` / `InvalidUsage` / `Backpressure` / `Internal`
/// categories (each has a stable per-kind `TST_E_*` code). The
/// `InputMalformed` category has 4 variants mapping to 3 different
/// `TstError` codes, so 2 variants get explicit overrides. The
/// diagnostic message uses `MuxError`'s `Display` impl preserving
/// spec-rich diagnostics from the `#[error("...")]` attributes.
///
/// **CI invariants:**
///
/// 1. `scripts/check-raw-c-mapper-coverage.sh` — every `MuxError`
///    variant must be mentioned in the per-variant routing table
///    inside this function before the wildcard arm.
/// 2. The in-file unit test `every_known_mux_error_variant_maps_to_expected_code`
///    verifies all 32 variants produce the expected `TstError` code.
pub(crate) fn record_mux_error(e: &MuxError) {
    use tst_core::error::MuxSenderErrorKind;

    // Per-variant code routing (covered by kind() projection below
    // unless explicitly overridden). The ratchet
    // scripts/check-raw-c-mapper-coverage.sh greps this block for
    // every MuxError::VariantName before the wildcard arm.
    //
    //   MuxError::InvalidNal              -> TstError::InvalidNal     [override]
    //   MuxError::KlvTooLarge             -> TstError::KlvTooLarge    [override]
    //   MuxError::AudioTooLarge           -> TstError::InvalidUsage   (InputMalformed kind default)
    //   MuxError::SubtitleTooLarge        -> TstError::InvalidUsage   (InputMalformed kind default)
    //   MuxError::BufferFull              -> TstError::BufferFull     (Backpressure kind default)
    //   MuxError::InvalidConfig           -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::ConfigInvalid           -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::InvalidLanguageCode     -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::InvalidTeletextField    -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::TooManyVideoStreams     -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::TooManyKlvStreams       -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::TooManyAudioStreams     -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::TooManySubtitleStreams  -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::TooManyPrograms         -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::EmptyProgram            -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::DuplicateProgramNumber  -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::DuplicatePmtPid         -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::DuplicatePidAcrossPrograms -> TstError::InvalidConfig (ConfigInvalid kind default)
    //   MuxError::PmtPidConflictsWithStream  -> TstError::InvalidConfig (ConfigInvalid kind default)
    //   MuxError::SubtitlePidUsedAsPcrPid -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::KlvPidUsedAsPcrPid      -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::SubtitleOnlyProgram     -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::MalformedDescriptor     -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::PmtTooLarge             -> TstError::InvalidConfig  (ConfigInvalid kind default)
    //   MuxError::InvalidStreamHandle     -> TstError::InvalidUsage   (InvalidUsage kind default)
    //   MuxError::AmbiguousTarget         -> TstError::InvalidUsage   (InvalidUsage kind default)
    //   MuxError::NoKlvStreamsConfigured  -> TstError::InvalidUsage   (InvalidUsage kind default)
    //   MuxError::NoAudioStreamsConfigured -> TstError::InvalidUsage  (InvalidUsage kind default)
    //   MuxError::NoSubtitleStreamsConfigured -> TstError::InvalidUsage (InvalidUsage kind default)
    //   MuxError::ProgramNotFound         -> TstError::InvalidUsage   (InvalidUsage kind default)
    //   MuxError::DescriptorIndexOutOfRange -> TstError::InvalidUsage (InvalidUsage kind default)
    //   MuxError::AbsIndexOutOfRange      -> TstError::InvalidUsage   (InvalidUsage kind default)
    let code = match e {
        // InputMalformed bucket — variant-specific code overrides.
        // The kind-default for InputMalformed maps to InvalidUsage;
        // these 2 variants project to more specific codes for
        // diagnostic precision.
        MuxError::InvalidNal => TstError::InvalidNal,
        MuxError::KlvTooLarge { .. } => TstError::KlvTooLarge,

        // All other variants route via the kind() projection.
        _ => match e.kind() {
            MuxSenderErrorKind::ConfigInvalid => TstError::InvalidConfig,
            MuxSenderErrorKind::InvalidUsage => TstError::InvalidUsage,
            MuxSenderErrorKind::Backpressure => TstError::BufferFull,
            // AudioTooLarge + SubtitleTooLarge fall through here (the
            // 2 InputMalformed variants not covered by overrides above).
            // Both project to InvalidUsage per the pre-Wave-6.D behavior.
            MuxSenderErrorKind::InputMalformed => TstError::InvalidUsage,
            MuxSenderErrorKind::Internal => TstError::Internal,
            // Required by #[non_exhaustive]. CI ratchet
            // scripts/check-mux-error-kind-coverage.sh enforces every
            // MuxSenderErrorKind variant is matched above before this arm.
            // Matches the wildcard-default-to-Internal pattern from Wave
            // 4.A (record_shell_error) and Wave 6.D (MuxError::kind() at
            // tst-core/src/error.rs:631): an unknown future coarse kind
            // is more truthful as a library/internal failure than as
            // caller InvalidConfig.
            _ => TstError::Internal,
        },
    };
    // Use the existing Display impl on MuxError — each variant's
    // #[error("...")] attribute already produces a spec-rich diagnostic
    // string.
    set_last_error(code, &e.to_string());
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
            // Required by #[non_exhaustive]. See scripts/check-raw-c-mapper-coverage.sh
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

/// Record `NotAvailable` (-13) with a per-call message and return the
/// negative code. Use this from C ABI entry points that hit a transient
/// "unavailable" condition (typically `socket_stats() -> None` mid-reconnect
/// or after close).
///
/// Replaces the direct `TstError::NotAvailable as i32` pattern that leaves
/// stale last-error state visible to `tst_get_last_error()` (per Codex
/// re-review finding 1, plan #93).
#[allow(dead_code)]
pub(crate) fn record_not_available(msg: &str) -> i32 {
    set_last_error(TstError::NotAvailable, msg);
    TstError::NotAvailable as i32
}

/// Record `NotFound` (-14) with a per-call message and return the negative
/// code. Use this from C ABI per-PID / per-key accessors when the requested
/// key has never been observed on this handle.
///
/// Replaces the direct `TstError::NotFound as i32` pattern that leaves
/// stale last-error state visible to `tst_get_last_error()`.
#[allow(dead_code)]
pub(crate) fn record_not_found(msg: &str) -> i32 {
    set_last_error(TstError::NotFound, msg);
    TstError::NotFound as i32
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
    fn tst_clear_last_error_resets_to_success_state() {
        // Defensive baseline: ensure we start from success state regardless
        // of any test ordering. set_last_error in Step 1 then primes the
        // specific non-success state we're testing the clear of.
        clear_last_error_for_test();

        // Step 1: prime the thread-local with a non-success error.
        set_last_error(TstError::InvalidConfig, "stale failure");
        assert_eq!(
            unsafe { tst_get_last_error() },
            TstError::InvalidConfig as i32,
            "precondition: error should be set before clear"
        );
        let s_ptr = unsafe { tst_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert_eq!(
            s.to_str().unwrap(),
            "stale failure",
            "precondition: message should be 'stale failure' before clear"
        );

        // Step 2: call the new public C entry under test.
        unsafe { tst_clear_last_error() };

        // Step 3: assert both code and message are reset.
        assert_eq!(
            unsafe { tst_get_last_error() },
            0,
            "after tst_clear_last_error(), code should be TST_E_SUCCESS (0)"
        );
        let s_ptr = unsafe { tst_get_last_error_str() };
        let s = unsafe { std::ffi::CStr::from_ptr(s_ptr) };
        assert_eq!(
            s.to_str().unwrap(),
            "",
            "after tst_clear_last_error(), message should be empty"
        );
    }

    #[test]
    fn tst_clear_last_error_idempotent_when_already_clear() {
        // Reset baseline, then clear twice — must remain in success state.
        clear_last_error_for_test();
        assert_eq!(
            unsafe { tst_get_last_error() },
            0,
            "baseline: expected TST_E_SUCCESS (0) after clear_last_error_for_test()"
        );

        unsafe { tst_clear_last_error() };
        assert_eq!(unsafe { tst_get_last_error() }, 0);
        unsafe { tst_clear_last_error() };
        assert_eq!(unsafe { tst_get_last_error() }, 0);
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
        // The message is the MuxError Display impl output which says
        // "call push_video_to(handle, ...) instead" — the key is that
        // it points to a disambiguation API, not the deferred path.
        assert!(msg.contains("push_video_to"), "got: {msg}");
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

    #[test]
    fn record_not_available_sets_last_error_code() {
        test_clear_last_error();
        let rc = record_not_available("socket stats unavailable (reconnecting)");
        assert_eq!(rc, TstError::NotAvailable as i32);
        assert_eq!(test_last_error_code(), TstError::NotAvailable as i32);
    }

    #[test]
    fn record_not_available_overwrites_prior_error() {
        test_clear_last_error();
        // Seed a stale unrelated error (simulating a prior failing call).
        set_last_error(TstError::InvalidConfig, "stale config error");
        assert_eq!(test_last_error_code(), TstError::InvalidConfig as i32);

        // record_not_available must overwrite both code AND message.
        let _ = record_not_available("socket stats unavailable");
        assert_eq!(test_last_error_code(), TstError::NotAvailable as i32);
        assert!(
            test_last_error_msg().contains("socket stats unavailable"),
            "last-error message did not overwrite; got: {:?}",
            test_last_error_msg()
        );
    }

    #[test]
    fn record_not_found_sets_last_error_code() {
        test_clear_last_error();
        let rc = record_not_found("pid 0x100 not observed on this handle");
        assert_eq!(rc, TstError::NotFound as i32);
        assert_eq!(test_last_error_code(), TstError::NotFound as i32);
    }

    #[test]
    fn record_not_found_overwrites_prior_error() {
        test_clear_last_error();
        set_last_error(TstError::InvalidUsage, "stale usage error");
        assert_eq!(test_last_error_code(), TstError::InvalidUsage as i32);

        let _ = record_not_found("pid not observed");
        assert_eq!(test_last_error_code(), TstError::NotFound as i32);
        assert!(
            test_last_error_msg().contains("pid not observed"),
            "last-error message did not overwrite; got: {:?}",
            test_last_error_msg()
        );
    }
}
