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

// ------------------------------------------------------------------
// Subordinate list-element structs (Task 7)
// ------------------------------------------------------------------

/// `repr(C)` mirror of `tst_core::mpegts::demux::NalUnit`.
///
/// Three Rust variants (H264 / H265 / H266) collapsed into one C
/// struct; field semantics keyed by the parent `tst_event_t.u.sample.codec`.
/// * H.264: `nal_type` 5-bit; `ref_idc_or_layer_id` is `ref_idc`;
///   `temporal_id_plus1` is `0` (H.264 has no temporal_id).
/// * H.265 / H.266: `nal_type` 6-bit (H.265) or 5-bit (H.266);
///   `ref_idc_or_layer_id` is `nuh_layer_id`; `temporal_id_plus1`
///   is the temporal-id field +1 (per spec).
///
/// `payload` borrows from the demuxer's NAL-unit Vec; valid until
/// the next `_recv_event` / `_close` call on this handle.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstNal {
    pub nal_type: u8,
    pub ref_idc_or_layer_id: u8,
    pub temporal_id_plus1: u8,
    pub _reserved: u8,
    pub payload: *const u8,
    pub payload_len: usize,
}

const _TST_NAL_SIZE: () = assert!(
    std::mem::size_of::<TstNal>() == 24,
    "TstNal must be 24 bytes (4 bytes header + 8 bytes pointer + 8 bytes len)"
);

/// `repr(C)` mirror of `tst_core::mpegts::demux::Obu`.
///
/// `has_extension` is 0 or 1; `temporal_id` and `spatial_id` are
/// valid only when `has_extension == 1`. `payload` is the OBU body
/// (header byte + extension byte + LEB128 size already stripped).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstObu {
    pub obu_type: u8,
    pub has_extension: u8,
    pub temporal_id: u8,
    pub spatial_id: u8,
    pub payload: *const u8,
    pub payload_len: usize,
}

const _TST_OBU_SIZE: () = assert!(
    std::mem::size_of::<TstObu>() == 24,
    "TstObu must be 24 bytes"
);

/// `repr(C)` mirror of `tst_core::mpegts::demux::psi::RawDescriptor`.
///
/// `data` borrows from the demuxer's per-PMT descriptor list; valid
/// until the next `_recv_event` / `_close` call on this handle. The
/// length byte from the wire is stripped — `data_len` is the body length.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstDescriptor {
    pub tag: u8,
    pub _reserved: [u8; 7],
    pub data: *const u8,
    pub data_len: usize,
}

const _TST_DESCRIPTOR_SIZE: () = assert!(
    std::mem::size_of::<TstDescriptor>() == 24,
    "TstDescriptor must be 24 bytes"
);

/// `repr(C)` mirror of `tst_core::mpegts::demux::StreamInfo`.
///
/// `stream_kind` is `TST_STREAM_KIND_*` (see `TstStreamKindTag`);
/// `codec` is `TST_VIDEO_CODEC_*` / `TST_AUDIO_CODEC_*` /
/// `TST_SUBTITLE_CODEC_*` keyed by `stream_kind`, or `-1` when
/// `stream_kind` is KlvSync / KlvAsync / Unknown.
///
/// `raw_descriptors` borrows from the demuxer's per-PMT descriptor
/// list; valid until the next `_recv_event` / `_close` call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstStreamInfo {
    pub pid: u16,
    pub stream_type: u8,
    pub _pad: u8,
    pub stream_kind: c_int,
    pub codec: c_int,
    pub program_number: u16,
    pub _pad2: [u8; 6],
    pub raw_descriptors: *const TstDescriptor,
    pub descriptor_count: usize,
}

const _TST_STREAM_INFO_SIZE: () = assert!(
    std::mem::size_of::<TstStreamInfo>() == 40,
    "TstStreamInfo must be 40 bytes"
);

/// `repr(C)` mirror of `tst_core::mpegts::demux::KlvLink`.
/// `source` is `TST_LINK_SOURCE_*` (see `TstLinkSource`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstKlvLink {
    pub klv_pid: u16,
    pub video_pid: u16,
    pub source: c_int,
}

