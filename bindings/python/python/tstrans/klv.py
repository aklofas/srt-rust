"""tstrans.klv — KLV typed sets (ST 0601, ST 0102, ST 0605, ST 0903).

Decode surface:

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

Symmetric `encode_*` entry points live alongside the decoders.
"""

import enum
from dataclasses import dataclass, replace


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

    def __post_init__(self) -> None:
        # Audit-2 #4 — the wire field is a single byte (0..=255); reject
        # out-of-range values early so callers see the construction site.
        if not 0 <= self.raw <= 0xFF:
            raise ValueError(f"TimeStatus.raw must be 0..=255; got {self.raw}")

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

    def with_(self, **changes: object) -> "PrecisionTimeStampPack":
        """Return a copy with the named fields replaced. The typed sets
        are frozen dataclasses (attribute assignment raises
        `FrozenInstanceError`) — this is the ergonomic update path.
        Thin wrapper over `dataclasses.replace`: unknown field names
        raise `TypeError`, and construction-time validation re-runs on
        the copy."""
        return replace(self, **changes)


# Spec-compat alias per design spec §API shape table.
Klv0605 = PrecisionTimeStampPack


# Re-export the Rust-side decode entry points. The Rust impls live in
# bindings/python/src/klv.rs and are exposed via `_native.decode_*`.
from tstrans import _native as _native_mod

decode_precision_timestamp = _native_mod.decode_precision_timestamp
encode_precision_timestamp = _native_mod.encode_precision_timestamp


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


# ---------------------------------------------------------------------------
# ST 0102.12 Security Metadata LS
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SecurityLs:
    """MISB ST 0102.12 Security Metadata Local Set typed view.

    Required tags per spec (§6.7 Table 1): 1, 2, 3, 12, 13, 22.
    Lenient decode tolerates missing required tags + unknown enum
    codepoints (surfaced as raw int) + malformed Tag 13 UTF-16
    (recorded in `field_errors`; raw bytes are NOT preserved in
    `unknown` for Tag 13 per Rust's decode comment — re-emitting
    malformed UTF-16 wouldn't help any consumer). Strict decode
    (`decode_security(buf, strict=True)`) rejects all of the above.

    Unknown enum codepoints on Tags 1 / 2 / 12 surface as the raw
    `int` codepoint rather than an enum instance — callers checking
    `isinstance(field, SecurityClassification)` will be False for
    out-of-spec codepoints; check `isinstance(..., int)` if you want
    to handle forward-compat values.
    """

    security_classification: SecurityClassification | int | None = None
    classifying_country_coding_method: ClassifyingCountryCodingMethod | int | None = None
    classifying_country: str | None = None
    object_country_coding_method: ObjectCountryCodingMethod | int | None = None
    object_country_codes: str | None = None
    version: int | None = None
    sci_shi_info: str | None = None
    caveats: str | None = None
    releasing_instructions: str | None = None
    classified_by: str | None = None
    derived_from: str | None = None
    classification_reason: str | None = None
    declassification_date: str | None = None
    classification_marking_system: str | None = None
    classification_comments: str | None = None
    classifying_country_coding_method_version_date: str | None = None
    object_country_coding_method_version_date: str | None = None
    unknown: tuple[tuple[int, bytes], ...] = ()
    field_errors: tuple[KlvFieldError, ...] = ()

    def with_(self, **changes: object) -> "SecurityLs":
        """Return a copy with the named fields replaced. The typed sets
        are frozen dataclasses (attribute assignment raises
        `FrozenInstanceError`) — this is the ergonomic update path.
        Thin wrapper over `dataclasses.replace`: unknown field names
        raise `TypeError`, and construction-time validation re-runs on
        the copy."""
        return replace(self, **changes)


# Spec-compat alias.
Klv0102 = SecurityLs


decode_security = _native_mod.decode_security
encode_security = _native_mod.encode_security
encode_security_strict_compliance = _native_mod.encode_security_strict_compliance


