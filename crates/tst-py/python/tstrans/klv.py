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


__all__: list[str] = [
    "KlvFieldErrorKind",
    "KlvFieldError",
    "ST_0601_UL",
    "SECURITY_LS_UL",
    "PRECISION_TIMESTAMP_PACK_UL",
    "VMTI_LS_UL",
    "is_st0601_family",
]
