"""tstrans.klv — KLV typed sets (ST 0601, ST 0102, ST 0605, ST 0903).

Phase 3 of the tst-py v1 plan added the KLV decode surface:

- `TimeStatus` — ST 0603 §7.4 time-status byte wrapper
- `PrecisionTimeStampPack` (alias `Klv0605`) — ST 0605 §7 pack
- `SecurityClassification`, `ClassifyingCountryCodingMethod`,
  `ObjectCountryCodingMethod` — ST 0102 §6.1 enums
- `SecurityLs` (alias `Klv0102`) — ST 0102 Security Metadata LS
- `VTargetPack` — ST 0903 §10.2 per-target pack
- `VmtiLs` (alias `Klv0903`) — ST 0903 VMTI LS
- `GeoPoint`, `Attitude`, `FieldOfView`, `Corners` — ST 0601 composites
- `UasDatalinkLs` (alias `Klv0601`) — ST 0601 UAS Datalink LS
- `KlvFieldError`, `KlvFieldErrorKind` — non-fatal per-field errors
- `decode_uas_datalink`, `decode_security`, `decode_precision_timestamp`,
  `decode_vmti` — per-set entry points
- `parse_klv_universal` — UL-dispatching universal entry point
- `ST_0601_UL`, `SECURITY_LS_UL`, `PRECISION_TIMESTAMP_PACK_UL`,
  `VMTI_LS_UL` — well-known 16-byte UL constants
- `is_st0601_family` — predicate for the ST 0601 UL family
  (tolerates legacy non-zero byte 13 + byte 14)

Phase 4 (Muxer) adds the symmetric `encode_*` surface.
"""

import enum
from dataclasses import dataclass


class KlvFieldErrorKind(enum.Enum):
    """Mirrors Rust's `tst_core::error::KlvFieldError` variants.

    Field-level decode failures are recoverable — they do NOT raise;
    they accumulate on the typed-set object's `.field_errors` list per
    the design spec's "best-effort parse" semantics for ST 0601 in the
    field. Marked `#[non_exhaustive]` on the Rust side; Python matchers
    should include a default arm."""

    OUT_OF_RANGE = "out_of_range"
    INVALID_UTF8 = "invalid_utf8"
    INVALID_UTF16 = "invalid_utf16"
    INVALID_LENGTH = "invalid_length"
    INVALID_SENTINEL = "invalid_sentinel"
    INVALID_CODEPOINT = "invalid_codepoint"
    TRUNCATED_FIELD = "truncated_field"
    UNSUPPORTED_IMAPB_LENGTH = "unsupported_imapb_length"
    INVALID_IMAPB_PARAMS = "invalid_imapb_params"


@dataclass(frozen=True, slots=True)
class KlvFieldError:
    """Non-fatal per-field decode failure. Each decoded typed-set
    object carries a `.field_errors: tuple[KlvFieldError, ...]`
    collecting these. `tag` is the BER-OID-decoded tag (u32 to cover
    forward-compat multi-byte BER-OID); `message` is the
    `Display` string from the Rust variant."""

    kind: KlvFieldErrorKind
    tag: int
    message: str


# ---------------------------------------------------------------------------
# Well-known SMPTE / MISB Universal Labels (16 bytes each)
# ---------------------------------------------------------------------------

# Per MISB ST 0601.19 §6.2 (PDF p.4) — bytes 13/14/15 all 0x00.
# UL CRC 56773.
ST_0601_UL: bytes = bytes.fromhex("060e2b34020b01010e01030101000000")

# Per MISB ST 0102.12 §6.7 — Security Metadata LS UL. CRC 40980.
SECURITY_LS_UL: bytes = bytes.fromhex("060e2b34020301010e01030302000000")

# Per MISB ST 0807.27 row 1061 — Precision Time Stamp Pack UL. CRC 23259.
PRECISION_TIMESTAMP_PACK_UL: bytes = bytes.fromhex("060e2b34020501010e01010311000000")

# Per MISB ST 0903.6 §10.1 — VMTI LS UL.
VMTI_LS_UL: bytes = bytes.fromhex("060e2b34020b01010e01030306000000")


def is_st0601_family(buf: bytes) -> bool:
    """Mirror of Rust `UniversalLabel::is_st0601_family`. Returns True
    iff `buf` is at least 16 bytes AND bytes 0..=12 match the canonical
    ST 0601 family prefix AND byte 15 is 0x00. Bytes 13 (document
    version byte) and 14 are tolerated for legacy capture interop
    per ST 0601.8-19's transitional rule."""

    if len(buf) < 16:
        return False
    canonical = bytes.fromhex("060e2b34020b01010e01030101")
    if buf[:13] != canonical:
        return False
    return buf[15] == 0x00


