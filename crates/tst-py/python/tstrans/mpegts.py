"""tstrans.mpegts — MPEG-TS packet, PES, PSI, muxer, demuxer surface.

Public types:

- `Pts90khz` — 90 kHz timestamp wrapper.
- `VideoCodec`, `AudioCodec`, `SubtitleCodec`, `StrictMode` enums.
- `StreamId`, `StreamInfo`, `KlvLink`, `ProgramMap` dataclasses.
- `DemuxEvent` base + subclasses: `ProgramMap`, `Video`, `Audio`,
  `Subtitle`, `Klv`, `UnknownSample`, `Discontinuity`, `NonConformant`,
  `ReconnectDiscontinuity`.
- `DemuxerConfig`, `Demuxer` — feed bytes, get events; supports
  `strict_mode`, `pes_cap_*`, `cfi_tolerance`.
- `MuxerConfig`, `MuxerProgramConfig`, `Muxer`, `MuxerFileSink` —
  build TS, drain to file (with optional atomic-rename mode).
- `MultiCellAuReason`, `CellFragmentIndication` — typed diagnostics
  on `NonConformant` events.
"""

import enum
import os
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, ClassVar, Optional


@dataclass(frozen=True, slots=True)
class Pts90khz:
    """A 90 kHz timestamp tick count, the MPEG-TS PTS unit.

    Wraps an `i64` to allow signed-diff arithmetic (per the Rust
    `tst_core::mpegts::common::Pts90khz`). Construct via
    `Pts90khz.from_raw(int)`, `Pts90khz.from_ms(int)`, or
    `Pts90khz.from_seconds(float)`.
    """

    raw: int

    def __post_init__(self) -> None:
        # Audit-2 #4 — fail-fast on out-of-i64 values; Rust extracts as
        # i64 and would raise OverflowError later anyway, but the early
        # ValueError points at the user's construction site.
        if not -(1 << 63) <= self.raw <= (1 << 63) - 1:
            raise ValueError(
                f"Pts90khz.raw must fit signed i64; got {self.raw}"
            )

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
        # Sign-aware integer truncate toward zero — matches Rust's `i64 / 90`
        # semantics. Python's `//` floors toward -inf, which diverges on
        # negatives (e.g. `-1 // 90 == -1` vs Rust's `0`); a float-based
        # `int(raw / 90)` would lose precision past float64's 53-bit
        # mantissa, so do it in integers only.
        if self.raw < 0:
            return -((-self.raw) // 90)
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

    Set the same value on both sides for a successful round-trip:

    - Sender side: pass to `MuxerConfigBuilder.av1_carriage(...)`.
    - Receiver side: pass to `DemuxerConfig(av1_carriage=...)`.

    A mismatch surfaces on the receiver as
    `NonConformantKind.AV1_WRONG_STREAM_ID` plus
    `NonConformantKind.AV1_MISSING_TS_OBU_FRAMING`; the Sample still
    arrives via the lenient raw-OBU fallback path.
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
    CFI_TOLERATED = "malformed_au_cell_cfi_tolerated"
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
    UnknownSample: ClassVar[type["_UnknownSampleEvent"]]
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
    # Phase 5: list[NalUnit] for H.264/H.265/H.266; list[Obu] for AV1.
    payload: Any
    random_access_indicator: bool
    # None — video typed-parse cannot fail at this layer.
    codec_parse_error: Optional[Any] = None


@dataclass(frozen=True, slots=True)
class _AudioEvent(DemuxEvent):
    stream: StreamId
    pts: Pts90khz
    dts: Optional[Pts90khz]
    codec: AudioCodec
    # Phase 5: list[AdtsFrame] (AAC) | list[Mpeg2AudioFrame] (MP2) | bytes
    # (LATM/AC-3/fallback-on-error). Use `codec_parse_error` to distinguish
    # bytes-due-to-error from bytes-by-design.
    payload: Any
    # Populated when mid-stream parse failure triggers bytes fallback (option c).
    # None on clean parse or codec types whose typed parser is deferred.
    codec_parse_error: Optional[Any] = None


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
    # Multi-cell AU reassembly surface (H.222.0 §2.12.4.2). `False` + `1`
    # on single-cell (Complete) AUs and on non-KlvSyncAuCell metadata
    # events. Defaults preserve backward compatibility for any callers
    # that construct `_KlvEvent` directly (the Rust converter always
    # populates these explicitly).
    was_reassembled: bool = False
    cell_count: int = 1


@dataclass(frozen=True, slots=True)
class _UnknownSampleEvent(DemuxEvent):
    """Sample on a PID whose stream_type the demuxer does not classify
    as Video / Audio / Subtitle / KLV. The raw stream_type byte (per
    PMT) and the unparsed PES payload are preserved verbatim so callers
    can archive, forward, or post-process.

    Audit-2 finding #1 — prior versions collapsed unknown samples into
    a NonConformant diagnostic and discarded the payload bytes.
    """

    stream: StreamId
    pts: Pts90khz
    dts: Optional[Pts90khz]
    stream_type: int  # raw PMT byte, 0..=255
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
    # Typed reason set only when `kind == NonConformantKind.MULTI_CELL_AU`;
    # `None` for all other issues. Default keeps existing constructors
    # working without changes.
    multi_cell_au_reason: Optional["MultiCellAuReason"] = None
    # Typed CFI bits set only when
    # `kind == NonConformantKind.CFI_TOLERATED`; `None`
    # for all other issues. `observed_cfi` is the wire value the demuxer
    # read; `treated_as` is the value substituted (always
    # `CellFragmentIndication.COMPLETE` today). Both default to None so
    # existing constructors keep working.
    observed_cfi: Optional["CellFragmentIndication"] = None
    treated_as: Optional["CellFragmentIndication"] = None


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
DemuxEvent.UnknownSample = _UnknownSampleEvent
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
    """Demuxer configuration. Mirrors Rust's
    `tst_core::mpegts::demux::DemuxerConfig` for the knobs currently
    exposed to Python:

    - `strict_mode` — Off / TimingOnly / PsiOnly / Full ladder.
    - `pes_cap_per_pid`, `pes_cap_total` — reassembly memory caps.
    - `cfi_tolerance` — opt-in lenient AU-cell CFI substitution
      (see MultiCellAuReason / CellFragmentIndication).
    - `av1_carriage` — AV1 PES carriage mode the demuxer expects;
      `None` (the default) defers to the Rust default
      (`Av1CarriageMode.MPEG2_TS_BINDING`). Set to
      `Av1CarriageMode.INTEROP_RAW_OBU` when receiving from
      ffmpeg / libaom / hls.js / mediamtx senders. A mismatched value
      against the wire carriage surfaces as
      `NonConformantKind.AV1_WRONG_STREAM_ID` plus
      `NonConformantKind.AV1_MISSING_TS_OBU_FRAMING`; the Sample
      still arrives via the lenient raw-OBU fallback.
    - `au_cell_cap_per_pid` — per-PID cap on the in-flight
      sync-metadata AU cell reassembly buffer in bytes; `None` defers
      to the Rust default of 1 MiB. Breach surfaces as
      `NonConformantKind.MULTI_CELL_AU` with
      `multi_cell_au_reason = MultiCellAuReason.OVERFLOW`.
    - `lenient_psi_reassembly` — when False (default), PSI section
      reassembly drops the partial section on continuity-counter
      jumps and emits `NonConformantKind.PSI_CC_DISCONTINUITY`
      (matches ffmpeg `mpegts.c:3118-3142`). When True, continuation
      packets are accepted across jumps (today's permissive
      behavior — the section either passes by luck or fails its CRC).

    `link_klv` and `treat_as` overrides (per-PID PMT-bypass knobs)
    remain Rust-only today; open an issue if your use case needs them.
    """

    strict_mode: StrictMode = StrictMode.OFF
    pes_cap_per_pid: int = _DEFAULT_PES_CAP_PER_PID
    pes_cap_total: int = _DEFAULT_PES_CAP_TOTAL
    # When True, the demuxer treats orphan Middle/Last AU cells as
    # Complete if the inner payload independently validates as one
    # complete KLV record (SMPTE 336M UL + BER length match). Always
    # emits a `_NonConformantEvent` with kind
    # `CFI_TOLERATED` alongside the rescued metadata
    # event. Default False keeps the spec-strict behavior: orphan cells
    # surface only as `MULTI_CELL_AU{ORPHAN}`.
    cfi_tolerance: bool = False
    # `None` defers to the Rust default
    # (`Av1CarriageMode::Mpeg2TsBinding`). Storing the Python
    # `Av1CarriageMode` value (not its `.value` string) lets the
    # `build_demuxer()` plumbing call `value` for the
    # already-existing Rust mapping.
    av1_carriage: Optional[Av1CarriageMode] = None
    # `None` defers to the Rust default of 1 MiB. Any positive int
    # caps the in-flight AU cell reassembly buffer at that many bytes.
    au_cell_cap_per_pid: Optional[int] = None
    # See class docstring above for the spec-strict vs lenient PSI
    # reassembly trade-off.
    lenient_psi_reassembly: bool = False


# Phase 5: re-export NalUnit / Obu / ObuExtension so callers can import
# them from `tstrans.mpegts` without also importing from `tstrans.codec`.
# These are the types that appear in `DemuxEvent.Video.payload` lists.
from tstrans.codec import NalUnit, Obu, ObuExtension

# Re-export the Rust-side PyDemuxer class. The Rust impl lives in
# crates/tst-py/src/mpegts.rs and is exposed via `_native.Demuxer`.
from tstrans import _native as _native_mod

Demuxer = _native_mod.Demuxer

# `MultiCellAuReason` — PyO3 `eq_int` enum mirroring
# `tst_core::mpegts::demux::event::MultiCellAuReason`. Re-exported here so
# Python users can `from tstrans.mpegts import MultiCellAuReason`. Set on
# `_NonConformantEvent.multi_cell_au_reason` when the issue is
# `MULTI_CELL_AU`; `None` otherwise.
MultiCellAuReason = _native_mod.MultiCellAuReason

# `CellFragmentIndication` — PyO3 `eq_int` enum mirroring
# `tst_core::mpegts::au_cell::CellFragmentIndication`. Re-exported here so
# Python users can `from tstrans.mpegts import CellFragmentIndication`.
# Set on `_NonConformantEvent.observed_cfi` and `_NonConformantEvent.treated_as`
# when the issue is `CFI_TOLERATED`; `None` otherwise.
# Discriminant values match the wire bits exactly: MIDDLE=0, LAST=1,
# FIRST=2, COMPLETE=3 (per H.222.0 V9 Table 2-157).
CellFragmentIndication = _native_mod.CellFragmentIndication

# Phase 4 Task 3 — stream handle newtypes. Rust impls live in
# crates/tst-py/src/mux.rs as `Py{Video,Audio,Klv,Subtitle}StreamHandle`,
# exposed on `_native` under the names below via `#[pyclass(name=...)]`.
VideoStreamHandle = _native_mod.VideoStreamHandle
AudioStreamHandle = _native_mod.AudioStreamHandle
KlvStreamHandle = _native_mod.KlvStreamHandle
SubtitleStreamHandle = _native_mod.SubtitleStreamHandle

# Phase 4 Task 4 — program-level config + builder. Rust impls in
# crates/tst-py/src/mux.rs as `PyMuxerProgramConfig` /
# `PyMuxerProgramConfigBuilder`, exposed on `_native` under the
# names below via `#[pyclass(name=...)]`.
MuxerProgramConfig = _native_mod.MuxerProgramConfig
MuxerProgramConfigBuilder = _native_mod.MuxerProgramConfigBuilder

# Phase 4 Task 5 — top-level muxer config + builder. Rust impls in
# crates/tst-py/src/mux.rs as `PyMuxerConfig` / `PyMuxerConfigBuilder`,
# exposed on `_native` under the names below via `#[pyclass(name=...)]`.
# `MuxerConfigBuilder.build()` runs Rust-side validation and raises
# `tstrans.exceptions.MuxError` on failure.
MuxerConfig = _native_mod.MuxerConfig
MuxerConfigBuilder = _native_mod.MuxerConfigBuilder

# Phase 4 Task 6 — Muxer base (init + pull + pending + capacity). Rust
# impl in crates/tst-py/src/mux.rs as `PyMuxer`, exposed on `_native`
# under the name below via `#[pyclass(name=...)]`. Constructor takes a
# `MuxerConfig` and re-runs Rust-side validation, surfacing failures
# as `tstrans.exceptions.MuxError`. The `push_*` and handle-getter
# surface lands in Tasks 7-9.
Muxer = _native_mod.Muxer

# Phase 4 Task 10 — MuxerStats snapshot type. Rust impl in
# crates/tst-py/src/mux.rs as `PyMuxerStats`, exposed on `_native` under
# the name below via `#[pyclass(name=...)]`. Frozen — returned by
# `Muxer.stats()`. The `per_stream` BTreeMap on the Rust side is not
# surfaced in v1 (the per-PID `StreamStats` shape exists but isn't yet
# wrapped); the scalar counters cover the common dashboard case.
MuxerStats = _native_mod.MuxerStats


# Phase 4 Task 10 — StreamCodecStats tagged union. Pure-Python
# dataclasses (no PyO3 wrap) because the Rust enum has 3+ struct
# variants and PyO3 lacks ergonomic enum support — the
# `Muxer.stream_codec_stats(pid)` accessor constructs the right
# subclass on each call. `Some(Unknown)` from Rust (configured-but-
# no-data PID) is rendered as `None` from Python in v1; callers
# distinguish "configured and pushed" (typed subclass) from "either
# unconfigured or never pushed" (`None`).

@dataclass(frozen=True, slots=True)
class StreamCodecStats:
    """Abstract base for per-stream codec counter snapshots.

    Returned by `Muxer.stream_codec_stats(pid)`. Concrete subclasses
    are `VideoStreamCodecStats`, `KlvStreamCodecStats`, and
    `AudioStreamCodecStats` — match on the subclass to read the
    typed counters. Mirrors Rust
    `tst_core::mpegts::stats::StreamCodecStats`.
    """


@dataclass(frozen=True, slots=True)
class VideoStreamCodecStats(StreamCodecStats):
    """H.264 / H.265 / H.266 (NALs) or AV1 (OBUs) counters."""

    nals_or_obus: int
    random_access_aus: int


@dataclass(frozen=True, slots=True)
class KlvStreamCodecStats(StreamCodecStats):
    """KLV metadata counters — one bump per `push_klv` call."""

    records: int


@dataclass(frozen=True, slots=True)
class AudioStreamCodecStats(StreamCodecStats):
    """Audio frame counters — populated for MP2 + AAC-ADTS only; LATM
    and AC-3 PIDs return `None` (no frame iterator in v1)."""

    frames: int

# Phase 4 Task 11 — MuxerFileSink + MuxerDrainProxy + `Muxer.write_file`.
# Pure-Python sink: opens a file in `wb` mode and drains pending TS
# packets after every `push_*` call inside the `with` block. The proxy
# uses `__getattr__` to forward every other attribute to the wrapped
# Muxer untouched, so callers see a near-transparent Muxer surface plus
# the implicit drain-to-disk behavior.

# Drain chunk size: 4 packets × 188 bytes. Small enough to keep memory
# footprint low; large enough to amortize the pull() call cost.
_DRAIN_CHUNK_PACKETS = 4
_DRAIN_CHUNK_BYTES = _DRAIN_CHUNK_PACKETS * 188


def _drain_muxer_to_file(muxer: "Muxer", fh) -> None:
    """Pull pending packets in 4-packet chunks and write to `fh`.

    Stops when pull() returns 0 or pending drops to 0. Used by both
    `MuxerDrainProxy` (after each push) and `MuxerFileSink.__exit__`
    (final drain on close).
    """

    buf = bytearray(_DRAIN_CHUNK_BYTES)
    while muxer.pending_packets() > 0:
        n = muxer.pull(buf)
        if n == 0:
            break
        fh.write(bytes(buf[:n]))


class MuxerDrainProxy:
    """Returned by `MuxerFileSink.__enter__`. Delegates every attribute
    access to the wrapped `Muxer`; intercepts the `push_*` methods to
    drain pending packets to the sink's file after each push call.

    Non-push methods (e.g. `pending_packets()`, `video_handles()`,
    `stats()`) pass through unchanged via `__getattr__`. This keeps the
    proxy near-transparent — code inside the `with` block can treat
    `proxy` as a Muxer for read-only inspection while the implicit
    drain happens behind the scenes on every push.
    """

    __slots__ = ("_muxer", "_fh")

    # The set of methods to wrap with a post-push drain. Kept in sync
    # with `PyMuxer`'s push_* surface (Tasks 7 + 8).
    _PUSH_METHODS = frozenset({
        "push_video", "push_video_to", "push_video_to_with_dts",
        "push_audio", "push_audio_to",
        "push_klv", "push_klv_to",
        "push_subtitle", "push_subtitle_to",
    })

    def __init__(self, muxer, fh) -> None:
        # `object.__setattr__` because we use `__slots__` and want to
        # bypass any future `__setattr__` override (defensive).
        object.__setattr__(self, "_muxer", muxer)
        object.__setattr__(self, "_fh", fh)

    def __getattr__(self, name: str) -> Any:
        # `__getattr__` only fires for attrs NOT found via the normal
        # lookup chain — so `_muxer` and `_fh` (slot descriptors) hit
        # the fast path and never recurse here.
        attr = getattr(self._muxer, name)
        if name in MuxerDrainProxy._PUSH_METHODS:
            fh = self._fh
            muxer = self._muxer

            def wrapper(*args, **kwargs):
                result = attr(*args, **kwargs)
                _drain_muxer_to_file(muxer, fh)
                return result

            return wrapper
        return attr


class MuxerFileSink:
    """Context manager owning a file handle + a draining proxy for an
    external `Muxer`. Always flushes + closes on `__exit__`; user
    exceptions inside the `with` body are re-raised unchanged (the
    sink does NOT suppress them, but it DOES still drain whatever's
    pending and close the file so partial output is preserved).

    **Non-atomic exit behavior (default, `atomic=False`):** on
    exception inside the `with` block, the destination file may exist
    as a valid TS prefix (whatever was drained before the exception).
    The caller is responsible for unlinking it if partial output is
    unwanted. For atomic semantics — file appears at destination only
    on successful exit — use `Muxer.write_file(path, atomic=True)`.

    **Atomic mode (`atomic=True`):** on `__enter__`, opens a
    `*.partial` temp file in the same directory as `path` (so the
    final rename stays on the same filesystem). On successful exit,
    drains + closes + `os.replace(tmp, path)` (atomic on both POSIX
    and Windows). On exception, drains + closes + unlinks the temp
    file so nothing appears at the destination. The user's exception
    is re-raised either way.

    Construct via `Muxer.write_file(path)` or
    `Muxer.write_file(path, atomic=True)`, not directly. The Muxer
    itself is borrowed, not owned — it remains usable after the `with`
    block exits, including for further `write_file(...)` calls.
    """

    __slots__ = ("_muxer", "_path", "_fh", "_proxy", "_atomic", "_tmp_path")

    def __init__(self, muxer, path, *, atomic: bool = False) -> None:
        self._muxer = muxer
        # `Path()` accepts str, os.PathLike, and Path itself.
        self._path = Path(path)
        self._fh = None
        self._proxy = None
        self._atomic = atomic
        self._tmp_path = None

    def __enter__(self) -> MuxerDrainProxy:
        if self._atomic:
            # Tempfile in the SAME directory as the destination so the
            # eventual `os.replace` stays on one filesystem (rename
            # across filesystems is not atomic). `delete=False` so the
            # NamedTemporaryFile wrapper doesn't try to unlink on close
            # — we manage the lifetime ourselves in `__exit__`.
            tmp = tempfile.NamedTemporaryFile(
                dir=self._path.parent,
                suffix=".partial",
                delete=False,
            )
            self._tmp_path = Path(tmp.name)
            self._fh = tmp
        else:
            self._fh = self._path.open("wb")
        self._proxy = MuxerDrainProxy(self._muxer, self._fh)
        return self._proxy

    def __exit__(self, exc_type, exc, tb) -> None:
        # Audit-2 #2: drive cleanup with one outer try/finally so that
        # even if drain/close raises, the atomic-mode .partial file is
        # always removed (or replaced on success). Do not suppress the
        # caller's exception.
        drain_or_close_failed = False
        try:
            try:
                _drain_muxer_to_file(self._muxer, self._fh)
            finally:
                # Always close, even if drain raised. Both exceptions
                # propagate at the end; close-time exception chains onto
                # the drain-time one via Python's implicit __context__.
                self._fh.close()
        except BaseException:
            drain_or_close_failed = True
            if self._atomic:
                try:
                    os.unlink(self._tmp_path)
                except FileNotFoundError:
                    pass
            raise
        finally:
            # Only on the success path: promote .partial → final dest.
            if self._atomic and exc_type is None and not drain_or_close_failed:
                os.replace(self._tmp_path, self._path)
            elif self._atomic and exc_type is not None and not drain_or_close_failed:
                # User-body exception (drain succeeded): discard partial.
                try:
                    os.unlink(self._tmp_path)
                except FileNotFoundError:
                    pass
        # Returning None — never suppress.


def _muxer_write_file(self, path, *, atomic: bool = False) -> MuxerFileSink:
    """Open `path` for writing (mode `wb`) and return a context manager
    that drains pending TS packets after each `push_*` call inside the
    `with` block and on exit. The Muxer is borrowed, not owned —
    callers can reuse it for further `write_file(...)` calls after the
    `with` block exits.

    Pass `atomic=True` to write via a `*.partial` tempfile in the same
    directory and `os.replace` to `path` only on successful exit. On
    exception, the tempfile is removed and no file appears at the
    destination. See `MuxerFileSink` for the full atomic-vs-default
    contract.

    Equivalent to constructing `MuxerFileSink(self, path, atomic=...)`
    directly.
    """

    return MuxerFileSink(self, path, atomic=atomic)


# Bind `write_file` onto the PyO3 Muxer class. PyO3 #[pyclass] types
# in abi3 mode without `subclass` may reject attribute assignment; if
# that happens, switch to the `#[pyclass(..., subclass)]` + subclass
# pattern (see task notes).
Muxer.write_file = _muxer_write_file  # type: ignore[attr-defined]


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
    "MultiCellAuReason",
    "CellFragmentIndication",
    "VideoStreamHandle",
    "AudioStreamHandle",
    "KlvStreamHandle",
    "SubtitleStreamHandle",
    "MuxerProgramConfig",
    "MuxerProgramConfigBuilder",
    "MuxerConfig",
    "MuxerConfigBuilder",
    "Muxer",
    "MuxerStats",
    "StreamCodecStats",
    "VideoStreamCodecStats",
    "KlvStreamCodecStats",
    "AudioStreamCodecStats",
    "MuxerFileSink",
    "MuxerDrainProxy",
    # Phase 5: NalUnit / Obu / ObuExtension re-exported from tstrans.codec
    # for convenient import from tstrans.mpegts (they appear in
    # DemuxEvent.Video.payload lists).
    "NalUnit",
    "Obu",
    "ObuExtension",
]
