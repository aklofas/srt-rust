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

`MuxErrorKind` mirrors Rust's 5-variant
`tst_core::error::MuxErrorKind` coarse-tier categorical
classification. `KlvEncodeErrorKind` accompanies the `klv.encode_*`
Python wrappers. `DemuxErrorKind` / `KlvErrorKind` / `CodecErrorKind`
mirror their owning Rust enums.
"""

import enum
from typing import Any, Optional


class TstError(Exception):
    """Base class for every error raised by tstrans. Catch this to
    handle anything from this package."""


class _KindMessageError(TstError):
    """Private base for the nine transport/domain error classes that share
    an identical ``(*, kind, message: str)`` constructor.  Rust raises
    these via ``cls.call((), Some(&kwargs))`` with ``{kind=...,
    message=...}``, so the constructor must accept exactly those two
    keyword-only arguments.

    Not exported (absent from ``__all__``); do not instantiate directly.
    Subclasses redeclare ``kind`` with the concrete ``*Kind`` type so
    static type checkers see the narrower annotation.
    """

    # `Any` (not `object`): subclasses redeclare `kind` with their concrete
    # `*Kind` enum, and narrowing a mutable `object` attribute would be an
    # invalid override to type checkers; `Any` permits the narrowing.
    kind: Any
    message: str

    def __init__(self, *, kind: Any, message: str) -> None:
        super().__init__(message)
        self.kind = kind
        self.message = message


class MuxErrorKind(enum.IntEnum):
    """Mirrors Rust `tst_core::error::MuxErrorKind` — the
    coarse-tier 5-variant classification of muxer-side failures
    introduced in plan #91. Every `MuxError` carries one of these on
    `.kind` for programmatic matching; the underlying free-text message
    captures the specific Rust `MuxError` variant.

    The Rust enum is `#[non_exhaustive]`; Python matchers should
    include a default arm. Mapping is performed by
    `tst_py::errors::make_mux_error` in Rust, which translates from
    `MuxErrorKind` to this enum's variant name via SHOUTY_SNAKE.
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
    """Mirrors Rust's `tst_core::mpegts::DemuxError` variants.

    ``STRICT_REJECTION`` is the Python-side representation of
    ``DemuxError::StrictRejection`` — raised when ``StrictMode`` is
    non-Off and the demuxer encounters a non-conformance that the
    configured policy escalates to a fatal error.  The underlying
    ``DemuxError`` message carries the specific ``NonConformantIssue``
    name as a diagnostic string.

    Matchers should include a default arm — the Rust ``DemuxError``
    enum is ``#[non_exhaustive]`` and new variants may appear in minor
    releases.
    """

    SYNC_LOSS = "sync_loss"
    BAD_PMT = "bad_pmt"
    BAD_PES = "bad_pes"
    UNEXPECTED_EOF = "unexpected_eof"
    # Strict-mode policy rejection — StrictMode converted a non-conformance
    # into a fatal error.  Placed before INTERNAL (increasing severity).
    STRICT_REJECTION = "strict_rejection"
    INTERNAL = "internal"


class KlvErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::error::KlvDecodeError` variants
    collapsed to user-facing buckets. Only set-level (structural)
    errors raise as `KlvError` — per-field validation failures land
    on the decoded typed-set object as `.field_errors: list[KlvFieldError]`
    instead. See `docs/specs/2026-05-22-tst-py-design.md` "Error
    mapping" for the full mapping table.

    The Rust `KlvDecodeError` enum is `#[non_exhaustive]` — Python
    matchers should include a default arm. The initial 8-variant
    lineup is below; future Rust variants get added here when
    surfaced."""

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
    VTARGET_PACK_EMPTY = 8
    DUPLICATE_TARGET_ID = 9
    FORBIDDEN_STANDALONE_OFFSET = 10


class CodecErrorKind(enum.IntEnum):
    """Mirrors `tst_core::codec::CodecParseError` variants.

    Unknown Rust variants (e.g. from a newer library version) are mapped
    to ``ENGINE_ERROR`` by the native extension; Python pattern-matchers
    must include a default fallback to remain robust.
    """

    TRUNCATED_RBSP = 1
    INVALID_GOLOMB = 2
    RESERVED_VALUE = 3
    UNSUPPORTED_PROFILE = 4
    DANGLING_SPS_REFERENCE = 5
    DANGLING_VPS_REFERENCE = 6
    ENGINE_ERROR = 7
    INVALID_LEB128 = 8
    BAD_SYNC_WORD = 9
    TRUNCATED = 10
    FORBIDDEN = 11
    UNSUPPORTED_FREE_FORMAT = 12


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


class DemuxError(_KindMessageError):
    """Raised by `tstrans.mpegts.Demuxer` operations."""

    kind: DemuxErrorKind


class KlvError(_KindMessageError):
    """Raised by `tstrans.klv` set-level decoders. Field-level warnings
    appear as `field_errors` on the returned typed-set object, not
    raised."""

    kind: KlvErrorKind


class KlvEncodeError(TstError):
    """Raised by `tstrans.klv.encode_*` functions when the typed record
    cannot be serialized. Carries `.kind` (`KlvEncodeErrorKind`) and an
    optional `.tag`. For most tag-bearing variants `.tag` is the offending
    ST item (KLV tag) code — `OUT_OF_RANGE`, `STRING_TOO_LONG`,
    `MISSING_MANDATORY_ITEM`, `RESERVED_TAG_IN_UNKNOWN`, and
    `FORBIDDEN_STANDALONE_OFFSET`. For `VTARGET_PACK_EMPTY` and
    `DUPLICATE_TARGET_ID`, `.tag` instead carries the VTarget Pack
    `target_id` (a target identifier, not a KLV tag). `BUFFER_TOO_SMALL`,
    `RECORD_TOO_LARGE`, `UNSUPPORTED_IMAPB_LENGTH`, and
    `INVALID_IMAPB_PARAMS` have no associated value — `.tag` is `None`.
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


class RtspErrorKind(enum.IntEnum):
    """Mirrors `tst_rtp::rtsp::RtspError` variants collapsed to
    user-facing buckets. Raised by `tstrans.rtp.RtspClient` /
    `tstrans.rtp.RtspServer` operations (connect, play, pause,
    teardown, start, stop, add_mount).

    Available only when tstrans was built with the `rtp` cargo
    feature (default-on in published wheels).
    """

    PROTOCOL = 1
    AUTH_FAILED = 2
    AUTH_REQUIRED = 3
    NOT_FOUND = 4
    UNSUPPORTED_TRANSPORT = 5
    TLS = 6
    IO = 7
    TIMEOUT = 8
    SERVER = 9
    MOUNT = 10


class RtspError(_KindMessageError):
    """Raised by `tstrans.rtp.RtspClient` / `RtspServer` / `MountHandle`
    operations. Carries a typed `.kind` (`RtspErrorKind`) plus a
    free-text message on `.message` / `.args[0]`."""

    kind: RtspErrorKind


class RtpErrorKind(enum.IntEnum):
    """Mirrors `tst_rtp::transport::RtpError` variants. Raised by
    `tstrans.rtp.Sender` / `Receiver` / `MuxSender` / `DemuxReceiver`
    send/recv/push operations.

    Available only when tstrans was built with the `rtp` cargo
    feature (default-on in published wheels).
    """

    TRANSPORT = 1
    MALFORMED_PACKET = 2
    CANCELLED = 3


class RtpError(_KindMessageError):
    """Raised by `tstrans.rtp` transport operations."""

    kind: RtpErrorKind


class SrtErrorKind(enum.IntEnum):
    """Mirrors `tst_srt::IoError` / `ConnectError` / `AcceptError`
    + `tst_pipeline::TransportError` variants collapsed to user-facing
    buckets. Raised by `tstrans.srt` socket / sender / receiver
    operations.

    Available only when tstrans was built with the `srt` cargo
    feature (default-on in published wheels).
    """

    CONNECT_FAILED = 0
    ACCEPT_FAILED = 1
    WOULD_BLOCK = 2
    TIMEOUT = 3
    CLOSED = 4
    BROKEN = 5
    CONFIG_INVALID = 6
    IO = 7


class SrtError(_KindMessageError):
    """Raised by `tstrans.srt` operations. Discriminate via `.kind`."""

    kind: SrtErrorKind


# ── Plan A5b — udp / tcp / hls / rist transport error classes ────────────
# Kind enums mirror the Rust `*ErrorKind` variant sets (SCREAMING_SNAKE).
# The Rust side raises these via `errors::make_<proto>_error(py, "VARIANT",
# message)` (import-based, mirroring make_rtsp_error — NOT create_exception!).


class UdpErrorKind(enum.IntEnum):
    """Mirrors `tst_udp::UdpErrorKind`. Raised by `tstrans.udp` operations
    (built with the `udp` cargo feature, default-on)."""

    URL = 0
    HOST_NOT_LITERAL = 1
    IO = 2
    IFACE_UNSUPPORTED = 3
    PAYLOAD_TOO_LARGE = 4
    CLOSED = 5
    INVALID_CONFIG = 6


