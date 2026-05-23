"""Exception hierarchy raised by tstrans.

Every error raised by any tstrans surface is a subclass of `TstError`.
Domain errors (`MuxError`, `DemuxError`, `KlvError`, `KlvEncodeError`,
`CodecError`) carry a typed `.kind` attribute matching the corresponding
Rust *ErrorKind enum. These enums mirror Rust enums marked
`#[non_exhaustive]`, so new variants may appear in minor releases;
matchers should include a default arm.

Field-level KLV parse warnings are NOT raised — they live on the
parsed object as `field_errors`, matching Rust's "best-effort parse"
semantics for ST 0601 in the field.

`MuxErrorKind` was refined in Phase 4 (Muxer wrap) to mirror Rust's
5-variant `tst_core::error::MuxSenderErrorKind` coarse-tier categorical
classification. `KlvEncodeErrorKind` ships in Phase 4 alongside the
`klv.encode_*` Python wrappers. `DemuxErrorKind` / `KlvErrorKind` /
`CodecErrorKind` are placeholders refined in their owning phases.
"""

import enum
from typing import Optional


class TstError(Exception):
    """Base class for every error raised by tstrans. Catch this to
    handle anything from this package."""


class MuxErrorKind(enum.IntEnum):
    """Mirrors Rust `tst_core::error::MuxSenderErrorKind` — the
    coarse-tier 5-variant classification of muxer-side failures
    introduced in plan #91. Every `MuxError` carries one of these on
    `.kind` for programmatic matching; the underlying free-text message
    captures the specific Rust `MuxError` variant.

    The Rust enum is `#[non_exhaustive]`; Python matchers should
    include a default arm. Mapping is performed by
    `tst_py::errors::make_mux_error` in Rust, which translates from
    `MuxSenderErrorKind` to this enum's variant name via SHOUTY_SNAKE.
    """

    # Caller pushed input bytes that don't conform — non-Annex-B NAL,
    # KLV / audio / subtitle PES payloads over the 16-bit PES length cap.
    INPUT_MALFORMED = 0

    # `MuxerConfig::validate()` rejected the construction-time config —
    # duplicate PIDs, too many streams, malformed descriptor TLV,
    # PCR-PID conflicts, ISO 639 / DVB teletext field violations, PMT
    # over-budget, etc. The muxer was not constructed.
    CONFIG_INVALID = 1

    # API misuse on a successfully-built muxer — wrong-muxer stream
    # handle, ambiguous-target shorthand on a multi-stream muxer,
    # unknown program reference, out-of-range descriptor / abs index.
    INVALID_USAGE = 2

    # Muxer output buffer is full; drain via pull and retry.
    BACKPRESSURE = 3

    # Bug-path invariant tripped inside the muxer — should not occur in
    # well-formed use; file an issue with reproduction bytes.
    INTERNAL = 4


class DemuxErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::mpegts::DemuxError` variants."""

    SYNC_LOSS = "sync_loss"
    BAD_PMT = "bad_pmt"
    BAD_PES = "bad_pes"
    UNEXPECTED_EOF = "unexpected_eof"
    INTERNAL = "internal"


class KlvErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::error::KlvDecodeError` variants
    collapsed to user-facing buckets. Only set-level (structural)
    errors raise as `KlvError` — per-field validation failures land
    on the decoded typed-set object as `.field_errors: list[KlvFieldError]`
    instead. See `docs/specs/2026-05-22-tst-py-design.md` "Error
    mapping" for the full mapping table.

    The Rust `KlvDecodeError` enum is `#[non_exhaustive]` — Python
    matchers should include a default arm. Phase 3 establishes the
    initial 8-variant lineup; future Rust variants get added here
    when surfaced."""

    BAD_UNIVERSAL_LABEL = "bad_universal_label"
    TRUNCATED_SET = "truncated_set"
    UNKNOWN_SET = "unknown_set"
    CHECKSUM_MISMATCH = "checksum_mismatch"
    DUPLICATE_TAG = "duplicate_tag"
    MISSING_REQUIRED_TAG = "missing_required_tag"
    MALFORMED_BYTES = "malformed_bytes"
    INTERNAL = "internal"


class KlvEncodeErrorKind(enum.IntEnum):
    """Mirrors Rust `tst_core::error::KlvEncodeError` variant tags.
    Raised by KLV `encode_*` functions when a typed record cannot be
    serialized to wire bytes — output buffer too small, value outside
    the spec-declared range, IMAPB params violating ST 1201.5 §6
    preconditions, mandatory ST 0601 items missing under
    `encode_strict_compliance`, or a reserved tag placed in `unknown`.

    The Rust enum is `#[non_exhaustive]`; new variants land as the
    encoder catches more failure modes. Python matchers should include
    a default arm.
    """

    BUFFER_TOO_SMALL = 0
    RECORD_TOO_LARGE = 1
    OUT_OF_RANGE = 2
    STRING_TOO_LONG = 3
    UNSUPPORTED_IMAPB_LENGTH = 4
    INVALID_IMAPB_PARAMS = 5
    MISSING_MANDATORY_ITEM = 6
    RESERVED_TAG_IN_UNKNOWN = 7


class CodecErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::codec` error variants."""

    UNSUPPORTED_PROFILE = "unsupported_profile"
    BAD_SLICE_HEADER = "bad_slice_header"
    BAD_PARAMETER_SET = "bad_parameter_set"
    TRUNCATED_NAL = "truncated_nal"
    INTERNAL = "internal"