# ---------------------------------------------------------------------------
# ST 0903.6 §10.2 VTargetPack (carried inside VmtiLs.targets)
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class VTargetPack:
    """MISB ST 0903.6 §10.2 Table 10 per-target pack.

    Wire form is a leading BER-OID-encoded `target_id` (no Tag — per
    §10.2.2.1) followed by a Local Set body with BER-OID tag + BER
    short/long length + value tuples.

    7 nested LSes (VMask, VObject, VFeature, VTracker, VChip, VChipSeries,
    VObjectSeries) stay as `bytes | None` pass-through bytes — typed
    inner layers are deferred at the Rust layer too (see
    `docs/deferred-features.md`).

    `detection_status` is the raw §10.2.2.24 / §7.2 Table 5 codepoint:
    0=Inactive, 1=Active-Moving, 2=Dropped, 3=Active-Stopped,
    4=Active-Coasting. Typed enum deferred — stays as raw `int`."""

    target_id: int  # BER-OID, capped at u32::MAX

    def __post_init__(self) -> None:
        # Audit-2 #4 — target_color encodes as a 3-byte RGB value on the
        # wire (ST 0903.6 §10.2.2.8 Tag 8, 3 bytes); validate at
        # construction so callers see the error at their code site rather
        # than at the encode boundary.
        if self.target_color is not None:
            tc = self.target_color
            if len(tc) != 3:
                raise ValueError(
                    f"VTargetPack.target_color must be a 3-tuple (R, G, B); "
                    f"got length {len(tc)}"
                )
            if not all(0 <= ch <= 255 for ch in tc):
                raise ValueError(
                    f"VTargetPack.target_color channels must be 0..=255; "
                    f"got {tc!r}"
                )
    centroid_pixel: int | None = None
    bbox_top_left_pixel: int | None = None
    bbox_bottom_right_pixel: int | None = None
    priority: int | None = None
    confidence_level: int | None = None
    history: int | None = None
    percentage_of_target_pixels: int | None = None
    target_color: tuple[int, int, int] | None = None  # R, G, B
    target_intensity: int | None = None
    centroid_lat_offset: float | None = None
    centroid_lon_offset: float | None = None
    centroid_hae: float | None = None
    bbox_top_left_lat_offset: float | None = None
    bbox_top_left_lon_offset: float | None = None
    bbox_bottom_right_lat_offset: float | None = None
    bbox_bottom_right_lon_offset: float | None = None
    target_location: bytes | None = None
    geospatial_contour_series: bytes | None = None
    centroid_pix_row: int | None = None
    centroid_pix_col: int | None = None
    algorithm_id: int | None = None
    detection_status: int | None = None
    vmask: bytes | None = None
    vtracker: bytes | None = None
    vchip: bytes | None = None
    vchip_series: bytes | None = None
    vobject_series: bytes | None = None
    unknown: tuple[tuple[int, bytes], ...] = ()
    field_errors: tuple[KlvFieldError, ...] = ()


# ---------------------------------------------------------------------------
# ST 0903.6 VMTI Local Set
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class VmtiLs:
    """MISB ST 0903.6 VMTI (Video Moving Target Indicator) Local Set
    typed view.

    Required tags per ST 0903.6 §6 Table 1: precision_time_stamp,
    vmti_system_name (when applicable), version_number, frame_width,
    frame_height. Lenient decode tolerates missing required tags and
    surfaces per-field decode failures in `field_errors`. Strict
    decode rejects missing required tags.

    `algorithm_series` and `ontology_series` are top-level nested LS
    pass-through bytes (typed inner layers deferred at the Rust layer
    too — see `docs/deferred-features.md`).

    `miis_id` is the MISB ST 1204 Minor Item Identification System
    Core Identifier — pass-through bytes (typed layer deferred)."""

    checksum: int | None = None
    precision_time_stamp: int | None = None
    vmti_system_name: str | None = None
    version_number: int | None = None
    total_targets_in_frame: int | None = None
    num_targets_reported: int | None = None
    frame_width: int | None = None
    frame_height: int | None = None
    source_sensor: str | None = None
    horizontal_fov: float | None = None
    vertical_fov: float | None = None
    miis_id: bytes | None = None
    targets: tuple[VTargetPack, ...] = ()
    algorithm_series: bytes | None = None
    ontology_series: bytes | None = None
    unknown: tuple[tuple[int, bytes], ...] = ()
    field_errors: tuple[KlvFieldError, ...] = ()

    def with_(self, **changes: object) -> "VmtiLs":
        """Return a copy with the named fields replaced. The typed sets
        are frozen dataclasses (attribute assignment raises
        `FrozenInstanceError`) — this is the ergonomic update path.
        Thin wrapper over `dataclasses.replace`: unknown field names
        raise `TypeError`, and construction-time validation re-runs on
        the copy."""
        return replace(self, **changes)


# Spec-compat alias.
Klv0903 = VmtiLs