const _TST_KLV_LINK_SIZE: () = assert!(
    std::mem::size_of::<TstKlvLink>() == 8,
    "TstKlvLink must be 8 bytes"
);

// ------------------------------------------------------------------
// TstEvent tagged union (Task 8)
// ------------------------------------------------------------------

/// Per-event-kind union body for `TstEvent`. cbindgen emits this as
/// `union { ... } u` on the C side; each kind's fields are a flat
/// nested struct. Fields not relevant to the active `kind` are zero
/// or null after `convert()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union TstEventBody {
    pub program_map: TstEventProgramMap,
    pub sample: TstEventSample,
    pub metadata: TstEventMetadata,
    pub discontinuity: TstEventDiscontinuity,
    pub nonconformant: TstEventNonConformant,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEventProgramMap {
    pub program_number: u16,
    pub pcr_pid: u16,
    pub _pad: [u8; 4],
    pub streams: *const TstStreamInfo,
    pub stream_count: usize,
    pub klv_links: *const TstKlvLink,
    pub klv_link_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEventSample {
    pub pid: u16,
    pub program_number: u16,
    pub stream_kind: c_int,
    pub pts: i64,
    pub dts: i64, // INT64_MIN if absent
    pub codec: c_int,
    pub _pad: [u8; 4],
    pub nals: *const TstNal,
    pub nal_count: usize,
    pub obus: *const TstObu,
    pub obu_count: usize,
    pub payload: *const u8,
    pub payload_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEventMetadata {
    pub pid: u16,
    pub program_number: u16,
    pub _pad: [u8; 4],
    pub pts: i64,
    pub metadata_kind: c_int,
    pub _pad2: [u8; 4],
    pub payload: *const u8,
    pub payload_len: usize,
    // KlvSyncAuCell-only fields (zero/false for other kinds)
    pub metadata_service_id: u8,
    pub sequence_number: u8,
    pub cell_fragment_indication: u8, // 0=Middle, 1=Last, 2=First, 3=Complete
    pub decoder_config_flag: u8,
    pub random_access_indicator: u8,
    pub _pad3: [u8; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEventDiscontinuity {
    pub pid: u16,
    pub _pad: [u8; 2],
    pub discontinuity_kind: c_int,
    pub cc_expected: u8,
    pub cc_observed: u8,
    pub _pad2: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEventNonConformant {
    pub pid: u16,
    pub _pad: [u8; 2],
    pub issue_code: c_int,
    pub pcr_delta: i64,
    pub table_id: u8,
    pub last_section_number: u8,
    pub cc_expected: u8,
    pub cc_observed: u8,
    pub _pad2: [u8; 4],
    pub observed_len: usize,
    pub obu_type: u8,
    pub _pad3: [u8; 7],
    pub programs: *const u16,    // PidReusedAcrossPrograms (len 2)
    pub tags: *const u8,         // SubtitleDescriptorAmbiguous
    pub tag_count: usize,
    pub detail: *const c_char,   // Other(String); also human-readable summary
}

/// Top-level event struct. Caller stack-allocates and passes
/// `&mut TstEvent` to `tst_demux_receiver_recv_event`. After a
/// success return, `kind` selects which `u.*` body is populated;
/// pointer fields on the active body borrow from the receiver's
/// `EventArena` until the next `_recv_event` / `_close` call.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TstEvent {
    pub kind: c_int,
    pub _pad: [u8; 4],
    pub u: TstEventBody,
}

const _TST_EVENT_SIZE_REASONABLE: () = assert!(
    std::mem::size_of::<TstEvent>() <= 256,
    "TstEvent should fit in 256 bytes for stack-allocation comfort"
);

impl Default for TstEvent {
    fn default() -> Self {
        // SAFETY: zeroed union is valid for all our variants (every
        // payload struct is plain old data + nullable pointers).
        unsafe { std::mem::zeroed() }
    }
}

// ------------------------------------------------------------------
// EventArena — reusable per-handle backing storage
// ------------------------------------------------------------------

/// Per-handle arena owning the backing buffers pointed to by `TstEvent`
/// pointer fields. Reused across `_recv_event` calls — zero-alloc in
/// steady state once the Vecs reach their working size.
///
/// All Vec fields are cleared (not dropped) at the start of each
/// `convert()` call; capacity is retained.
#[allow(dead_code)]
pub(crate) struct EventArena {
    pub(crate) nals: Vec<TstNal>,
    pub(crate) obus: Vec<TstObu>,
    pub(crate) descriptors: Vec<TstDescriptor>,
    pub(crate) stream_infos: Vec<TstStreamInfo>,
    pub(crate) klv_links: Vec<TstKlvLink>,
    /// CString buffer for NonConformant::Other(String).detail; one
    /// per convert() call.
    pub(crate) detail_buf: Vec<u8>,
    /// Programs array for PidReusedAcrossPrograms (always len 2 when used).
    pub(crate) programs_buf: [u16; 2],
    /// Tags array for SubtitleDescriptorAmbiguous.
    pub(crate) tags_buf: Vec<u8>,
}

#[allow(dead_code)]
impl EventArena {
    pub(crate) fn new() -> Self {
        Self {
            nals: Vec::new(),
            obus: Vec::new(),
            descriptors: Vec::new(),
            stream_infos: Vec::new(),
            klv_links: Vec::new(),
            detail_buf: Vec::new(),
            programs_buf: [0; 2],
            tags_buf: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.nals.clear();
        self.obus.clear();
        self.descriptors.clear();
        self.stream_infos.clear();
        self.klv_links.clear();
        self.detail_buf.clear();
        self.programs_buf = [0; 2];
        self.tags_buf.clear();
    }
}

// ------------------------------------------------------------------
// convert() — DemuxEvent → TstEvent
// ------------------------------------------------------------------

/// Convert a `tst_core` `DemuxEvent` into a populated `TstEvent`,
/// using `arena` as backing storage for pointer-valued fields.
///
/// On return, `out` is fully populated:
/// * `out.kind` set to the corresponding `TstEventKind`.
/// * The active `u.*` body's fields filled; inactive bodies' contents
///   are unspecified (they share storage via the union).
/// * Pointer fields point into the arena's Vec buffers; valid until
///   the next `convert()` call clears the arena.
#[allow(dead_code)]
pub(crate) fn convert(
    arena: &mut EventArena,
    ev: &tst_core::mpegts::demux::DemuxEvent,
    out: &mut TstEvent,
) {
    use tst_core::mpegts::demux::DemuxEvent;
    arena.clear();
    *out = TstEvent::default();
    match ev {
        DemuxEvent::ProgramMap(pm) => fill_program_map(arena, pm, out),
        DemuxEvent::Sample { stream, pts, dts, payload } => {
            fill_sample(arena, stream, *pts, *dts, payload, out);
        }
        DemuxEvent::Metadata { stream, pts, kind, payload } => {
            fill_metadata(stream, *pts, kind, payload, out);
        }
        DemuxEvent::Discontinuity { stream, kind } => {
            fill_discontinuity(stream, kind, out);
        }
        DemuxEvent::NonConformant { stream, issue } => {
            fill_nonconformant(arena, stream, issue, out);
        }
    }
}

fn fill_program_map(
    arena: &mut EventArena,
    pm: &tst_core::mpegts::demux::ProgramMap,
    out: &mut TstEvent,
) {
    // Populate descriptors first (one flat Vec across all streams);
    // each StreamInfo references a slice via pointer + count.
    let mut per_stream_desc_ranges: Vec<(usize, usize)> = Vec::with_capacity(pm.streams.len());
    for si in &pm.streams {
        let start = arena.descriptors.len();
        for d in &si.raw_descriptors {
            arena.descriptors.push(TstDescriptor {
                tag: d.tag,
                _reserved: [0; 7],
                data: d.data.as_ptr(),
                data_len: d.data.len(),
            });
        }
        per_stream_desc_ranges.push((start, arena.descriptors.len() - start));
    }
    let descriptors_base = arena.descriptors.as_ptr();
    for (si, (start, count)) in pm.streams.iter().zip(per_stream_desc_ranges.iter()) {
        let (kind_tag, codec_int) = stream_kind_to_c(&si.kind);
        arena.stream_infos.push(TstStreamInfo {
            pid: si.pid,
            stream_type: si.stream_type,
            _pad: 0,
            stream_kind: kind_tag,
            codec: codec_int,
            program_number: si.program_number,
            _pad2: [0; 6],
            // SAFETY: descriptors_base offset is in bounds; we built
            // both arrays in this function and the base pointer is
            // stable until arena.clear() in the next convert() call.
            raw_descriptors: unsafe { descriptors_base.add(*start) },
            descriptor_count: *count,
        });
    }
    for link in &pm.klv_links {
        arena.klv_links.push(TstKlvLink {
            klv_pid: link.klv_pid,
            video_pid: link.video_pid,
            source: link_source_to_c(link.source),
        });
    }
    out.kind = TstEventKind::ProgramMap as c_int;
    out.u.program_map = TstEventProgramMap {
        program_number: pm.program_number,
        pcr_pid: pm.pcr_pid,
        _pad: [0; 4],
        streams: arena.stream_infos.as_ptr(),
        stream_count: arena.stream_infos.len(),
        klv_links: arena.klv_links.as_ptr(),
        klv_link_count: arena.klv_links.len(),
    };
}

fn fill_sample(
    arena: &mut EventArena,
    stream: &tst_core::mpegts::demux::StreamId,
    pts: i64,
    dts: Option<i64>,
    payload: &tst_core::mpegts::demux::SamplePayload,
    out: &mut TstEvent,
) {
    use tst_core::mpegts::demux::{SamplePayload, VideoPayload};
    let (kind_tag, _codec_int) = stream_kind_to_c(&stream.kind);
    let mut codec = -1i32;
    let mut nals_ptr: *const TstNal = std::ptr::null();
    let mut nal_count: usize = 0;
    let mut obus_ptr: *const TstObu = std::ptr::null();
    let mut obu_count: usize = 0;
    let mut payload_ptr: *const u8 = std::ptr::null();
    let mut payload_len: usize = 0;
    match payload {
        SamplePayload::Video { codec: vc, payload: vp } => {
            codec = crate::config::TstVideoCodec::from_core(*vc) as i32;
            match vp {
                VideoPayload::Nals(nals) => {
                    for n in nals {
                        arena.nals.push(nal_to_c(n));
                    }
                    nals_ptr = arena.nals.as_ptr();
                    nal_count = arena.nals.len();
                }
                VideoPayload::Obus(obus) => {
                    for o in obus {
                        arena.obus.push(obu_to_c(o));
                    }
                    obus_ptr = arena.obus.as_ptr();
                    obu_count = arena.obus.len();
                }
            }
        }
        SamplePayload::Audio { codec: ac, frames } => {
            codec = crate::config::TstAudioCodec::from_core(*ac) as i32;
            payload_ptr = frames.as_ptr();
            payload_len = frames.len();
        }
        SamplePayload::Subtitle { codec: sc, payload: pl } => {
            codec = crate::config::TstSubtitleCodec::from_core(*sc) as i32;
            payload_ptr = pl.as_ptr();
            payload_len = pl.len();
        }
        SamplePayload::Unknown { stream_type: _, raw } => {
            // codec stays -1; stream_kind == Unknown carries the stream_type
            // via the per-stream PMT entry rather than here.
            payload_ptr = raw.as_ptr();
            payload_len = raw.len();
        }
    }
    out.kind = TstEventKind::Sample as c_int;
    out.u.sample = TstEventSample {
        pid: stream.pid,
        program_number: 0, // TODO: thread program_number through StreamId per design §7.3
        stream_kind: kind_tag,
        pts,
        dts: dts.unwrap_or(i64::MIN),
        codec,
        _pad: [0; 4],
        nals: nals_ptr,
        nal_count,
        obus: obus_ptr,
        obu_count,
        payload: payload_ptr,
        payload_len,
    };
}

fn fill_metadata(
    stream: &tst_core::mpegts::demux::StreamId,
    pts: i64,
    kind: &tst_core::mpegts::demux::MetadataKind,
    payload: &[u8],
    out: &mut TstEvent,
) {
    use tst_core::mpegts::demux::MetadataKind;
    let mut metadata_service_id = 0u8;
    let mut sequence_number = 0u8;
    let mut cell_fragment_indication = 0u8;
    let mut decoder_config_flag = 0u8;
    let mut random_access_indicator = 0u8;
    let md_kind = match kind {
        MetadataKind::KlvSyncAuCell {
            metadata_service_id: sid,
            sequence_number: seq,
            cell_fragment_indication: cfi,
            decoder_config_flag: dcf,
            random_access_indicator: rai,
        } => {
            metadata_service_id = *sid;
            sequence_number = *seq;
            cell_fragment_indication = *cfi as u8;
            decoder_config_flag = *dcf as u8;
            random_access_indicator = *rai as u8;
            TstMetadataKindTag::KlvSyncAuCell as c_int
        }
        MetadataKind::KlvAsync => TstMetadataKindTag::KlvAsync as c_int,
        MetadataKind::Unknown(_) => TstMetadataKindTag::Unknown as c_int,
    };
    out.kind = TstEventKind::Metadata as c_int;
    out.u.metadata = TstEventMetadata {
        pid: stream.pid,
        program_number: 0, // TODO: thread program_number through StreamId per design §7.3
        _pad: [0; 4],
        pts,
        metadata_kind: md_kind,
        _pad2: [0; 4],
        payload: payload.as_ptr(),
        payload_len: payload.len(),
        metadata_service_id,
        sequence_number,
        cell_fragment_indication,
        decoder_config_flag,
        random_access_indicator,
        _pad3: [0; 3],
    };
}

fn fill_discontinuity(
    stream: &tst_core::mpegts::demux::StreamId,
    kind: &tst_core::mpegts::demux::DiscontinuityKind,
    out: &mut TstEvent,
) {
    use tst_core::mpegts::demux::DiscontinuityKind;
    let (tag, cc_expected, cc_observed) = match kind {
        DiscontinuityKind::ContinuityJump { expected, observed } => (
            TstDiscontinuityKindTag::ContinuityJump as c_int,
            *expected,
            *observed,
        ),
        DiscontinuityKind::PesOversize { pid: _ } => {
            (TstDiscontinuityKindTag::PesOversize as c_int, 0, 0)
        }
        DiscontinuityKind::PesTotalOversize => {
            (TstDiscontinuityKindTag::PesTotalOversize as c_int, 0, 0)
        }
        DiscontinuityKind::AdaptationFieldFlag => {
            (TstDiscontinuityKindTag::AdaptationFieldFlag as c_int, 0, 0)
        }
    };
    out.kind = TstEventKind::Discontinuity as c_int;
    out.u.discontinuity = TstEventDiscontinuity {
        pid: stream.pid,
        _pad: [0; 2],
        discontinuity_kind: tag,
        cc_expected,
        cc_observed,
        _pad2: [0; 6],
    };
}

fn fill_nonconformant(
    arena: &mut EventArena,
    stream: &tst_core::mpegts::demux::StreamId,
    issue: &tst_core::mpegts::demux::NonConformantIssue,
    out: &mut TstEvent,
) {
    use tst_core::mpegts::demux::NonConformantIssue;
    let mut body = TstEventNonConformant {
        pid: stream.pid,
        _pad: [0; 2],
        issue_code: TstNonConformantCode::Other as c_int,
        pcr_delta: 0,
        table_id: 0,
        last_section_number: 0,
        cc_expected: 0,
        cc_observed: 0,
        _pad2: [0; 4],
        observed_len: 0,
        obu_type: 0,
        _pad3: [0; 7],
        programs: std::ptr::null(),
        tags: std::ptr::null(),
        tag_count: 0,
        detail: std::ptr::null(),
    };
    match issue {
        NonConformantIssue::StreamTypeMismatchSyncOnAsyncPid => {
            body.issue_code = TstNonConformantCode::StreamTypeMismatchSyncOnAsyncPid as c_int;
        }
        NonConformantIssue::StreamTypeMismatchAsyncOnSyncPid => {
            body.issue_code = TstNonConformantCode::StreamTypeMismatchAsyncOnSyncPid as c_int;
        }
        NonConformantIssue::MissingMetadataDescriptor => {
            body.issue_code = TstNonConformantCode::MissingMetadataDescriptor as c_int;
        }
        NonConformantIssue::PcrAnomaly { delta } => {
            body.issue_code = TstNonConformantCode::PcrAnomaly as c_int;
            body.pcr_delta = *delta;
        }
        NonConformantIssue::PsiChecksumMismatch { pid } => {
            body.issue_code = TstNonConformantCode::PsiChecksumMismatch as c_int;
            body.pid = *pid;
        }
        NonConformantIssue::PusiMidPes => {
            body.issue_code = TstNonConformantCode::PusiMidPes as c_int;
        }
        NonConformantIssue::PidReusedAcrossPrograms { pid, programs } => {
            body.issue_code = TstNonConformantCode::PidReusedAcrossPrograms as c_int;
            body.pid = *pid;
            arena.programs_buf = *programs;
            body.programs = arena.programs_buf.as_ptr();
        }
        NonConformantIssue::SubtitleMissingDescriptor { pid } => {
            body.issue_code = TstNonConformantCode::SubtitleMissingDescriptor as c_int;
            body.pid = *pid;
        }
        NonConformantIssue::SubtitleDescriptorAmbiguous { pid, tags } => {
            body.issue_code = TstNonConformantCode::SubtitleDescriptorAmbiguous as c_int;
            body.pid = *pid;
            arena.tags_buf.extend_from_slice(tags);
            body.tags = arena.tags_buf.as_ptr();
            body.tag_count = arena.tags_buf.len();
        }
        NonConformantIssue::SubtitleDescriptorMalformed { pid, tag } => {
            body.issue_code = TstNonConformantCode::SubtitleDescriptorMalformed as c_int;
            body.pid = *pid;
            body.table_id = *tag; // reuse table_id field as the descriptor tag carrier
        }
        NonConformantIssue::Av1RegistrationMalformed { pid } => {
            body.issue_code = TstNonConformantCode::Av1RegistrationMalformed as c_int;
            body.pid = *pid;
        }
        NonConformantIssue::Av1ObuMissingSizeField { pid, obu_type } => {
            body.issue_code = TstNonConformantCode::Av1ObuMissingSizeField as c_int;
            body.pid = *pid;
            body.obu_type = *obu_type;
        }
        NonConformantIssue::Av1TileListNotAllowed { pid } => {
            body.issue_code = TstNonConformantCode::Av1TileListNotAllowed as c_int;
            body.pid = *pid;
        }
        NonConformantIssue::PsiOverlongSection { pid, observed_len } => {
            body.issue_code = TstNonConformantCode::PsiOverlongSection as c_int;
            body.pid = *pid;
            body.observed_len = *observed_len;
        }
        NonConformantIssue::TransportErrorPacket { pid } => {
            body.issue_code = TstNonConformantCode::TransportErrorPacket as c_int;
            body.pid = *pid;
        }
        NonConformantIssue::PsiCcDiscontinuity { pid, expected, observed } => {
            body.issue_code = TstNonConformantCode::PsiCcDiscontinuity as c_int;
            body.pid = *pid;
            body.cc_expected = *expected;
            body.cc_observed = *observed;
        }
        NonConformantIssue::MultiCellAu { pid, dropped_bytes } => {
            body.issue_code = TstNonConformantCode::MultiCellAu as c_int;
            body.pid = *pid;
            body.observed_len = *dropped_bytes;
        }
        NonConformantIssue::PsiMultiSectionUnsupported { pid, table_id, last_section_number } => {
            body.issue_code = TstNonConformantCode::PsiMultiSectionUnsupported as c_int;
            body.pid = *pid;
            body.table_id = *table_id;
            body.last_section_number = *last_section_number;
        }
        NonConformantIssue::Other(s) => {
            body.issue_code = TstNonConformantCode::Other as c_int;
            arena.detail_buf.extend_from_slice(s.as_bytes());
            arena.detail_buf.push(0); // NUL terminator
            body.detail = arena.detail_buf.as_ptr() as *const c_char;
        }
    }
    out.kind = TstEventKind::NonConformant as c_int;
    out.u.nonconformant = body;
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn stream_kind_to_c(kind: &tst_core::mpegts::demux::StreamKind) -> (c_int, c_int) {
    use tst_core::mpegts::demux::StreamKind;
    match kind {
        StreamKind::Video(c) => (
            TstStreamKindTag::Video as c_int,
            crate::config::TstVideoCodec::from_core(*c) as c_int,
        ),
        StreamKind::Audio(c) => (
            TstStreamKindTag::Audio as c_int,
            crate::config::TstAudioCodec::from_core(*c) as c_int,
        ),
        StreamKind::Subtitle(c) => (
            TstStreamKindTag::Subtitle as c_int,
            crate::config::TstSubtitleCodec::from_core(*c) as c_int,
        ),
        StreamKind::KlvSync { .. } => (TstStreamKindTag::KlvSync as c_int, -1),
        StreamKind::KlvAsync => (TstStreamKindTag::KlvAsync as c_int, -1),
        StreamKind::Unknown(_) => (TstStreamKindTag::Unknown as c_int, -1),
    }
}

fn link_source_to_c(s: tst_core::mpegts::demux::LinkSource) -> c_int {
    use tst_core::mpegts::demux::LinkSource;
    match s {
        LinkSource::Declared => TstLinkSource::Declared as c_int,
        LinkSource::Inferred => TstLinkSource::Inferred as c_int,
        LinkSource::Override => TstLinkSource::Override as c_int,
    }
}

fn nal_to_c(n: &tst_core::mpegts::demux::NalUnit) -> TstNal {
    use tst_core::mpegts::demux::NalUnit;
    match n {
        NalUnit::H264 { nal_type, ref_idc, payload } => TstNal {
            nal_type: *nal_type,
            ref_idc_or_layer_id: *ref_idc,
            temporal_id_plus1: 0,
            _reserved: 0,
            payload: payload.as_ptr(),
            payload_len: payload.len(),
        },
        NalUnit::H265 { nal_type, layer_id, temporal_id_plus1, payload }
        | NalUnit::H266 { nal_type, layer_id, temporal_id_plus1, payload } => TstNal {
            nal_type: *nal_type,
            ref_idc_or_layer_id: *layer_id,
            temporal_id_plus1: *temporal_id_plus1,
            _reserved: 0,
            payload: payload.as_ptr(),
            payload_len: payload.len(),
        },
    }
}

fn obu_to_c(o: &tst_core::mpegts::demux::Obu) -> TstObu {
    match &o.extension {
        Some(ext) => TstObu {
            obu_type: o.obu_type,
            has_extension: 1,
            temporal_id: ext.temporal_id,
            spatial_id: ext.spatial_id,
            payload: o.payload.as_ptr(),
            payload_len: o.payload.len(),
        },
        None => TstObu {
            obu_type: o.obu_type,
            has_extension: 0,
            temporal_id: 0,
            spatial_id: 0,
            payload: o.payload.as_ptr(),
            payload_len: o.payload.len(),
        },
    }
}
