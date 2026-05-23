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


# Population happens task-by-task. __all__ accumulates as types land.
__all__: list[str] = [
    "Pts90khz",
    "VideoCodec",
    "AudioCodec",
    "SubtitleCodec",
    "StreamKindTag",
    "MetadataKindTag",
    "DiscontinuityKindTag",
    "NonConformantKind",
    "StrictMode",
    "LinkSource",
]
