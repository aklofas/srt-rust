"""tstrans.mpegts — MPEG-TS packet, PES, PSI, muxer, demuxer.

Phase 2 of the tst-py v1 plan added the demuxer surface:

- `Pts90khz` — 90 kHz timestamp wrapper
- `VideoCodec`, `AudioCodec`, `SubtitleCodec`, `StrictMode` enums
- `StreamId`, `StreamInfo`, `KlvLink`, `ProgramMap` dataclasses
- `DemuxEvent` base + subclasses (`ProgramMap`, `Video`, `Audio`,
  `Subtitle`, `Klv`, `Discontinuity`, `NonConformant`,
  `ReconnectDiscontinuity`)
- `DemuxerConfig`, `Demuxer` — feed bytes, get events

Phase 4 adds `Muxer` + `MuxerConfig` here.
"""

import enum
from dataclasses import dataclass
from typing import ClassVar, Optional


@dataclass(frozen=True, slots=True)
class Pts90khz:
    """A 90 kHz timestamp tick count, the MPEG-TS PTS unit.

    Wraps an `i64` to allow signed-diff arithmetic (per the Rust
    `tst_core::mpegts::common::Pts90khz`). Construct via
    `Pts90khz.from_raw(int)`, `Pts90khz.from_ms(int)`, or
    `Pts90khz.from_seconds(float)`.
    """

    raw: int

    @classmethod
    def from_raw(cls, ticks: int) -> "Pts90khz":
        return cls(raw=int(ticks))

    @classmethod
    def from_ms(cls, ms: int) -> "Pts90khz":
        return cls(raw=int(ms) * 90)

    @classmethod
    def from_seconds(cls, seconds: float) -> "Pts90khz":
        return cls(raw=int(seconds * 90_000))

    @property
    def ms(self) -> int:
        # Truncating divide — matches Rust's integer arithmetic.
        return self.raw // 90

    @property
    def seconds(self) -> float:
        return self.raw / 90_000.0

    def __repr__(self) -> str:
        return f"Pts90khz(raw={self.raw}, ms={self.ms})"


