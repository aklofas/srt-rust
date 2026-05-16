//! `tst_event_t` tagged union + subordinate `repr(C)` structs +
//! per-handle `EventArena` for the demux receiver hot path.
//!
//! Lifetime contract (design §4.5): all pointer fields on `TstEvent`
//! borrow from the `EventArena` owned by the `TstDemuxReceiver`
//! handle. Valid until the next `_recv_event` / `_close` call on the
//! same handle. Callers wanting longer lifetime memcpy out.

use libc::{c_char, c_int};

// ------------------------------------------------------------------
// Top-level event kind discriminator (5 variants)
// ------------------------------------------------------------------

/// `repr(i32)` discriminator for `TstEvent::kind`. cbindgen emits
/// `#define TST_EVENT_*` blocks.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstEventKind {
    ProgramMap = 1,
    Sample = 2,
    Metadata = 3,
    Discontinuity = 4,
    NonConformant = 5,
}

// ------------------------------------------------------------------
// Stream-kind discriminator (6 variants matching tst_core StreamKind)
// ------------------------------------------------------------------

/// `repr(i32)` mirror of `tst_core::mpegts::demux::StreamKind`'s
/// discriminator. Variant payloads (codec, declared_link, unknown
/// stream_type) live on the per-event union fields, not on this enum.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstStreamKindTag {
    Video = 0,
    Audio = 1,
    Subtitle = 2,
    KlvSync = 3,
    KlvAsync = 4,
    Unknown = 5,
}

// ------------------------------------------------------------------
// LinkSource (3 variants)
// ------------------------------------------------------------------

/// `repr(i32)` mirror of `tst_core::mpegts::demux::LinkSource`.
/// On `tst_klv_link_t.source` and on synthetic KLV-link inference.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstLinkSource {
    Declared = 0,
    Inferred = 1,
    Override = 2,
}

// ------------------------------------------------------------------
// Metadata-kind discriminator (3 variants)
// ------------------------------------------------------------------

/// `repr(i32)` mirror of `tst_core::mpegts::demux::MetadataKind`'s
/// discriminator. Variant payload (the KlvSyncAuCell 5 fields) lives
/// on `tst_event_t.u.metadata` fields, zero/false for other kinds.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstMetadataKindTag {
    KlvSyncAuCell = 0,
    KlvAsync = 1,
    Unknown = 2,
}

// ------------------------------------------------------------------
// Discontinuity-kind discriminator (4 variants)
// ------------------------------------------------------------------

/// `repr(i32)` mirror of `tst_core::mpegts::demux::DiscontinuityKind`'s
/// discriminator. ContinuityJump's `expected`/`observed` and
/// PesOversize's `pid` live on the union fields.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstDiscontinuityKindTag {
    ContinuityJump = 0,
    PesOversize = 1,
    PesTotalOversize = 2,
    AdaptationFieldFlag = 3,
}

// ------------------------------------------------------------------
// Non-conformant-issue codes (19 variants)
// ------------------------------------------------------------------

/// `repr(i32)` mirror of `tst_core::mpegts::demux::NonConformantIssue`'s
/// discriminator. Issue-specific fields live on `tst_event_t.u.nonconformant`
/// (zero/null for variants that don't carry them).
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstNonConformantCode {
    StreamTypeMismatchSyncOnAsyncPid = 0,
    StreamTypeMismatchAsyncOnSyncPid = 1,
    MissingMetadataDescriptor = 2,
    PcrAnomaly = 3,
    PsiChecksumMismatch = 4,
    PusiMidPes = 5,
    PidReusedAcrossPrograms = 6,
    SubtitleMissingDescriptor = 7,
    SubtitleDescriptorAmbiguous = 8,
    SubtitleDescriptorMalformed = 9,
    Av1RegistrationMalformed = 10,
    Av1ObuMissingSizeField = 11,
    Av1TileListNotAllowed = 12,
    PsiOverlongSection = 13,
    TransportErrorPacket = 14,
    PsiCcDiscontinuity = 15,
    MultiCellAu = 16,
    PsiMultiSectionUnsupported = 17,
    Other = 18,
}

// Subsequent items (subordinate structs, TstEvent, EventArena) are
// added in Tasks 7 and 8. Placeholder marker:
//
// pub struct TstNal { ... }            // Task 7
// pub struct TstObu { ... }            // Task 7
// pub struct TstDescriptor { ... }     // Task 7
// pub struct TstStreamInfo { ... }     // Task 7
// pub struct TstKlvLink { ... }        // Task 7
// pub struct TstEvent { ... }          // Task 8
// pub(crate) struct EventArena { ... } // Task 8
// pub(crate) fn convert(...)           // Task 8

// Suppress "unused" warnings on the C-only types until Tasks 7/8 wire
// them into the receiver entry points.
#[allow(dead_code)]
fn _silence_unused() {
    let _ = TstEventKind::ProgramMap;
    let _ = TstStreamKindTag::Video;
    let _ = TstLinkSource::Declared;
    let _ = TstMetadataKindTag::KlvSyncAuCell;
    let _ = TstDiscontinuityKindTag::ContinuityJump;
    let _ = TstNonConformantCode::Other;
    let _: c_int = 0;
    let _: c_char = 0;
}