decode_vmti = _native_mod.decode_vmti
encode_vmti = _native_mod.encode_vmti
encode_vmti_standalone = _native_mod.encode_vmti_standalone
encode_vmti_strict_compliance = _native_mod.encode_vmti_strict_compliance
encode_vmti_standalone_strict_compliance = _native_mod.encode_vmti_standalone_strict_compliance


# ---------------------------------------------------------------------------
# ST 0601 composite read-only views
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class GeoPoint:
    """Lat / lon / alt triple. ST 0601 surfaces multiple GeoPoints
    (sensor position, frame center). Altitude is meters AMSL unless
    the source field specifies WGS84 ellipsoid height."""

    lat_deg: float
    lon_deg: float
    alt_m: float


@dataclass(frozen=True, slots=True)
class Attitude:
    """3-axis attitude in degrees. ST 0601 surfaces multiple
    Attitudes (sensor relative az/el/roll, platform heading/pitch/roll)."""

    heading_deg: float
    pitch_deg: float
    roll_deg: float


@dataclass(frozen=True, slots=True)
class FieldOfView:
    """Horizontal + vertical sensor field-of-view in degrees."""

    horizontal_deg: float
    vertical_deg: float


@dataclass(frozen=True, slots=True)
class Corners:
    """Four corner lat/lon points (upper-left looking forward). Each
    corner is a `(lat_deg, lon_deg)` tuple."""

    p1: tuple[float, float]
    p2: tuple[float, float]
    p3: tuple[float, float]
    p4: tuple[float, float]