class VideoCodec(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::demux::event::VideoCodec`."""
    H264 = "h264"
    H265 = "h265"
    H266 = "h266"
    AV1 = "av1"


class AudioCodec(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::demux::event::AudioCodec`."""
    MP2 = "mp2"
    AAC = "aac"
    AAC_LATM = "aac_latm"
    AC3 = "ac3"


class SubtitleCodec(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::demux::event::SubtitleCodec`."""
    DVB_SUBTITLING = "dvb_subtitling"
    DVB_TELETEXT = "dvb_teletext"
    CEA708_STANDALONE = "cea708_standalone"
    WEBVTT_IN_TS = "webvtt_in_ts"


class KlvStreamType(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::mux::KlvStreamType`. Picks the
    PMT stream_type byte (and PES wrap shape) for KLV streams:
    `SYNCHRONOUS_METADATA` (0x15) is strict ST 1402 sync and triggers
    a 5-byte H.222.0 §2.12.4.2 Metadata_AU_cell wrapper (auto-prepended
    by the muxer — callers pass raw LS bytes); `PRIVATE_DATA` (0x06)
    is the broadly-recognized form (pass-through, no AU cell wrap).

    Whether the KLV PES carries a PTS is controlled separately via the
    `carries_pts` field on `KlvStreamSpec`.
    """

    PRIVATE_DATA = "private_data"
    SYNCHRONOUS_METADATA = "synchronous_metadata"


class Av1CarriageMode(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::mux::Av1CarriageMode`. Default
    is `MPEG2_TS_BINDING` — AV1-in-MPEG-2-TS binding conformant
    carriage (PES `stream_id=0xBD`, OBUs wrapped in
    `ts_open_bitstream_unit()` framing).

    `INTEROP_RAW_OBU` is the interoperability mode for the
    ffmpeg / libaom / hls.js / mediamtx AV1-in-TS toolchain — PES
    `stream_id=0xE0` and raw OBU payload (no
    `ts_open_bitstream_unit` framing). Non-conformant per the binding
    spec, but matches the de facto carriage used by those tools today.

    The symmetric setting on the demuxer (`DemuxerConfig.av1_carriage`)
    MUST match for a successful round-trip.
    """

    MPEG2_TS_BINDING = "mpeg2_ts_binding"
    INTEROP_RAW_OBU = "interop_raw_obu"


# StreamSpec ABC + 4 concrete subclasses — match-statement-compat
# tagged union, same pattern as Phase 2's DemuxEvent hierarchy.
# Mirrors Rust `tst_core::mpegts::mux::StreamSpec`'s variants.

@dataclass(frozen=True, slots=True)
class StreamSpec:
    """Abstract base for elementary-stream specs within a program.

    Concrete subclasses: `VideoStreamSpec`, `KlvStreamSpec`,
    `AudioStreamSpec`, `SubtitleStreamSpec`. Frozen + slotted so
    instances are hashable, value-equal, and immutable.
    """

    pid: int


@dataclass(frozen=True, slots=True)
class VideoStreamSpec(StreamSpec):
    """Video elementary stream — `pid` + `codec`."""

    codec: VideoCodec


@dataclass(frozen=True, slots=True)
class KlvStreamSpec(StreamSpec):
    """KLV metadata elementary stream — `pid`, `stream_type`
    (`PRIVATE_DATA` 0x06 or `SYNCHRONOUS_METADATA` 0x15), and
    `carries_pts` (whether to emit a PTS in the PES header)."""

    stream_type: KlvStreamType
    carries_pts: bool


@dataclass(frozen=True, slots=True)
class AudioStreamSpec(StreamSpec):
    """Audio elementary stream — `pid`, `codec`, optional `language`
    (3-byte ISO 639-2 lowercase ASCII, e.g. `b"eng"`; None omits the
    ISO 639 language descriptor)."""

    codec: AudioCodec
    language: Optional[bytes] = None


@dataclass(frozen=True, slots=True)
class SubtitleStreamSpec(StreamSpec):
    """Subtitle / caption elementary stream — `pid` + `codec`. The
    codec value itself carries any per-variant parameters (language,
    DVB subtitling_type, etc.)."""

    codec: SubtitleCodec


class StreamKindTag(enum.Enum):
    """Discriminator for `StreamKind`. The actual codec (when applicable)
    lives on the `codec` field of `StreamInfo` / `StreamId`."""
    VIDEO = "video"
    AUDIO = "audio"
    SUBTITLE = "subtitle"
    KLV_SYNC = "klv_sync"
    KLV_ASYNC = "klv_async"
    UNKNOWN = "unknown"


class MetadataKindTag(enum.Enum):
    """Discriminator for `DemuxEvent.Klv.kind`. Mirrors Rust
    `MetadataKind` collapsed to its variant tag."""
    KLV_SYNC_AU_CELL = "klv_sync_au_cell"
    KLV_ASYNC = "klv_async"
    UNKNOWN = "unknown"


class DiscontinuityKindTag(enum.Enum):
    """Discriminator for `DemuxEvent.Discontinuity.kind`. Mirrors Rust
    `DiscontinuityKind` variants."""
    CONTINUITY_JUMP = "continuity_jump"
    PES_OVERSIZE = "pes_oversize"
    PES_TOTAL_OVERSIZE = "pes_total_oversize"
    ADAPTATION_FIELD_FLAG = "adaptation_field_flag"


class NonConformantKind(enum.Enum):
    """Discriminator for `DemuxEvent.NonConformant.kind`. Collapses
    Rust `NonConformantIssue`'s 30+ variants. The `issue` field on the
    event carries the human-readable detail string."""
    PCR_ANOMALY = "pcr_anomaly"
    PSI_CHECKSUM_MISMATCH = "psi_checksum_mismatch"
    MALFORMED_PES = "malformed_pes"
    PUSI_MID_PES = "pusi_mid_pes"
    TRANSPORT_ERROR_PACKET = "transport_error_packet"
    STREAM_TYPE_MISMATCH = "stream_type_mismatch"
    PID_REUSED_ACROSS_PROGRAMS = "pid_reused_across_programs"
    NAL_HEADER = "nal_header"
    AV1_OBU_HEADER = "av1_obu_header"
    AV1_REGISTRATION_MALFORMED = "av1_registration_malformed"
    PES_HEADER_MALFORMED = "pes_header_malformed"
    PTS_ANOMALY = "pts_anomaly"
    MISSING_REQUIRED_PTS = "missing_required_pts"
    PCR_MALFORMED = "pcr_malformed"
    SUBTITLE_MISSING_DESCRIPTOR = "subtitle_missing_descriptor"
    SUBTITLE_ALIGNMENT_MISSING = "subtitle_alignment_missing"
    MULTI_CELL_AU = "multi_cell_au"
    PSI_MULTI_SECTION_UNSUPPORTED = "psi_multi_section_unsupported"
    PSI_CC_DISCONTINUITY = "psi_cc_discontinuity"
    PSI_OVERLONG_SECTION = "psi_overlong_section"
    DVB_SUB_DATA_IDENTIFIER = "dvb_sub_data_identifier"
    AC3_SYNC_MISSING = "ac3_sync_missing"
    LATM_FRAMING = "latm_framing"
    AV1_WRONG_STREAM_ID = "av1_wrong_stream_id"
    AV1_MISSING_TS_OBU_FRAMING = "av1_missing_ts_obu_framing"
    AV1_OBU_MISSING_SIZE_FIELD = "av1_obu_missing_size_field"
    AV1_TILE_LIST_NOT_ALLOWED = "av1_tile_list_not_allowed"
    MISSING_METADATA_DESCRIPTOR = "missing_metadata_descriptor"
    SUBTITLE_DESCRIPTOR_AMBIGUOUS = "subtitle_descriptor_ambiguous"
    SUBTITLE_DESCRIPTOR_MALFORMED = "subtitle_descriptor_malformed"
    OTHER = "other"


class StrictMode(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::demux::StrictMode`. The strictness
    ladder the demuxer applies when it sees non-conformant streams:
    Off lets everything through as NonConformant events; Full converts
    them to DemuxError."""
    OFF = "off"
    TIMING_ONLY = "timing_only"
    PSI_ONLY = "psi_only"
    FULL = "full"


class LinkSource(enum.Enum):
    """Mirrors Rust `tst_core::mpegts::demux::event::LinkSource`. Tells
    where a KLV-to-video link came from."""
    DECLARED = "declared"
    INFERRED = "inferred"
    OVERRIDE = "override"


# Type alias for "any codec enum or None". Used on StreamId / StreamInfo
# where the kind determines which codec enum (if any) is meaningful.
# PEP 604 union syntax — requires Python 3.10+ (the project floor).
Codec = VideoCodec | AudioCodec | SubtitleCodec | None


@dataclass(frozen=True, slots=True)
class StreamId:
    """Identity of a single elementary stream in a TS. `kind` is the
    discriminator; `codec` carries the specific codec when applicable
    (None for KLV / Unknown). `program_number` resolves cross-program
    PID reuse (per the project's first-program-wins policy)."""

    pid: int
    kind: StreamKindTag
    codec: Codec
    program_number: int


@dataclass(frozen=True, slots=True)
class StreamInfo:
    """PMT-derived per-stream metadata. `stream_type` is the raw PMT
    byte (kept for forward-compat with stream types not yet recognized
    by the demuxer)."""

    pid: int
    stream_type: int  # u8
    kind: StreamKindTag
    codec: Codec
    program_number: int


@dataclass(frozen=True, slots=True)
class KlvLink:
    """A declared, inferred, or overridden link between a KLV PID and
    the video PID it timestamps against. Lives on `ProgramMap`."""

    klv_pid: int
    video_pid: int
    source: LinkSource


@dataclass(frozen=True, slots=True)
class ProgramMap:
    """One program's PSI summary, emitted on PAT/PMT discovery and on
    each PSI version-bump. `streams` is the elementary-stream list;
    `klv_links` is the demuxer's view of which KLV PIDs belong to
    which video PIDs.

    `streams` and `klv_links` are tuples (not lists) so the dataclass
    remains hashable + value-equal."""

    program_number: int
    pcr_pid: int
    streams: tuple
    klv_links: tuple


class DemuxEvent:
    """Top-level event emitted by `Demuxer.next_event()`.

    `DemuxEvent` is a namespace base class — instantiate one of its
    subclasses (`DemuxEvent.Video(...)`, `DemuxEvent.Klv(...)`, etc.)
    or let `Demuxer` construct them for you. Pattern-match with
    Python 3.10+:

    ```
    for ev in parse_file(path):
        match ev:
            case DemuxEvent.Video(stream=s, pts=p, payload=b):
                ...
            case DemuxEvent.Klv(payload=b):
                ...
    ```

    Subclasses are defined immediately below as class-attribute
    dataclasses for clean `DemuxEvent.Video(...)` access syntax.
    """

    # ClassVar annotations so type-checkers (Pylance, mypy) see the
    # subclasses as proper types when users do
    # `DemuxEvent.Video(...)`. The actual assignments happen after
    # the subclass dataclass definitions below.
    ProgramMap: ClassVar[type["_ProgramMapEvent"]]
    Video: ClassVar[type["_VideoEvent"]]
    Audio: ClassVar[type["_AudioEvent"]]
    Subtitle: ClassVar[type["_SubtitleEvent"]]
    Klv: ClassVar[type["_KlvEvent"]]
    Discontinuity: ClassVar[type["_DiscontinuityEvent"]]
    NonConformant: ClassVar[type["_NonConformantEvent"]]
    ReconnectDiscontinuity: ClassVar[type["_ReconnectDiscontinuityEvent"]]


# Subclasses use the `DemuxEvent.X` attribute pattern. Each is a
# frozen dataclass so it's hashable, equality-by-value, and reads
# nicely with pattern matching.

@dataclass(frozen=True, slots=True)
class _ProgramMapEvent(DemuxEvent):
    programs: tuple[ProgramMap, ...]


@dataclass(frozen=True, slots=True)
class _VideoEvent(DemuxEvent):
    stream: StreamId
    pts: Pts90khz
    dts: Optional[Pts90khz]
    codec: VideoCodec
    payload: bytes
    random_access_indicator: bool


@dataclass(frozen=True, slots=True)
class _AudioEvent(DemuxEvent):
    stream: StreamId
    pts: Pts90khz
    dts: Optional[Pts90khz]
    codec: AudioCodec
    frames: bytes


@dataclass(frozen=True, slots=True)
class _SubtitleEvent(DemuxEvent):
    stream: StreamId
    pts: Pts90khz
    dts: Optional[Pts90khz]
    codec: SubtitleCodec
    payload: bytes


@dataclass(frozen=True, slots=True)
class _KlvEvent(DemuxEvent):
    stream: StreamId
    pts: Pts90khz
    kind: MetadataKindTag
    payload: bytes


@dataclass(frozen=True, slots=True)
class _DiscontinuityEvent(DemuxEvent):
    stream: StreamId
    kind: DiscontinuityKindTag


@dataclass(frozen=True, slots=True)
class _NonConformantEvent(DemuxEvent):
    stream: StreamId
    issue: str
    kind: NonConformantKind


@dataclass(frozen=True, slots=True)
class _ReconnectDiscontinuityEvent(DemuxEvent):
    pass


# Expose subclasses as attributes of the base. This gives the
# `DemuxEvent.Video(...)` access syntax in the spec while keeping
# the subclasses defined as top-level dataclasses (so dataclass
# decorators work properly).
DemuxEvent.ProgramMap = _ProgramMapEvent
DemuxEvent.Video = _VideoEvent
DemuxEvent.Audio = _AudioEvent
DemuxEvent.Subtitle = _SubtitleEvent
DemuxEvent.Klv = _KlvEvent
DemuxEvent.Discontinuity = _DiscontinuityEvent
DemuxEvent.NonConformant = _NonConformantEvent
DemuxEvent.ReconnectDiscontinuity = _ReconnectDiscontinuityEvent


# Default PES caps mirror `tst_core::mpegts::demux::DemuxerConfig`
# (4 MB per-PID, 64 MB total). Update both sides together if Rust
# changes its defaults.
_DEFAULT_PES_CAP_PER_PID: int = 4 * 1024 * 1024
_DEFAULT_PES_CAP_TOTAL: int = 64 * 1024 * 1024


@dataclass(frozen=True, slots=True)
class DemuxerConfig:
    """Phase 2 minimal Demuxer configuration. Advanced knobs
    (link_klv, treat_as, av1_carriage) are deferred — a future plan
    will add them once a Python consumer needs them.

    Defaults mirror Rust's `tst_core::mpegts::demux::DemuxerConfig`.
    """

    strict_mode: StrictMode = StrictMode.OFF
    pes_cap_per_pid: int = _DEFAULT_PES_CAP_PER_PID
    pes_cap_total: int = _DEFAULT_PES_CAP_TOTAL


# Re-export the Rust-side PyDemuxer class. The Rust impl lives in
# crates/tst-py/src/mpegts.rs and is exposed via `_native.Demuxer`.
from tstrans import _native as _native_mod

Demuxer = _native_mod.Demuxer

# Population happens task-by-task. __all__ accumulates as types land.
__all__: list[str] = [
    "Pts90khz",
    "VideoCodec",
    "AudioCodec",
    "SubtitleCodec",
    "KlvStreamType",
    "Av1CarriageMode",
    "StreamSpec",
    "VideoStreamSpec",
    "KlvStreamSpec",
    "AudioStreamSpec",
    "SubtitleStreamSpec",
    "StreamKindTag",
    "MetadataKindTag",
    "DiscontinuityKindTag",
    "NonConformantKind",
    "StrictMode",
    "LinkSource",
    "Codec",
    "StreamId",
    "StreamInfo",
    "KlvLink",
    "ProgramMap",
    "DemuxEvent",
    "DemuxerConfig",
    "Demuxer",
]
