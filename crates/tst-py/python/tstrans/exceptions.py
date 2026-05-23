"""Exception hierarchy raised by tstrans.

Every error raised by any tstrans surface is a subclass of `TstError`.
Domain errors (`MuxError`, `DemuxError`, `KlvError`, `CodecError`) carry
a typed `.kind` attribute matching the corresponding Rust *ErrorKind
enum. These enums mirror Rust enums marked `#[non_exhaustive]`, so new
variants may appear in minor releases; matchers should include a
default arm.

Field-level KLV parse warnings are NOT raised -- they live on the
parsed object as `field_errors: list[KlvFieldError]`, matching Rust's
"best-effort parse" semantics for ST 0601 in the field.
"""

import enum
from typing import Optional


class TstError(Exception):
    """Base class for every error raised by tstrans. Catch this to
    handle anything from this package."""


class MuxErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::mpegts::MuxError` variants. Marked
    `#[non_exhaustive]` on the Rust side; new variants may appear in
    minor releases."""

    INVALID_CONFIG = "invalid_config"
    MISSING_STREAM = "missing_stream"
    PID_CONFLICT = "pid_conflict"
    INTERNAL = "internal"


class DemuxErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::mpegts::DemuxError` variants."""

    SYNC_LOSS = "sync_loss"
    BAD_PMT = "bad_pmt"
    BAD_PES = "bad_pes"
    UNEXPECTED_EOF = "unexpected_eof"
    INTERNAL = "internal"


class KlvErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::klv::KlvError` variants. Only
    set-level (not field-level) errors raise -- field warnings live on
    `Klv0601.field_errors`."""

    BAD_UNIVERSAL_LABEL = "bad_universal_label"
    TRUNCATED_SET = "truncated_set"
    UNKNOWN_SET = "unknown_set"
    INTERNAL = "internal"


class CodecErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::codec` error variants."""

    UNSUPPORTED_PROFILE = "unsupported_profile"
    BAD_SLICE_HEADER = "bad_slice_header"
    BAD_PARAMETER_SET = "bad_parameter_set"
    TRUNCATED_NAL = "truncated_nal"
    INTERNAL = "internal"


class MuxError(TstError):
    """Raised by `tstrans.mpegts.Muxer` operations."""

    kind: MuxErrorKind
    message: str

    def __init__(self, *, kind: MuxErrorKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


class DemuxError(TstError):
    """Raised by `tstrans.mpegts.Demuxer` operations."""

    kind: DemuxErrorKind
    message: str

    def __init__(self, *, kind: DemuxErrorKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


class KlvError(TstError):
    """Raised by `tstrans.klv` set-level decoders / encoders. Field-level
    warnings appear as `field_errors` on the returned typed-set object,
    not raised."""

    kind: KlvErrorKind
    message: str

    def __init__(self, *, kind: KlvErrorKind, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


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
    "CodecError",
    "CodecErrorKind",
]