# ---------------------------------------------------------------------------
# ST 0601 UAS Datalink Local Set
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class UasDatalinkLs:
    """MISB ST 0601 UAS Datalink Local Set typed view.

    Mirror of the 80-field Rust `tst_core::klv::st0601::UasDatalinkLs`
    flat struct. Composite views (sensor position, attitude, FOV,
    corners) are accessor methods that return `None` when any of the
    underlying primitive fields is absent.

    `security_local_set: bytes | None` carries the Tag 48 ST 0102 LS
    body bytes (no UL prefix); call `tstrans.klv.decode_security(...)`
    on it for typed access.

    `vmti: bytes | None` carries the Tag 74 ST 0903 LS body bytes (no
    UL prefix); call `tstrans.klv.decode_vmti(...)` on it for typed
    access.

    `unknown` preserves any tag not in the typed-modeled set per
    ST 0107.5 §6 future-proof skip rule.

    `field_errors` collects per-field decode failures from lenient
    mode; strict mode raises `KlvError` instead."""

    universal_label: bytes = b"\x00" * 16
    declared_version: int = 0

    def __post_init__(self) -> None:
        # Audit-2 #4 — universal_label is the SMPTE UL identifying this LS;
        # it is always exactly 16 bytes on the wire. Reject other lengths
        # at construction rather than at the encoder/PyO3 boundary.
        if len(self.universal_label) != 16:
            raise ValueError(
                f"UasDatalinkLs.universal_label must be exactly 16 bytes; "
                f"got {len(self.universal_label)}"
            )

    # Identity
    mission_id: str | None = None
    platform_tail_number: str | None = None
    platform_designation: str | None = None
    image_source_sensor: str | None = None
    image_coordinate_system: str | None = None
    platform_call_sign: str | None = None
    uas_ls_version: int | None = None
    # Time
    timestamp_us: int | None = None
    # Platform state
    platform_heading_deg: float | None = None
    platform_pitch_deg: float | None = None
    platform_roll_deg: float | None = None
    platform_true_airspeed: float | None = None
    platform_indicated_airspeed: float | None = None
    platform_pitch_full_deg: float | None = None
    platform_roll_full_deg: float | None = None
    platform_angle_of_attack_deg: float | None = None
    # Sensor pose & position
    sensor_lat_deg: float | None = None
    sensor_lon_deg: float | None = None
    sensor_alt_m: float | None = None
    sensor_ellipsoid_height_m: float | None = None
    sensor_hfov_deg: float | None = None
    sensor_vfov_deg: float | None = None
    sensor_rel_az_deg: float | None = None
    sensor_rel_el_deg: float | None = None
    sensor_rel_roll_deg: float | None = None
    # Ranging & frame center
    slant_range_m: float | None = None
    target_width_m: float | None = None
    frame_center_lat_deg: float | None = None
    frame_center_lon_deg: float | None = None
    frame_center_elev_m: float | None = None
    frame_center_ellipsoid_height_m: float | None = None
    # Image corner offsets (tags 26-33)
    corner_lat_offset_p1_deg: float | None = None
    corner_lon_offset_p1_deg: float | None = None
    corner_lat_offset_p2_deg: float | None = None
    corner_lon_offset_p2_deg: float | None = None
    corner_lat_offset_p3_deg: float | None = None
    corner_lon_offset_p3_deg: float | None = None
    corner_lat_offset_p4_deg: float | None = None
    corner_lon_offset_p4_deg: float | None = None
    # Image corner full lat/lon (tags 82-89)
    corner_lat_p1_deg: float | None = None
    corner_lon_p1_deg: float | None = None
    corner_lat_p2_deg: float | None = None
    corner_lon_p2_deg: float | None = None
    corner_lat_p3_deg: float | None = None
    corner_lon_p3_deg: float | None = None
    corner_lat_p4_deg: float | None = None
    corner_lon_p4_deg: float | None = None
    # Misc
    generic_flag_data: int | None = None
    security_local_set: bytes | None = None  # Tag 48 → ST 0102
    vmti: bytes | None = None  # Tag 74 → ST 0903
    # Pass-through
    unknown: tuple[tuple[int, bytes], ...] = ()
    field_errors: tuple[KlvFieldError, ...] = ()
    sentinel_tags: tuple[int, ...] = ()

    def with_(self, **changes: object) -> "UasDatalinkLs":
        """Return a copy with the named fields replaced. The typed sets
        are frozen dataclasses (attribute assignment raises
        `FrozenInstanceError`) — this is the ergonomic update path.
        Thin wrapper over `dataclasses.replace`: unknown field names
        raise `TypeError`, and construction-time validation re-runs on
        the copy."""
        return replace(self, **changes)

    def sensor_position(self) -> GeoPoint | None:
        if (
            self.sensor_lat_deg is not None
            and self.sensor_lon_deg is not None
            and self.sensor_alt_m is not None
        ):
            return GeoPoint(
                lat_deg=self.sensor_lat_deg,
                lon_deg=self.sensor_lon_deg,
                alt_m=self.sensor_alt_m,
            )
        return None

    def sensor_attitude(self) -> Attitude | None:
        if (
            self.sensor_rel_az_deg is not None
            and self.sensor_rel_el_deg is not None
            and self.sensor_rel_roll_deg is not None
        ):
            return Attitude(
                heading_deg=self.sensor_rel_az_deg,
                pitch_deg=self.sensor_rel_el_deg,
                roll_deg=self.sensor_rel_roll_deg,
            )
        return None

    def sensor_fov(self) -> FieldOfView | None:
        if self.sensor_hfov_deg is not None and self.sensor_vfov_deg is not None:
            return FieldOfView(
                horizontal_deg=self.sensor_hfov_deg,
                vertical_deg=self.sensor_vfov_deg,
            )
        return None

    def platform_attitude(self) -> Attitude | None:
        if (
            self.platform_heading_deg is not None
            and self.platform_pitch_deg is not None
            and self.platform_roll_deg is not None
        ):
            return Attitude(
                heading_deg=self.platform_heading_deg,
                pitch_deg=self.platform_pitch_deg,
                roll_deg=self.platform_roll_deg,
            )
        return None

    def frame_center(self) -> GeoPoint | None:
        if (
            self.frame_center_lat_deg is not None
            and self.frame_center_lon_deg is not None
            and self.frame_center_elev_m is not None
        ):
            return GeoPoint(
                lat_deg=self.frame_center_lat_deg,
                lon_deg=self.frame_center_lon_deg,
                alt_m=self.frame_center_elev_m,
            )
        return None

    def corners(self) -> Corners | None:
        # Prefer absolute (tags 82-89) when fully populated.
        absolute = (
            self.corner_lat_p1_deg,
            self.corner_lon_p1_deg,
            self.corner_lat_p2_deg,
            self.corner_lon_p2_deg,
            self.corner_lat_p3_deg,
            self.corner_lon_p3_deg,
            self.corner_lat_p4_deg,
            self.corner_lon_p4_deg,
        )
        if all(v is not None for v in absolute):
            return Corners(
                p1=(absolute[0], absolute[1]),  # type: ignore[arg-type]
                p2=(absolute[2], absolute[3]),  # type: ignore[arg-type]
                p3=(absolute[4], absolute[5]),  # type: ignore[arg-type]
                p4=(absolute[6], absolute[7]),  # type: ignore[arg-type]
            )
        # Fall back to offsets + frame center.
        if self.frame_center_lat_deg is None or self.frame_center_lon_deg is None:
            return None
        offsets = (
            self.corner_lat_offset_p1_deg,
            self.corner_lon_offset_p1_deg,
            self.corner_lat_offset_p2_deg,
            self.corner_lon_offset_p2_deg,
            self.corner_lat_offset_p3_deg,
            self.corner_lon_offset_p3_deg,
            self.corner_lat_offset_p4_deg,
            self.corner_lon_offset_p4_deg,
        )
        if not all(v is not None for v in offsets):
            return None
        lat0 = self.frame_center_lat_deg
        lon0 = self.frame_center_lon_deg
        return Corners(
            p1=(lat0 + offsets[0], lon0 + offsets[1]),  # type: ignore[operator]
            p2=(lat0 + offsets[2], lon0 + offsets[3]),  # type: ignore[operator]
            p3=(lat0 + offsets[4], lon0 + offsets[5]),  # type: ignore[operator]
            p4=(lat0 + offsets[6], lon0 + offsets[7]),  # type: ignore[operator]
        )


