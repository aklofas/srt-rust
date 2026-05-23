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

# Population happens task-by-task. __all__ accumulates as types land.
__all__: list[str] = []
