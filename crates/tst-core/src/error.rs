//! Non-transport error types: KLV, MPEG-TS mux/demux.

use thiserror::Error;

use crate::mpegts::mux::{StreamKind, TeletextField};

// ============================================================================
// KLV errors
// ============================================================================

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KlvDecodeError {
    #[error("buffer truncated at offset {offset}: needed {needed} bytes, have {have}")]
    Truncated {
        offset: usize,
        needed: usize,
        have: usize,
    },

    #[error("malformed BER length at offset {offset}")]
    MalformedLength { offset: usize },

    #[error("BER length {value} exceeds maximum supported size")]
    LengthOverflow { value: u64 },

    #[error("malformed BER-OID tag at offset {offset}")]
    MalformedTag { offset: usize },

    /// Non-canonical BER long-form length encoding (per MISB ST 0107.5
    /// §6.3.2: encoders shall use the fewest bytes). Returned by
    /// `read_ber_strict`; the permissive `read_ber` accepts non-canonical
    /// for legacy capture interop.
    #[error("non-canonical BER length encoding at offset {offset}")]
    NonCanonicalLength { offset: usize },

    /// Non-canonical BER-OID encoding (per MISB ST 0107.5 §6.3.1:
    /// leading `0x80` byte forbidden). Returned by `read_ber_oid_strict`;
    /// the permissive `read_ber_oid` accepts non-canonical for legacy.
    #[error("non-canonical BER-OID tag at offset {offset}")]
    NonCanonicalTag { offset: usize },

    #[error("unexpected universal label: expected {expected}, got {found}")]
    UnexpectedUniversalLabel {
        expected: crate::klv::UniversalLabel,
        found: crate::klv::UniversalLabel,
    },

    #[error("checksum mismatch: declared {expected:#06x}, computed {found:#06x}")]
    ChecksumMismatch { expected: u16, found: u16 },

    #[error("duplicate tag {tag} at offset {offset}")]
    DuplicateTag { tag: u32, offset: usize },

    #[error("trailing bytes after declared length: {len} extra")]
    TrailingBytes { len: usize },

    #[error("Precision Time Stamp Pack body must be 9 bytes, got {got}")]
    BadTimeStampPackLength { got: usize },

    /// Not produced by `klv::st0605::decode` (which is permissive about
    /// reserved bits per its doc); call `time_status.reserved_bits_valid()`
    /// on the decoded pack and raise this if a stricter caller wants it.
    #[error("Time Status reserved bits 4-0 must be 0b11111, got {got:#04x}")]
    ReservedBitsInvalid { got: u8 },

    #[error("Tag 2 (timestamp) must be the first element per ST 0601.8-09")]
    Tag2NotFirst,

    #[error("Tag 1 (checksum) must be the last element per ST 0601.8-11")]
    Tag1NotLast,

    #[error("Tag 65 (UAS LS Version) is required per ST 0601.8-12")]
    MissingTag65,

    /// Strict-mode `klv::st0102::decode_strict` only: spec-mandatory tag
    /// (1, 2, 3, 12, 13, or 22) absent from the record per ST 0102.12
    /// §6.7 Table 2.
    #[error("ST 0102 record missing required tag {tag} per ST 0102.12 §6.7")]
    St0102MissingRequiredTag { tag: u8 },

    /// Strict-mode `klv::st0903::decode_strict` only: spec-mandatory tag
    /// absent from the record per ST 0903.6 §6 Table 1.
    #[error("ST 0903 record missing required tag {tag} per ST 0903.6 §6")]
    St0903MissingRequiredTag { tag: u8 },

    /// Pack-internal malformation surfaced from VTargetSeries (Tag 101)
    /// decode. `offset` is the byte offset within the VTargetSeries
    /// payload (not the outer LS).
    #[error("ST 0903 VTargetPack at offset {offset}: {reason}")]
    St0903InvalidVTargetPack {
        offset: usize,
        reason: crate::klv::st0903::vtarget_pack::VTargetPackError,
    },

    /// A field-level validation error promoted to a fatal decode error.
    /// `klv::st0102::decode` raises this for InvalidLength on tag 1/2/12/22
    /// and InvalidUtf8 on ASCII string tags even in lenient mode (the
    /// graceful-fallback path is reserved for Tag 13 UTF-16 only —
    /// see the module-level rationale). `decode_strict` raises it for
    /// all field validation failures including unknown enum codepoints
    /// and Tag 13 UTF-16 decode failures.
    #[error("field validation failed: {0}")]
    FieldError(#[from] KlvFieldError),
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KlvEncodeError {
    #[error("output buffer too small: needed {needed} bytes, got {got}")]
    BufferTooSmall { needed: usize, got: usize },

    #[error("record exceeds maximum BER-encodable length")]
    RecordTooLarge,

    #[error("value out of range for tag {tag}: {value} not in [{min}, {max}]")]
    OutOfRange {
        tag: u32,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("string field for tag {tag} exceeds {max} bytes")]
    StringTooLong { tag: u32, max: usize },

    /// `ImapbParams::length` outside the supported range `1..=7`.
    /// ST 1201.5 defines IMAPB for any L-byte mapping, but this
    /// implementation uses i64 arithmetic internally, which overflows
    /// at L=8 (`signed_offset = 2^(8L-1)` would exceed `i64::MAX`);
    /// L=0 is degenerate. In-tree consumers use L ∈ {1,2,3,4,5,6}; if
    /// a future consumer needs L=8, swap the internal arithmetic to
    /// i128 / u64 with sign handling.
    #[error("IMAPB length {length} not supported (must be 1..=7)")]
    UnsupportedImapbLength { length: usize },
}

#[derive(Debug, Clone, Error, PartialEq)]
#[non_exhaustive]
pub enum KlvFieldError {
    #[error("tag {tag}: value {value} out of declared range [{min}, {max}]")]
    OutOfRange {
        tag: u32,
        value: f64,
        min: f64,
        max: f64,
    },

    #[error("tag {tag}: invalid UTF-8 in string field")]
    InvalidUtf8 { tag: u32 },

    #[error("tag {tag}: expected {expected} value bytes, got {got}")]
    InvalidLength {
        tag: u32,
        expected: usize,
        got: usize,
    },

    #[error("tag {tag}: value reserved as INVALID by spec")]
    InvalidSentinel { tag: u32 },

    /// Tag value declared as RFC 2781 UTF-16 contains malformed code
    /// units (lone surrogate, odd-length buffer). Reusable for any
    /// future UTF-16 / UCS-2 fields beyond ST 0102 Tag 13.
    #[error("tag {tag}: invalid UTF-16 in string field")]
    InvalidUtf16 { tag: u32 },

    /// Tag value's first byte is a typed-enum codepoint outside the
    /// spec's enumerated range. Strict-mode only; lenient decode
    /// surfaces an `Unknown(u8)` enum arm instead.
    #[error("tag {tag}: codepoint {value:#04x} not in spec-defined range")]
    InvalidCodepoint { tag: u32, value: u8 },

    /// Tag value's BER-declared length exceeded the available buffer,
    /// or its inner codec ran out of bytes. Surfaced by the lenient
    /// `klv::st0903::decode` walker when an LS body is truncated
    /// mid-field — the walker stops at the boundary, records this
    /// error, and returns the partially-populated record.
    #[error("tag {tag}: truncated value bytes")]
    TruncatedField { tag: u32 },

    /// `ImapbParams::length` outside the supported range `1..=7`.
    /// See `KlvEncodeError::UnsupportedImapbLength` for the rationale —
    /// the substrate caps at L=7 to keep its i64 internal arithmetic
    /// non-overflowing.
    #[error("IMAPB length {length} not supported (must be 1..=7)")]
    UnsupportedImapbLength { length: usize },
}

// ============================================================================
// MPEG-TS mux errors
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum MuxError {
    #[error("muxer configuration is invalid: {0}")]
    InvalidConfig(&'static str),

    #[error("video input is not Annex-B framed (no start code prefix)")]
    InvalidNal,

    #[error("muxer packet buffer is full ({capacity_packets} packets); drain via pull and retry")]
    BufferFull { capacity_packets: u64 },

    /// KLV blob exceeds the 16-bit `PES_packet_length` ceiling.
    ///
    /// PES_packet_length is at most 65535 and must cover flags1, flags2,
    /// header_data_length, the PTS field (if present), and the ES payload —
    /// so the KLV payload itself is bounded to 65532 bytes (no PTS) or
    /// 65527 bytes (with PTS). MISB ST 0601 packs are typically <2 KB so
    /// this is a sanity check, not a regular failure mode.
    #[error("KLV blob is {size} bytes, exceeds PES_packet_length ceiling of {max} bytes")]
    KlvTooLarge { size: usize, max: usize },

    /// Audio frames exceed the 16-bit `PES_packet_length` ceiling.
    ///
    /// PES_packet_length is at most 65535 and must cover flags1, flags2,
    /// header_data_length, the PTS field (always present for audio), and
    /// the ES payload — so the audio frames themselves are bounded to 65527 bytes.
    /// In practice audio frames are far smaller than this limit (KB scale), but
    /// the guard prevents silent wraparound on pathologically large inputs.
    #[error("audio frames too large: {size} bytes, max {max}")]
    AudioTooLarge { size: usize, max: usize },

    /// Caller passed a `VideoStreamHandle` / `KlvStreamHandle` that doesn't
    /// match a configured stream on this `Muxer`. Handles are obtained from
    /// `Muxer::video_handles()` / `klv_handles()` and are tied to the
    /// muxer that produced them — passing one from a different muxer is
    /// also rejected here.
    #[error("invalid {kind} stream handle (index {index}) — not a configured stream")]
    InvalidStreamHandle { kind: StreamKind, index: usize },

    /// Caller invoked the no-suffix `push_video` / `push_klv` (or the
    /// `MuxSender::send_video` / `send_klv` wrappers) on a muxer that has more
    /// than one stream of that kind. The single-target API can only resolve
    /// to a single handle when exactly one stream of that kind is configured.
    #[error(
        "ambiguous push: {count} {kind} streams configured — call push_{kind}_to(handle, ...) instead"
    )]
    AmbiguousTarget { kind: StreamKind, count: usize },

    /// `Muxer::push_klv` shorthand called when no KLV streams configured.
    /// Use `push_klv_to(handle, ...)` with a KLV handle from `klv_handles()`,
    /// or add a KLV stream to the config.
    #[error("no KLV streams configured; use push_klv_to with a handle from klv_handles()")]
    NoKlvStreamsConfigured,

    /// `Muxer::push_audio` shorthand called when no audio streams configured.
    /// Use `push_audio_to(handle, ...)` with an audio handle from `audio_handles()`,
    /// or add an audio stream to the config.
    #[error("no audio streams configured; use push_audio_to with a handle from audio_handles()")]
    NoAudioStreamsConfigured,

    /// `Muxer::push_subtitle` shorthand called when no subtitle streams configured.
    /// Use `push_subtitle_to(handle, ...)` with a subtitle handle from `subtitle_handles()`,
    /// or add a subtitle stream to the config.
    #[error(
        "no subtitle streams configured; use push_subtitle_to with a handle from subtitle_handles()"
    )]
    NoSubtitleStreamsConfigured,

    /// `MuxerConfig::validate` rejects more than 16 video streams.
    /// Trivially lifted if a consumer asks; 16 is well above realistic
    /// gimbaled-platform topologies (EO + IR + maybe IR-narrow + a depth
    /// channel = 4 in the wild today).
    #[error("too many video streams: {count} configured, cap is {cap}")]
    TooManyVideoStreams { count: usize, cap: usize },

    /// `MuxerConfig::validate` rejects more than 16 KLV streams.
    #[error("too many klv streams: {count} configured, cap is {cap}")]
    TooManyKlvStreams { count: usize, cap: usize },

    /// `MuxerConfig::validate` rejects more than 16 audio streams in any program.
    #[error("too many audio streams: {count} configured, cap is {cap}")]
    TooManyAudioStreams { count: usize, cap: usize },

    /// `MuxerConfig::validate` rejects more than 16 subtitle streams in any program.
    #[error("too many subtitle streams: {count} configured, cap is {cap}")]
    TooManySubtitleStreams { count: usize, cap: usize },

    /// `push_subtitle` payload exceeds the PES packet length budget. PES
    /// packet length is at most 65535 and must cover flags + PTS field +
    /// payload, bounding subtitle payloads to 65527 bytes.
    #[error("subtitle PES payload too large: {size} bytes (max {max})")]
    SubtitleTooLarge { size: usize, max: usize },

    /// Caller pinned a subtitle PID as the PCR PID. Subtitles are sparse
    /// and event-driven; using one for PCR pacing produces poor PCR
    /// spacing. Move PCR to a video / audio / KLV PID.
    #[error(
        "subtitle PID 0x{pid:04X} cannot be used as the PCR PID; subtitles are too sparse for PCR pacing"
    )]
    SubtitlePidUsedAsPcrPid { pid: u16 },

    /// PCR-PID resolved (caller-pinned or via fallback chain) to a KLV stream
    /// PID. KLV pushes are typically sparse (1-10 Hz from sensors) and would
    /// produce PCR at the same cadence, failing ETSI TR 101 290 §5.6.1's
    /// 100 ms ceiling. The right fix is to add a video stream to the program
    /// (PCR follows video naturally) or to caller-pin `pcr_pid` to a stream
    /// that pushes at ≥10 Hz. Today's deterministic-output muxer cannot emit
    /// standalone PCR-only TS packets between push events.
    #[error(
        "PCR PID 0x{pid:04X} resolves to a KLV stream — KLV push cadence is too sparse for PCR (ETSI TR 101 290 §5.6.1 requires ≤100 ms between PCRs); add a video stream or pin pcr_pid to a faster-cadence stream"
    )]
    KlvPidUsedAsPcrPid { pid: u16 },

    /// `MuxerConfig::validate` rejects ISO 639-2 language codes that aren't
    /// 3 lowercase ASCII bytes.
    #[error("invalid ISO 639-2 language code: {code:02x?} (must be 3 lowercase ASCII bytes)")]
    InvalidLanguageCode { code: [u8; 3] },

    /// `MuxerConfig::validate` rejects DVB teletext field values that exceed
    /// their bit-width budget.
    #[error("invalid DVB teletext {field}: {value} (max {max})")]
    InvalidTeletextField {
        field: TeletextField,
        value: u8,
        max: u8,
    },

    /// `MuxerConfig::validate` rejected a configuration whose total PMT
    /// section length wouldn't fit in a single TS packet. `used_bytes`
    /// is the estimated full PMT section size (header + program-level
    /// descriptors + per-stream entries with their auto-emit + caller-
    /// supplied descriptor bytes + CRC); `max_bytes` is the single-TS-
    /// packet payload cap (`MAX_PMT_SECTION_BYTES = 183`). Multi-section
    /// PMT support is out of scope; if you hit this, drop one or more
    /// user-supplied descriptors or shorten their payloads.
    #[error(
        "PMT too large: {used_bytes} bytes used, {max_bytes} max (single-section PMT must fit in one TS packet)"
    )]
    PmtTooLarge { used_bytes: usize, max_bytes: usize },

    /// Caller-supplied descriptor TLV bytes are not well-formed.
    /// Length byte must equal `data.len() - 2` and must not exceed 253.
    #[error(
        "malformed descriptor for stream {stream_index} descriptor {descriptor_index}: {reason}"
    )]
    MalformedDescriptor {
        stream_index: usize,
        descriptor_index: usize,
        reason: &'static str,
    },

    /// Configured `programs.len()` exceeded `MAX_PROGRAMS`.
    #[error("too many programs: {count} configured, cap is {cap}")]
    TooManyPrograms { count: usize, cap: usize },

    /// A program in `MuxerConfig::programs` had zero streams. Programs must
    /// carry at least one elementary stream.
    #[error("program {program_number} has no streams configured")]
    EmptyProgram { program_number: u16 },

    /// Two programs in `MuxerConfig::programs` share the same `program_number`.
    #[error("duplicate program_number {program_number} across programs")]
    DuplicateProgramNumber { program_number: u16 },

    /// Two programs in `MuxerConfig::programs` share the same `pmt_pid`.
    #[error("pmt_pid 0x{pid:04X} reused by programs {programs:?}")]
    DuplicatePmtPid { pid: u16, programs: [u16; 2] },

    /// A stream PID appears in two different programs. PID uniqueness across
    /// programs is required (repacking workflows control renumbering).
    #[error("stream PID 0x{pid:04X} used by programs {programs:?}")]
    DuplicatePidAcrossPrograms { pid: u16, programs: [u16; 2] },

    /// Caller referenced a program_number that doesn't exist in `MuxerConfig::programs`.
    #[error("program {program_number} not found")]
    ProgramNotFound { program_number: u16 },

    /// A program's `pmt_pid` collides with one of its own (or another program's)
    /// stream PIDs.
    #[error("pmt_pid 0x{pmt_pid:04X} of program {program_number} conflicts with a stream PID")]
    PmtPidConflictsWithStream { pmt_pid: u16, program_number: u16 },

    /// `MuxerConfig::validate` rejected a program that contains only subtitle
    /// streams. Subtitles must NOT carry PCR per ETSI EN 300 472 §4.0 +
    /// EN 300 743 §6.1; programs need ≥1 video / KLV / audio stream for
    /// PCR fallback resolution.
    #[error(
        "program {program_number} contains only subtitle streams; PCR cannot be resolved (subtitles must not carry PCR per EN 300 472 §4.0)"
    )]
    SubtitleOnlyProgram { program_number: u16 },

    /// A kind-relative descriptor index (as passed to
    /// `stream_descriptors_for_video` / `_for_klv` / `_for_audio` /
    /// `_for_subtitle`) is out of range for the corresponding stream list
    /// in the given program. Ensure the stream was added via `add_{kind}`
    /// before calling `stream_descriptors_for_{kind}`.
    #[error(
        "descriptor index {index} out of range for {kind} streams in program {program_number} \
         (call after the corresponding add_{kind})"
    )]
    DescriptorIndexOutOfRange {
        kind: StreamKind,
        index: u32,
        program_number: u16,
    },

    /// An absolute stream index (as passed to `stream_descriptors_for_stream`)
    /// is out of range for the given program's total stream count. `len` is
    /// the number of streams currently configured on the program.
    #[error("abs_idx {abs_idx} out of range for program {program_number} (has {len} streams)")]
    AbsIndexOutOfRange {
        abs_idx: u32,
        len: u32,
        program_number: u16,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_index_out_of_range_displays() {
        let err = MuxError::DescriptorIndexOutOfRange {
            kind: StreamKind::Video,
            index: 5,
            program_number: 1,
        };
        let s = err.to_string();
        assert!(s.contains("video"));
        assert!(s.contains("5"));
        assert!(s.contains("program 1"));
    }

    #[test]
    fn abs_index_out_of_range_displays() {
        let err = MuxError::AbsIndexOutOfRange {
            abs_idx: 99,
            len: 3,
            program_number: 1,
        };
        let s = err.to_string();
        assert!(s.contains("99"));
        assert!(s.contains("3 streams"));
        assert!(s.contains("program 1"));
    }
}

