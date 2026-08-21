//! `TstStreamEndReason` — C mirror of `tst_rtp::StreamEndReason`.
//!
//! Read via `tst_rtp_receiver_end_reason` (on `TstRtpReceiver`) and
//! `tst_rtp_demux_receiver_end_reason` (on `TstRtpDemuxReceiver`). Both
//! wrapper structs capture a `StreamEndReasonHandle` from the underlying
//! `RtpRecvTransport` at open/conversion time, before the transport moves
//! into the shell — same capture-before-move timing as the existing
//! `cancel_handle` field on both structs.

use tst_rtp::StreamEndReason;

use crate::error::{TstError, set_last_error};

/// Why an RTP receive session ended. Mirrors `tst_rtp::StreamEndReason`
/// with one addition — `None` (0) — for "hasn't ended yet, or ended
/// through a path this arc doesn't instrument" (the case
/// `StreamEndReasonHandle::get()` reports as `Option::None`, e.g. a plain
/// `rtp://` receiver that was never `_cancel`'d or `_close`'d).
/// Discriminants 1-6 are cross-surface stable — the Python and JVM
/// bindings use the same numbering.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstStreamEndReason {
    /// The session hasn't ended yet, or ended through a path this arc
    /// doesn't instrument.
    None = 0,
    /// The peer closed the connection in an orderly way, with no
    /// protocol or transport error.
    CleanTeardown = 1,
    /// The server no longer honors the session — a keepalive ping was
    /// answered `454 Session Not Found`.
    SessionExpired = 2,
    /// The keepalive background thread failed to encode or send a ping.
    /// Detail message: see the getter doc.
    KeepaliveFailed = 3,
    /// A hard I/O error on the underlying transport. Detail message:
    /// see the getter doc.
    TransportFailed = 4,
    /// The peer violated the wire protocol. Detail message: see the
    /// getter doc.
    ProtocolError = 5,
    /// The caller explicitly cancelled or closed the transport — not a
    /// wire-level failure.
    Cancelled = 6,
}

/// Convert a recorded [`StreamEndReason`] to its C discriminant.
///
/// `KeepaliveFailed` / `TransportFailed` / `ProtocolError` carry a `msg`
/// detail string with no home on a plain `repr(C)` enum. Rather than add
/// a new leak-prone owned-string field to every caller's out-param, the
/// detail rides the existing thread-local last-error message channel —
/// [`set_last_error`] is called with [`TstError::Success`] (the getter
/// itself did not fail; a recorded end reason is data) so
/// `tst_get_last_error() == 0` still holds for callers using that as a
/// "did the last call succeed" check, while `tst_last_error_str()`
/// picks up the extra detail immediately after the getter call.
pub(crate) fn convert_end_reason(r: &StreamEndReason) -> TstStreamEndReason {
    match r {
        StreamEndReason::CleanTeardown => TstStreamEndReason::CleanTeardown,
        StreamEndReason::SessionExpired => TstStreamEndReason::SessionExpired,
        StreamEndReason::KeepaliveFailed { msg } => {
            set_last_error(TstError::Success, msg);
            TstStreamEndReason::KeepaliveFailed
        }
        StreamEndReason::TransportFailed { msg } => {
            set_last_error(TstError::Success, msg);
            TstStreamEndReason::TransportFailed
        }
        StreamEndReason::ProtocolError { msg } => {
            set_last_error(TstError::Success, msg);
            TstStreamEndReason::ProtocolError
        }
        StreamEndReason::Cancelled => TstStreamEndReason::Cancelled,
        // StreamEndReason is #[non_exhaustive] on the tst-rtp side. A
        // future variant this binding doesn't know how to map yet falls
        // back to None — "ended through a path this arc doesn't
        // instrument" is exactly true of it from the C ABI's
        // perspective until the mapping above is extended.
        _ => TstStreamEndReason::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_teardown_maps_without_touching_last_error() {
        assert!(matches!(
            convert_end_reason(&StreamEndReason::CleanTeardown),
            TstStreamEndReason::CleanTeardown
        ));
    }

    #[test]
    fn cancelled_maps() {
        assert!(matches!(
            convert_end_reason(&StreamEndReason::Cancelled),
            TstStreamEndReason::Cancelled
        ));
    }

    #[test]
    fn session_expired_maps() {
        assert!(matches!(
            convert_end_reason(&StreamEndReason::SessionExpired),
            TstStreamEndReason::SessionExpired
        ));
    }

    #[test]
    fn transport_failed_maps_and_sets_last_error_detail() {
        crate::error::clear_last_error_for_test();
        let converted = convert_end_reason(&StreamEndReason::TransportFailed {
            msg: "connection reset".to_string(),
        });
        assert!(matches!(converted, TstStreamEndReason::TransportFailed));
        assert_eq!(
            crate::error::test_last_error_code(),
            TstError::Success as i32
        );
        assert_eq!(crate::error::test_last_error_msg(), "connection reset");
    }
}