class UdpError(_KindMessageError):
    """Raised by `tstrans.udp` operations. Discriminate via `.kind`."""

    kind: UdpErrorKind


class TcpErrorKind(enum.IntEnum):
    """Mirrors `tst_tcp::TcpErrorKind`. Raised by `tstrans.tcp` operations
    (built with the `tcp` cargo feature, default-on)."""

    URL = 0
    IO = 1
    PAYLOAD_TOO_LARGE = 2
    CLOSED = 3
    CONNECT_TIMEOUT = 4
    INVALID_CONFIG = 5
    TLS = 6
    TLS_DISABLED = 7


class TcpError(_KindMessageError):
    """Raised by `tstrans.tcp` operations. Discriminate via `.kind`."""

    kind: TcpErrorKind


class HlsErrorKind(enum.IntEnum):
    """Mirrors `tst_hls::HlsErrorKind`. Raised by `tstrans.hls`
    operations (built with the `hls` cargo feature, default-on)."""

    URL = 0
    IO = 1
    BIND_FAILED = 2
    INVALID_CONFIG = 3
    UNALIGNED_PUSH_TS = 4
    FINISHED = 5
    TLS_DISABLED = 6
    TLS = 7
    INTERNAL = 8


class HlsError(_KindMessageError):
    """Raised by `tstrans.hls` operations. Discriminate via `.kind`."""

    kind: HlsErrorKind


class RistErrorKind(enum.IntEnum):
    """Mirrors `tst_rist::RistErrorKind`. Raised by `tstrans.rist`
    operations (built with the `rist` cargo feature, default-on)."""

    URL = 0
    FFI = 1
    PAYLOAD_TOO_LARGE = 2
    CLOSED = 3
    INVALID_CONFIG = 4
    ENCRYPTION_DISABLED = 5
    CONTEXT_CREATE_FAILED = 6
    PEER_CREATE_FAILED = 7
    RECV_TIMEOUT = 8
    IO = 9


class RistError(_KindMessageError):
    """Raised by `tstrans.rist` operations. Discriminate via `.kind`."""

    kind: RistErrorKind


class CodecError(TstError):
    """Codec parser failure. See `CodecErrorKind` for the variant set.

    Variant-specific optional attributes:
    - `offset_bits` / `needed_bits` on TRUNCATED_RBSP / INVALID_GOLOMB
    - `field` / `value` on RESERVED_VALUE (Forbidden has only `field`)
    - `layer` on UNSUPPORTED_FREE_FORMAT
    - `profile_idc` on UNSUPPORTED_PROFILE
    - `sps_id` on DANGLING_SPS_REFERENCE
    - `vps_id` on DANGLING_VPS_REFERENCE
    - `offset_bytes` on INVALID_LEB128 / TRUNCATED
    - `expected` / `found` on BAD_SYNC_WORD
    - `needed` / `had` on TRUNCATED
    """

    def __init__(
        self,
        kind: CodecErrorKind,
        codec: str,
        message: str,
        *,
        offset_bits: Optional[int] = None,
        needed_bits: Optional[int] = None,
        field: Optional[str] = None,
        value: Optional[int] = None,
        profile_idc: Optional[int] = None,
        sps_id: Optional[int] = None,
        vps_id: Optional[int] = None,
        offset_bytes: Optional[int] = None,
        expected: Optional[int] = None,
        found: Optional[int] = None,
        needed: Optional[int] = None,
        had: Optional[int] = None,
        layer: Optional[int] = None,
    ) -> None:
        super().__init__(f"{codec}: {message}")
        self.kind = kind
        self.codec = codec
        self.message = message
        self.offset_bits = offset_bits
        self.needed_bits = needed_bits
        self.field = field
        self.value = value
        self.profile_idc = profile_idc
        self.sps_id = sps_id
        self.vps_id = vps_id
        self.offset_bytes = offset_bytes
        self.expected = expected
        self.found = found
        self.needed = needed
        self.had = had
        self.layer = layer


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
    "RtspError",
    "RtspErrorKind",
    "RtpError",
    "RtpErrorKind",
    "SrtError",
    "SrtErrorKind",
    "UdpErrorKind",
    "UdpError",
    "TcpErrorKind",
    "TcpError",
    "HlsErrorKind",
    "HlsError",
    "RistErrorKind",
    "RistError",
]