class MuxError(TstError):
    """Raised by `tstrans.mpegts.Muxer` construction, `push_*`, and
    builder `.build()` calls. Carries `MuxErrorKind` on `.kind` and a
    free-text message on `.message` / `.args[0]`. Optional `.pid` is
    populated for stream-not-found and PID-conflict diagnostics.

    Both signatures supported:
      `MuxError("bad config", kind=MuxErrorKind.CONFIG_INVALID)`
      `MuxError(kind=MuxErrorKind.CONFIG_INVALID, message="bad config")`
    """

    kind: MuxErrorKind
    message: str
    pid: Optional[int]

    def __init__(
        self,
        message: Optional[str] = None,
        *,
        kind: MuxErrorKind,
        pid: Optional[int] = None,
    ) -> None:
        if message is None:
            raise TypeError("MuxError requires a message (positional or via message=)")
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.pid = pid


class DemuxError(TstError):
    """Raised by `tstrans.mpegts.Demuxer` operations."""

    kind: DemuxErrorKind
    message: str

    def __init__(self, *, kind: DemuxErrorKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


class KlvError(TstError):
    """Raised by `tstrans.klv` set-level decoders. Field-level warnings
    appear as `field_errors` on the returned typed-set object, not
    raised."""

    kind: KlvErrorKind
    message: str

    def __init__(self, *, kind: KlvErrorKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


class KlvEncodeError(TstError):
    """Raised by `tstrans.klv.encode_*` functions when the typed record
    cannot be serialized. Carries `.kind` (`KlvEncodeErrorKind`) and
    optional `.tag` (the ST item code that triggered the rejection,
    where applicable — e.g. `OUT_OF_RANGE`, `STRING_TOO_LONG`,
    `MISSING_MANDATORY_ITEM`, `RESERVED_TAG_IN_UNKNOWN`). `BUFFER_TOO_SMALL`,
    `RECORD_TOO_LARGE`, `UNSUPPORTED_IMAPB_LENGTH`, and
    `INVALID_IMAPB_PARAMS` have no associated tag — `.tag` is `None`.
    """

    kind: KlvEncodeErrorKind
    message: str
    tag: Optional[int]

    def __init__(
        self,
        message: Optional[str] = None,
        *,
        kind: KlvEncodeErrorKind,
        tag: Optional[int] = None,
    ) -> None:
        if message is None:
            raise TypeError("KlvEncodeError requires a message (positional or via message=)")
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.tag = tag


class CodecError(TstError):
    """Raised by `tstrans.codec` frame parsers. `codec` is one of
    `"h264"`, `"h265"`, `"h266"`, `"av1"`, `"aac"`, `"mpeg2audio"`."""

    kind: CodecErrorKind
    message: str
    codec: str

    def __init__(self, *, kind: CodecErrorKind, message: str, codec: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message
        self.codec = codec


__all__ = [
    "TstError",
    "MuxError",
    "MuxErrorKind",
    "DemuxError",
    "DemuxErrorKind",
    "KlvError",
    "KlvErrorKind",
    "KlvEncodeError",
    "KlvEncodeErrorKind",
    "CodecError",
    "CodecErrorKind",
]