// ============================================================================
// MPEG-TS demux errors
// ============================================================================

/// Errors emitted by `mpegts::demux`.
///
/// Lenient-mode demuxing typically does NOT return errors — non-conformance
/// surfaces as `DemuxEvent::NonConformant { issue }` so the receive loop
/// keeps running. The error variants below fire when something is genuinely
/// fatal (the byte stream is unrecoverable, or strict mode converts a
/// `NonConformantIssue` into a hard failure).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum DemuxError {
    /// Byte stream is unrecoverable: too few bytes after a long sync-search
    /// window to make progress, or repeated PSI checksum failures.
    #[error("demuxer cannot recover sync after {after_bytes} bytes")]
    Unrecoverable { after_bytes: usize },

    /// Strict mode rejected a `NonConformantIssue`. Lenient mode would have
    /// emitted a `NonConformant` event instead and continued.
    #[error("strict-mode rejection: {0}")]
    StrictRejection(String),

    /// PSI section claimed a length that doesn't fit a valid PAT/PMT.
    /// Distinct from a checksum mismatch (which is `NonConformant` in
    /// lenient mode); this is structurally impossible.
    #[error("malformed PSI section at PID 0x{pid:04X}: {reason}")]
    MalformedPsi { pid: u16, reason: &'static str },

    /// PES header at PID 0x{pid:04X} declared a length that's too short to
    /// contain its own claimed flags. Unlike PSI checksum failures (which
    /// surface as `NonConformant` in lenient mode), this prevents the
    /// reassembler from making any forward progress.
    #[error("malformed PES header at PID 0x{pid:04X}: {reason}")]
    MalformedPes { pid: u16, reason: &'static str },

    /// The demuxer's pre-sync buffer (`Demuxer::sync_buf`) exceeded its
    /// hard ceiling. Fired when a peer feeds bytes with no 0x47 sync byte
    /// faster than the sync-search window can scan them — `feed` runs
    /// `extend_from_slice` up front, so a single oversized call would
    /// otherwise allocate the whole input before the per-loop window
    /// check could bail. The cap matches ffmpeg's `MpegTSSectionFilter`
    /// (4 MiB). On this error the demuxer drops `sync_buf` to release the
    /// adversarial bytes; the caller's only sane response is to teardown
    /// the demuxer or accept that subsequent reads will not align.
    #[error("demuxer sync buffer exhausted: {observed} bytes exceeds {max} byte ceiling")]
    SyncBufExhausted { observed: usize, max: usize },
}