# Spec-compat alias.
Klv0601 = UasDatalinkLs


OutOfRangePolicy = _native_mod.OutOfRangePolicy

decode_uas_datalink = _native_mod.decode_uas_datalink
encode_uas_datalink = _native_mod.encode_uas_datalink
encode_uas_datalink_strict_compliance = _native_mod.encode_uas_datalink_strict_compliance


def patch_uas_datalink(raw: bytes, edits: UasDatalinkLs | dict[str, object]) -> bytes:
    """Patch named tags in a raw ST 0601 local set; every other TLV is
    copied byte-for-byte in original order and the Tag 1 checksum is
    recomputed (only if the input carries one).

    ``edits`` is either a ``dict`` of ``UasDatalinkLs`` field names
    (e.g. ``{"corner_lat_p1_deg": 33.99}``) or a partial
    ``UasDatalinkLs`` itself — ``None`` fields leave the input
    untouched. Tags outside the typed model can be replaced through the
    ``unknown`` field: ``{"unknown": ((103, b"..."),)}``. Edited tags
    absent from the input are inserted before the trailing checksum.
    Bytes after the declared outer length are preserved verbatim.
    ``universal_label`` and ``declared_version`` are accepted but
    ignored — the input's 16-byte UL is always copied verbatim.
    Deleting a tag is not supported.

    Unlike ``decode_uas_datalink`` → ``encode_uas_datalink`` round
    trips, this never reorders TLVs and never re-encodes values you did
    not name — vendor tags, unmodeled tags, and non-canonical length
    encodings all survive verbatim. (Editing a tag canonicalizes that
    one TLV's encoding, even if the new value equals the old.)

    Raises ``KlvError`` for a malformed input local set and
    ``KlvEncodeError`` for an edit value that cannot be encoded
    (out-of-range, string too long, typed tag in ``unknown``).
    """
    if isinstance(edits, dict):
        edits = UasDatalinkLs(**edits)
    return _native_mod.patch_uas_datalink(raw, edits)


# ---------------------------------------------------------------------------
# Universal Label dispatcher
# ---------------------------------------------------------------------------


def _read_ber_length(buf: bytes, offset: int) -> tuple[int, int]:
    """Read a BER short/long-form length starting at `offset`. Returns
    `(value, bytes_consumed)`. Raises ValueError if malformed."""
    if offset >= len(buf):
        raise ValueError("truncated BER length")
    first = buf[offset]
    if first < 0x80:
        return (first, 1)
    nbytes = first & 0x7F
    if nbytes == 0:
        raise ValueError("indefinite-length BER not permitted in KLV")
    if offset + 1 + nbytes > len(buf):
        raise ValueError("truncated BER long-form length")
    value = int.from_bytes(buf[offset + 1 : offset + 1 + nbytes], "big")
    return (value, 1 + nbytes)