# ---------------------------------------------------------------------------
# ST 0605 §7 Precision Time Stamp Pack
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TimeStatus:
    """Time Status byte per MISB ST 0603 §7.4 Table 3.

    - bit 7: 0 = Locked, 1 = Lock Unknown
    - bit 6: 0 = Normal, 1 = Discontinuity
    - bit 5: 0 = Forward, 1 = Reverse (only meaningful when bit 6=1)
    - bits 4-0: reserved, must be 0b11111"""

    raw: int

    @property
    def is_locked(self) -> bool:
        """True if bit 7 = 0 (clock locked to absolute time reference)."""
        return (self.raw & 0x80) == 0

    @property
    def has_discontinuity(self) -> bool:
        """True if bit 6 = 1 (time has not incremented linearly forward)."""
        return (self.raw & 0x40) != 0

    @property
    def is_reverse_jump(self) -> bool:
        """True if bit 5 = 1 (only meaningful when `has_discontinuity` —
        indicates a backward time jump rather than forward)."""
        return (self.raw & 0x20) != 0

    @property
    def reserved_bits_valid(self) -> bool:
        """True if reserved bits 4-0 are the spec-required 0b11111."""
        return (self.raw & 0x1F) == 0x1F


@dataclass(frozen=True, slots=True)
class PrecisionTimeStampPack:
    """MISB ST 0605 §7 Precision Time Stamp Pack typed view.

    Wire form is 26 bytes: 16-byte UL + 1-byte BER length (`0x09`) +
    1-byte `TimeStatus` + 8-byte big-endian microsecond timestamp."""

    time_status: TimeStatus
    timestamp_us: int


# Spec-compat alias per design spec §API shape table.
Klv0605 = PrecisionTimeStampPack


# Re-export the Rust-side decode entry points. The Rust impls live in
# crates/tst-py/src/klv.rs and are exposed via `_native.decode_*`.
from tstrans import _native as _native_mod

decode_precision_timestamp = _native_mod.decode_precision_timestamp


# ---------------------------------------------------------------------------
# ST 0102.12 §6.1 enums
# ---------------------------------------------------------------------------


class SecurityClassification(enum.Enum):
    """ST 0102.12 §6.1.1 Tag 1 Security Classification.

    Rust adds an `Unknown(u8)` arm for forward-compat; on the Python
    side, unknown codepoints surface as the raw `int` on the
    `SecurityLs.security_classification` field rather than an enum
    instance (kept simple — the typed enum is the high-fidelity 90%
    path)."""

    UNCLASSIFIED = 0x01
    RESTRICTED = 0x02
    CONFIDENTIAL = 0x03
    SECRET = 0x04
    TOP_SECRET = 0x05


class ClassifyingCountryCodingMethod(enum.Enum):
    """ST 0102.12 §6.1.2 Tag 2 Classifying Country / Releasing
    Instructions Country Coding Method.

    Tag 2 and Tag 12 use DIFFERENT codepoints for the same logical
    coding method — see `ObjectCountryCodingMethod` for Tag 12 values.
    `OmittedValueXX` slots are spec-reserved; strict-mode decode
    rejects them."""

    ISO_3166_TWO_LETTER = 0x01
    ISO_3166_THREE_LETTER = 0x02
    FIPS_104_TWO_LETTER = 0x03
    FIPS_104_FOUR_LETTER = 0x04
    ISO_3166_NUMERIC = 0x05
    STANAG_1059_TWO_LETTER = 0x06
    STANAG_1059_THREE_LETTER = 0x07
    OMITTED_VALUE_08 = 0x08
    OMITTED_VALUE_09 = 0x09
    FIPS_104_MIXED = 0x0A
    ISO_3166_MIXED = 0x0B
    STANAG_1059_MIXED = 0x0C
    GENC_TWO_LETTER = 0x0D
    GENC_THREE_LETTER = 0x0E
    GENC_NUMERIC = 0x0F
    GENC_MIXED = 0x10


class ObjectCountryCodingMethod(enum.Enum):
    """ST 0102.12 §6.1.12 Tag 12 Object Country Coding Method.

    Codepoints differ from Tag 2 — the spec is non-contiguous and
    jumps to 0x40 for `GencAdminSub`."""

    ISO_3166_TWO_LETTER = 0x01
    ISO_3166_THREE_LETTER = 0x02
    ISO_3166_NUMERIC = 0x03  # vs Tag 2's 0x05
    FIPS_104_TWO_LETTER = 0x04  # vs Tag 2's 0x03
    FIPS_104_FOUR_LETTER = 0x05  # vs Tag 2's 0x04
    STANAG_1059_TWO_LETTER = 0x06
    STANAG_1059_THREE_LETTER = 0x07
    OMITTED_VALUE_08 = 0x08
    OMITTED_VALUE_09 = 0x09
    OMITTED_VALUE_0A = 0x0A
    OMITTED_VALUE_0B = 0x0B
    OMITTED_VALUE_0C = 0x0C
    GENC_TWO_LETTER = 0x0D
    GENC_THREE_LETTER = 0x0E
    GENC_NUMERIC = 0x0F
    GENC_ADMIN_SUB = 0x40


__all__: list[str] = [
    "KlvFieldErrorKind",
    "KlvFieldError",
    "ST_0601_UL",
    "SECURITY_LS_UL",
    "PRECISION_TIMESTAMP_PACK_UL",
    "VMTI_LS_UL",
    "is_st0601_family",
    "TimeStatus",
    "PrecisionTimeStampPack",
    "Klv0605",
    "decode_precision_timestamp",
    "SecurityClassification",
    "ClassifyingCountryCodingMethod",
    "ObjectCountryCodingMethod",
]