def parse_klv_universal(buf: bytes, *, strict: bool = False):
    """Inspect the first 16 bytes of `buf` (the SMPTE Universal Label)
    and route to the matching typed decoder. Returns one of:

    - `UasDatalinkLs` (alias `Klv0601`) when the UL is in the ST 0601 family
    - `SecurityLs` (alias `Klv0102`) for `SECURITY_LS_UL`
    - `PrecisionTimeStampPack` (alias `Klv0605`) for `PRECISION_TIMESTAMP_PACK_UL`
    - `VmtiLs` (alias `Klv0903`) for `VMTI_LS_UL`
    - `None` when the UL doesn't match any known set

    With `strict=True`, the per-set decoder's strict mode is used:
    ST 0601 additionally requires the ST 0601 family UL pattern
    (bytes 13/14 are tolerated for legacy interop — see
    `is_st0601_family`); ST 0102 / ST 0903 reject missing required
    tags. ST 0605 has a single always-validating decode (no strict
    knob). Default is lenient — per-field issues land on the typed
    set's `.field_errors`.

    Raises `tstrans.exceptions.KlvError(BAD_UNIVERSAL_LABEL)` when
    `buf` is too short to contain a 16-byte UL.

    For the body-only sets (ST 0102, ST 0903), `parse_klv_universal`
    peels the UL + outer BER length wrapper before invoking the
    per-set decoder. For the others (ST 0601, ST 0605), the
    per-set decoder consumes the full buffer including the UL."""

    # Local import dodges any module-init ordering ambiguity.
    from tstrans.exceptions import KlvError, KlvErrorKind

    if len(buf) < 16:
        raise KlvError(
            kind=KlvErrorKind.BAD_UNIVERSAL_LABEL,
            message=f"buffer too short for 16-byte UL: have {len(buf)} bytes",
        )

    ul = buf[:16]

    if is_st0601_family(ul):
        return decode_uas_datalink(buf, strict=strict)
    if ul == PRECISION_TIMESTAMP_PACK_UL:
        return decode_precision_timestamp(buf)
    if ul == SECURITY_LS_UL:
        try:
            value_len, ber_bytes = _read_ber_length(buf, 16)
        except ValueError as e:
            raise KlvError(
                kind=KlvErrorKind.TRUNCATED_SET,
                message=f"ST 0102 outer BER length unreadable: {e}",
            ) from e
        body_start = 16 + ber_bytes
        body_end = body_start + value_len
        if body_end > len(buf):
            raise KlvError(
                kind=KlvErrorKind.TRUNCATED_SET,
                message=(
                    f"ST 0102 declared body length {value_len} exceeds "
                    f"available {len(buf) - body_start}"
                ),
            )
        if body_end < len(buf):
            raise KlvError(
                kind=KlvErrorKind.MALFORMED_BYTES,
                message=(
                    f"ST 0102 universal record has {len(buf) - body_end} "
                    f"trailing bytes after declared body length {value_len}"
                ),
            )
        return decode_security(buf[body_start:body_end], strict=strict)
    if ul == VMTI_LS_UL:
        try:
            value_len, ber_bytes = _read_ber_length(buf, 16)
        except ValueError as e:
            raise KlvError(
                kind=KlvErrorKind.TRUNCATED_SET,
                message=f"ST 0903 outer BER length unreadable: {e}",
            ) from e
        body_start = 16 + ber_bytes
        body_end = body_start + value_len
        if body_end > len(buf):
            raise KlvError(
                kind=KlvErrorKind.TRUNCATED_SET,
                message=(
                    f"ST 0903 declared body length {value_len} exceeds "
                    f"available {len(buf) - body_start}"
                ),
            )
        if body_end < len(buf):
            raise KlvError(
                kind=KlvErrorKind.MALFORMED_BYTES,
                message=(
                    f"ST 0903 universal record has {len(buf) - body_end} "
                    f"trailing bytes after declared body length {value_len}"
                ),
            )
        return decode_vmti(buf[body_start:body_end], strict=strict)

    return None


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
    "encode_precision_timestamp",
    "SecurityClassification",
    "ClassifyingCountryCodingMethod",
    "ObjectCountryCodingMethod",
    "SecurityLs",
    "Klv0102",
    "decode_security",
    "encode_security",
    "encode_security_strict_compliance",
    "VTargetPack",
    "VmtiLs",
    "Klv0903",
    "decode_vmti",
    "encode_vmti",
    "encode_vmti_standalone",
    "encode_vmti_strict_compliance",
    "encode_vmti_standalone_strict_compliance",
    "GeoPoint",
    "Attitude",
    "FieldOfView",
    "Corners",
    "OutOfRangePolicy",
    "UasDatalinkLs",
    "Klv0601",
    "decode_uas_datalink",
    "encode_uas_datalink",
    "encode_uas_datalink_strict_compliance",
    "patch_uas_datalink",
    "parse_klv_universal",
]
