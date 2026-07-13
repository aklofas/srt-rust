# KLV Codec Guide


> **Who this is for:** You need to encode or decode MISB KLV metadata (ST 0601 FMV, ST 0102 security, ST 0605 amend tags, ST 0903 VMTI) — typed Rust structs in, bytes out, or the inverse.

> **You will learn:**
> - The substrate: SMPTE UL tags, BER length, the encode/decode round-trip
> - How typed `St0601Record` / `St0102Record` / `St0605Record` / `St0903Record` map to the wire format
> - The non-conformant-issue model (lenient decode + diagnostics) vs strict mode
> - How `KlvStreamType::SynchronousMetadata` wraps your KLV in H.222.0 §2.12.4.2 AU cells (and what to pass — raw KLV, not pre-wrapped)
> - VTargetPack inner structure for ST 0903
> - How to encode strict-compliance ST 0601 with `encode_strict_compliance`

## Introduction

When you need to put MISB-typed metadata onto an MPEG-TS — sensor pose,
platform telemetry, security marking, target tracks — `tst_core::klv` is the
bidirectional codec. It covers the MISB ST 0601 UAS Datalink Local Set, the
ST 0102 Security Metadata Universal Set, the ST 0605 Precision Time Stamp Pack,
and the ST 0903 VMTI Local Set, all layered on a generic SMPTE KLV substrate
(Universal Labels, BER lengths, IMAPB, checksum, pack iteration). MPEG-TS
sync-metadata AU cell carriage lives at `mpegts::au_cell` (per
ITU-T H.222.0 V9 § 2.12.4.2) — the muxer auto-wraps for
`KlvStreamType::SynchronousMetadata` streams.

What this module is *not*: a TS demuxer. Pulling KLV out of a captured
`.ts` file is done with FFmpeg / Bento4 / `cargo run -p tst-examples --example extract_klv`
(see Section 11). TS demux in the Rust core is on the deferred list — see
[`mpegts::demux`](/docs/project/deferred-features.md) in `deferred-features.md`.

> **Python:** `tstrans` ships `py.typed` type stubs for the core `io`/`codec`/`klv`/`mpegts` modules, so editors and `mypy` resolve these types directly.

Pick the decode entry point that fits your situation: `decode` for
general-purpose decoding (verifies checksum, accepts any UL); reach for
`decode_strict_compliance` only when validating producer output against
the published ST 0601 mandatory-field rules.

## Two layers

```
typed:    klv::st0601 (UAS Datalink LS, 49 typed items)
          klv::st0605 (Precision Time Stamp Pack)

substrate: klv::pack            (Iter, RawField, OwnedRawField)
           klv::length          (BER short/long, BER-OID)
           klv::imapb           (ST 1201.5 int↔float)
           klv::checksum        (16-bit running-sum)
           klv::universal_label (UniversalLabel)
```

The substrate is the part of the module that knows raw KLV machinery and
nothing about which standard set is on the wire. Reach for it when
you're working with a custom local set, debugging a capture byte by byte,
or translating fields without committing to a typed shape — anything
where "give me the next `(tag, len, value)` triple" is the right
abstraction. The substrate types are deliberately small and zero-copy
where the borrow checker allows; `pack::Iter::local_set(buf)` walks a
body without allocating, and `RawField<'a>` borrows its `value` slice
straight from the input buffer.

The typed layer is the right entry point for production decoding and
encoding of ST 0601 / ST 0605 / ST 0903. `klv::st0601::UasDatalinkLs` is
a flat plain-old-data struct: every typed item is `Option<T>`, the
unrecognized tags pass through in a `Vec<OwnedRawField>`, and the four
decode entry points (`decode`, `decode_unchecked`, `decode_strict`,
`decode_strict_compliance`) form a strictness ladder you walk top-down
when working with real captures (see Section 4). For most callers, the
typed layer is the correct level — drop to the substrate only when the
typed shape doesn't fit.

## Universal Labels

`UniversalLabel` is a 16-byte struct mirroring the SMPTE Universal Label
format from SMPTE 336M (the canonical SMPTE UL format spec, also
referenced as MISB ST 0107). Each byte has a documented role:

- bytes 0-3: SMPTE OID prefix (`UniversalLabel::oid`).
- byte 4: category designator (`category`).
- byte 5: registry designator (`registry`).
- byte 6: structure designator (`structure`).
- byte 13: `0x00` per ST 0601.19 §6.2 canonical registration. Some
  legacy captures ship a non-zero byte 13 reflecting the older
  "document version" convention (e.g. `0x13` = ST 0601.19); the
  accessor `st0601_version_byte()` returns the raw byte for legacy interop.
  ST 0601.8-19 forbids non-zero values in new developments.

The `klv::universal_label` module exposes well-known constants —
`ST_0601_LS`, `SMPTE_336M_LS_KEY`, `PRECISION_TIMESTAMP_PACK_UL` — and a
family check: `is_st0601_family()` returns true when the label belongs
to the ST 0601 family. The check validates bytes 0-12 against the
canonical prefix (universal designator + ST 0601 set kind) and requires
byte 15 to be `0x00`. Bytes 13 and 14 are tolerated at any value by
the family gate so legacy captures still round-trip. The constructor
is non-validating —
`UniversalLabel::new([..])` accepts any 16 bytes — because real-world
records do contain malformed or non-standard labels, and the typed
layer's `decode_strict` is the opt-in validation point.

```rust,no_run
use tst_core::klv::UniversalLabel;

fn inspect(buf: &[u8; 16]) {
    let ul = UniversalLabel::new(*buf);
    println!("oid={:02X?} category={:02X} registry={:02X}",
             ul.oid(), ul.category(), ul.registry());
    println!("structure={:02X} byte_13={:02X}",
             ul.structure(), ul.st0601_version_byte());
    if ul.is_st0601_family() {
        println!("ST 0601 family — byte_13={:02X} (canonical 0x00)",
                 ul.st0601_version_byte());
    } else {
        println!("not ST 0601 family — got {ul}");
    }
}
```

## The strictness ladder

The four decode entry points on `klv::st0601` differ in what they verify
before returning a `UasDatalinkLs`:

| Function | Checksum check? | UL gate? | Compliance rules? | When to use |
| --- | --- | --- | --- | --- |
| `decode` | Yes | No | No | Default — accept any UL, reject corrupted bytes |
| `decode_unchecked` | No | No | No | Diagnostic — bypass checksum to inspect malformed payloads |
| `decode_strict` | Yes | Yes (ST 0601 family) | No | Reject non-ST-0601 records |
| `decode_strict_compliance` | Yes | Yes | Yes (ST 0601.8 §10.3.1, §10.3.2, §10.3.3) | Production validation |

The "compliance rules" column refers to ST 0601.8 §10.3 mandatory
ordering: Tag 2 (Precision Time Stamp) first, Tag 1 (Checksum) last,
Tag 65 (UAS LS Version Number) present. These hardened in ST 0601.8 and
remain mandatory through ST 0601.9 / .11 / .12 / .19 — many older
captures violate them in benign ways, which is why the compliance rung
is opt-in.

The recommended pattern is to walk the ladder top-down — try
`decode_strict_compliance` first, fall back through `decode_strict`,
`decode`, and finally `decode_unchecked`. Each fall-back surfaces a more
permissive interpretation of the bytes, and the rejection error from the
previous rung tells you why the stricter rung said no:

```rust,no_run
use tst_core::klv::st0601::{
    decode, decode_strict, decode_strict_compliance, decode_unchecked, UasDatalinkLs,
};

fn try_all(buf: &[u8]) -> Result<UasDatalinkLs, Box<dyn std::error::Error>> {
    if let Ok(rec) = decode_strict_compliance(buf) {
        return Ok(rec);
    }
    if let Ok(rec) = decode_strict(buf) {
        return Ok(rec);
    }
    if let Ok(rec) = decode(buf) {
        return Ok(rec);
    }
    let rec = decode_unchecked(buf)?;
    Ok(rec)
}
```

For the worked example with per-rung error reporting, see
[../examples/klv-metadata/klv_decode_file.rs](/examples/klv-metadata/klv_decode_file.rs).

## Typed ST 0601 — `UasDatalinkLs`

`UasDatalinkLs` is a flat plain-old-data struct that mirrors the wire
format directly. Every typed item is an `Option<T>` — present when the
record carried that tag, `None` otherwise. The struct also carries the
parsed Universal Label and a declared version byte read from the UL.

Composite views are derived read-only methods that combine several typed
fields into a more meaningful shape:

- `sensor_position() -> Option<GeoPoint>` — combines Tags 13 / 14 / 15.
- `sensor_attitude() -> Option<Attitude>` — combines Tags 18 / 19 / 20.
- `sensor_fov() -> Option<FieldOfView>` — combines Tags 16 / 17.
- `platform_attitude() -> Option<Attitude>` — combines Tags 5 / 6 / 7.
- `frame_center() -> Option<GeoPoint>` — combines Tags 23 / 24 / 25.
- `corners() -> Option<Corners>` — prefers absolute Tags 82-89 when
  fully populated, else falls back to offset Tags 26-33 plus frame
  center.

Each composite returns `None` if any constituent field is missing — so a
caller doesn't have to spell out the partial-presence cases.

> **Python:** the typed sets are frozen dataclasses — attribute
> assignment raises `FrozenInstanceError`. Use
> `record.with_(sensor_lat_deg=33.5)` to get an updated copy (a thin
> `dataclasses.replace` wrapper on all four sets; unknown names raise
> `TypeError` and construction-time validation re-runs on the copy).
> To stream typed ST 0601 records straight from a file with their PTS,
> use `tstrans.io.iter_uas_datalink(path)`; on a `DemuxEvent.Metadata`
> demux event, `ev.parse()` dispatches by universal label.

Unknown-tag pass-through. Any tag not in the typed table lives in
`record.unknown: Vec<OwnedRawField>` per the ST 0107.5 future-proof skip
rule. This is what lets a record produced by a newer ST 0601 revision
round-trip through an older decoder without losing data — the
unrecognized tags survive in `unknown` and are re-emitted by
`encode_to_vec`.

Per-field decode errors. When one tag's value is malformed (wrong
length, invalid UTF-8, out-of-range mapped float), the decoder records
a `KlvFieldError` in `record.field_errors` and continues with the rest
of the record. The whole-record decode only fails on structural problems
(truncated buffer, malformed BER length, checksum mismatch under
verifying decoders, missing mandatory tags under
`decode_strict_compliance`).

For the complete typed-item list — which tags are typed, which are still
pass-through — see [reference/compatibility.md](/docs/reference/compatibility.md).

## Encoding

`encode_to_vec(&record) -> Result<Vec<u8>, KlvEncodeError>` is the happy
path. It allocates a buffer the right size, encodes, and returns the
bytes. The encoder auto-emits Tag 1 (the 16-bit running-sum checksum,
mandated last by ST 0601 §6.3) and Tag 65 (UAS LS Version Number,
mandated present by ST 0601.8 §10.3) when the caller didn't set them
explicitly — so a default-constructed record with a few typed fields
produces wire bytes that satisfy `decode_strict_compliance` out of the
box.

`encode_with(&record, &EncodeConfig, &mut [u8]) -> Result<usize, KlvEncodeError>`
is the in-place form for callers who want to control the output buffer
or override the Universal Label or version byte. `EncodeConfig` carries
two fields: `universal_label: UniversalLabel` (defaults to
`UniversalLabel::ST_0601_LS`) and `version: u8` (the Tag 65 value, default
`19` = ST 0601.19; decoupled from the UL's byte 13, which is `0x00` on the
canonical label per §6.2). Pre-size the output buffer
with `encoded_len(&record)` if you want to allocate exactly.

```rust,no_run
use tst_core::klv::st0601::{encode_to_vec, UasDatalinkLs};

fn build_minimal() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut rec = UasDatalinkLs::default();
    rec.timestamp_us = Some(1_700_000_000_000_000);
    rec.platform_designation = Some("test-platform".into());
    rec.sensor_lat_deg = Some(33.6800);
    rec.sensor_lon_deg = Some(-118.5500);
    rec.sensor_alt_m = Some(3500.0);
    let bytes = encode_to_vec(&rec)?;
    Ok(bytes)
}
```

For a worked example with attitude + sensor pose + frame center, plus
explicit `LinearRange` step-size calculations for each ranged tag, see
[../examples/klv-metadata/klv_encode_minimal.rs](/examples/klv-metadata/klv_encode_minimal.rs).

### Strict-compliance encode

`encode_strict_compliance(&record) -> Result<Vec<u8>, KlvEncodeError>`
is the mirror of `decode_strict_compliance` on the encode side. It runs
the same ST 0601.8 §10.3 mandatory-structure checks *before* serialising,
returning `KlvEncodeError::MissingMandatoryItem { tag, name }` rather
than silently producing wire bytes a strict decoder would reject. Use
this when you want a single round-trip invariant — "if it encodes,
`decode_strict_compliance` will accept it." Auto-emission of Tag 1
(checksum) and Tag 65 (LS Version) is unchanged.

`encode_to_vec` remains the lenient path: it produces conformant bytes
when the caller has set the conventional fields, but it does not gate
on every ST 0601.8-mandated item.

## Surgical tag patching

`decode` → modify → `encode` round-trips are *lenient*: they re-emit
only what the typed model carries, normalize TLV order, and re-encode
every IMAPB value. For "change a few tags, keep the rest identical"
edits — metadata correction, redaction, annotation — use the
byte-faithful patcher instead.

```rust,no_run
use tst_core::error::KlvPatchError;
use tst_core::klv::st0601::{patch, UasDatalinkLs};

fn correct_corners(raw_ls: &[u8]) -> Result<Vec<u8>, KlvPatchError> {
    let edits = UasDatalinkLs {
        corner_lat_p1_deg: Some(33.99),
        corner_lon_p1_deg: Some(-117.61),
        ..UasDatalinkLs::default()
    };
    patch(raw_ls, &edits)
}
```

```python
patched = klv.patch_uas_datalink(raw_ls, {
    "corner_lat_p1_deg": 33.99,
    "corner_lon_p1_deg": -117.61,
})
```

Only the named tags are re-encoded; every other TLV — vendor tags,
unmodeled tags, non-canonical length encodings, bytes after the
declared outer length — is copied byte-for-byte in original order,
and the Tag 1 checksum is recomputed (only if the input carries one).
Edited tags absent from the input are inserted before the trailing
checksum. Tags outside the typed model can be replaced via the
`unknown` field (`{"unknown": ((tag, value_bytes),)}`). Editing a tag
canonicalizes that one TLV's encoding even when the value is
unchanged. Deleting a tag is not supported.

## ST 0605 Precision Time Stamp Pack

ST 0605 is a separate KLV record from ST 0601 Tag 2, and the two
typically pair up in real captures: each video frame carries an ST 0601
Local Set on one PID for the geolocation payload and an ST 0605 pack
elsewhere for the higher-precision timestamp the producer attached at
PES emit time.

Different scope. ST 0601 Tag 2 is just an 8-byte microseconds-since-
epoch field embedded in the Local Set. ST 0605 §7 is a standalone 26-
byte pack (`[UL:16][BER 0x09:1][status:1][microseconds:8 BE]`) carrying
both the timestamp and a status byte that flags whether the producer's
clock is locked to an absolute time reference, whether a discontinuity
occurred, and whether the discontinuity was a forward or backward jump.

The typed view is `PrecisionTimeStampPack { time_status: TimeStatus,
timestamp_us: u64 }`. `TimeStatus(u8)` is a newtype wrapping the spec's
1-byte status field per MISB ST 0603 §7.4 Table 3, with four `const`
accessors:

- `is_locked()` — true when bit 7 = 0 (clock locked to absolute time).
- `has_discontinuity()` — true when bit 6 = 1 (time has not incremented
  forward in a linear fashion).
- `is_reverse_jump()` — true when bit 5 = 1 (only meaningful when
  `has_discontinuity()`).
- `reserved_bits_valid()` — true when bits 4-0 are the spec-required
  `0b11111`.

`klv::st0605::decode(buf)` parses a 26-byte buffer starting with the
canonical UL; `klv::st0605::encode(&pack) -> [u8; 26]` produces the
fixed-size bytes for transmission.

## Typed Security Local Set (`klv::st0102`)

ST 0601 Tag 48 (`security_local_set`) carries an MISB ST 0102.12 Security
Metadata Local Set — classification level, country codes, declassification
date, and related fields. The parent `UasDatalinkLs` surfaces it as
`Option<Vec<u8>>` (pass-through bytes, not coupled to the parent decoder).
The typed parser at `tst_core::klv::st0102` is a sibling-layer module:
consumers who decoded an ST 0601 record and want typed access to the
security metadata call `klv::st0102::decode` on the inner bytes.

```rust
use tst_core::klv::{st0102, st0601};

let parent = st0601::decode(&record_bytes)?;
if let Some(bytes) = parent.security_local_set.as_deref() {
    let security = st0102::decode(bytes)?;
    println!("classification: {:?}", security.security_classification);
}
```

Two decode entry points:

- `decode(bytes)` — **lenient.** Tolerates missing tags, unknown enum
  codepoints (decoded as `Unknown(u8)`), unknown LS tags (preserved in
  `unknown: Vec<OwnedRawField>`), malformed UTF-16 on Tag 13 (signaled
  via `field_errors: Vec<KlvFieldError>`).
- `decode_strict(bytes)` — **strict.** Rejects records missing any of
  the spec-mandatory tags (1, 2, 3, 12, 13, 22), rejects unknown enum
  codepoints and `OmittedValueXX` reserved slots on Tags 1/2/12,
  rejects malformed UTF-16 on Tag 13, rejects duplicate tags. Unknown
  LS tags are still preserved per ST 0107.5 §6 future-proof skip rule
  (matches `klv::st0601::decode_strict_compliance` posture).

Encode is symmetric (`encode`, `encode_to_vec`, `encoded_len`). Tag 13
(Object Country Codes) is RFC 2781 UTF-16 — encode emits BE BOM +
UTF-16 BE; decode accepts either endianness via BOM or defaults to BE
per RFC 2781 §4.3.

**What's not modeled:**
- Universal Set form of ST 0102 (LS-only on MPEG-TS+KLV streams).
- Country-code validation against ISO 3166 / GENC / FIPS 10-4 /
  STANAG 1059 / CAPCO tables — codes pass through as `String`
  verbatim.
- Calendar parsing of date fields (Tag 10 "YYYYMMDD", Tags 23/24
  "YYYY-MM-DD") — pass through as `String`.

See `examples/klv-metadata/decode_security_metadata.rs` for a runnable file walker
that demonstrates the sibling-layer composition pattern end-to-end.

## Typed VMTI Local Set (`klv::st0903`)

VMTI (Video Moving Target Indicator) per MISB ST 0903.6 carries
detected/tracked moving objects in a video frame: per-target
centroids, bounding boxes, lat/lon, classifications, track IDs,
confidence levels. Carried as ST 0601 Tag 74 in most real ISR
captures, or standalone on its own KLV PID.

`klv::st0903` ships:

- `VmtiLs` typed struct — top-level frame-level fields (timestamp,
  frame dims, sensor FOV, target counts) + `targets: Vec<VTargetPack>`.
- `VTargetPack` typed struct — per-target structural data: target ID,
  centroid pixel, bbox pixels, priority, confidence, dimensions
  (in meters via IMAPB), centroid lat/lon offsets + HAE, bounding
  box geo offsets, color, intensity, detection status, algorithm ID,
  pixel row/col, target location DLP, geospatial contour series.
- `decode` (lenient) + `decode_strict` (rejects missing required tags
  per ST 0903.6 §10.1.4 + §10.1.6 + duplicates + malformed values).
- Symmetric `encode` / `encode_to_vec` / `encoded_len` for synthetic
  fixture generation and round-trip testing.
- `VMTI_LS_UL` constant for standalone-PID consumers.

Seven nested/sibling Local Sets — `VMask`, `VTracker`, `VChip`,
`VChipSeries`, `VObjectSeries` (per-target) plus `Algorithm Series`
and `Ontology Series` (top-level) — stay as `Option<Vec<u8>>`
pass-through bytes. Typed layers for those are deferred (see
[`deferred-features.md`](/docs/project/deferred-features.md)).

### Sibling-layer pattern

`klv::st0601` doesn't recurse into Tag 74. The parent's `vmti` field
is `Option<Vec<u8>>` pass-through bytes — consumers dispatch
themselves:

```rust
use tst_core::klv::{st0601, st0903};

# fn process(parent_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
let uas = st0601::decode(parent_bytes)?;
if let Some(vmti_bytes) = uas.vmti.as_deref() {
    let vmti = st0903::decode(vmti_bytes)?;
    println!("targets={}", vmti.targets.len());
    for t in &vmti.targets {
        println!("  id={} conf={:?}", t.target_id, t.confidence_level);
    }
}
# Ok(())
# }
```

This decoupling keeps each typed layer independent: an ST 0601
consumer doesn't pull VMTI parsing into its build, and ST 0903 spec
revisions don't ripple into ST 0601 parsing.

### Standalone-PID pattern

For VMTI on its own KLV PID (separate from any ST 0601 stream),
match the AU-cell payload bytes against `VMTI_LS_UL`:

```rust,no_run
use tst_core::klv::{length, st0903};

# fn handle(data: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
if data.starts_with(&st0903::VMTI_LS_UL) {
    let after_ul = &data[16..];
    let (declared_len, after_len) = length::read_ber(after_ul)?;
    let inner = &after_len[..declared_len];
    let vmti = st0903::decode(inner)?;
    // ...
}
# Ok(())
# }
```

The demuxer remains UL-agnostic; consumer-side dispatch keeps new
typed-set additions from creating coupling load on the demuxer.

### Lenient vs strict decode

- `decode` (lenient) — production ingest. Tolerates missing tags,
  malformed sub-records (preserved in `field_errors`), and unknown
  tags (preserved in `unknown` per ST 0107.5 §6 future-proof skip
  rule). Always returns `Ok` for parseable BER framing.
- `decode_strict` — compliance-grade ingest. Rejects missing required
  tags (Tag 4 `vmtiLsVersionNum` per ST 0903.5-99; Tag 6
  `numTargetsReported` per ST 0903.4-19), duplicate tags, malformed
  UTF-8, pack-level malformations (typed via
  `KlvDecodeError::St0903InvalidVTargetPack`). Still preserves
  unknown tags per ST 0107.5 §6.

`decode_strict` does NOT enforce conditional-required tags (Tags 1,
2, 11, 12, 13 are required only for specific carriage paths per
ST 0903.6-117/-119/-120). Consumers needing carriage-aware validation
post-validate after a successful decode.

### Encoding

`encode_to_vec(&ls)` produces wire-format bytes; `encoded_len(&ls)`
predicts the length without serializing. Round-trip is bit-identical
for all spec-conformant input (modulo IMAPB quantization).

### Universal Set form

Out of scope — see [`deferred-features.md`](/docs/project/deferred-features.md).

## Sync metadata AU cell carriage

Synchronous KLV in MPEG-TS uses a 5-byte `Metadata_AU_cell` header per
ITU-T H.222.0 V9 § 2.12.4.2 (Tables 2-155+2-156) to wrap each KLV
record. The wrapper is an MPEG-TS systems-layer construct, not a KLV
substrate concern — see [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) for
the carriage details. The muxer auto-wraps for
`KlvStreamType::SynchronousMetadata` streams; the demuxer surfaces the
parsed header fields on `MetadataKind::KlvSyncAuCell`.

The wrapper substrate lives at `mpegts::au_cell`
(`AuCellHeader`, `CellFragmentIndication`, `write_metadata_au_cell`,
`read_metadata_au_cell`) for callers that need to construct or parse
AU cells outside the mux/demux machinery.

### Caller-supplied `metadata_service_id`

`Muxer::push_klv*` and `MuxSender::send_klv*` take a
`metadata_service_id: u8` parameter that lands in the AU cell header
(per H.222.0 § 2.12.4.2 + ST 1402.2 App. B Table 2). The spec default
is `0x00` — pass that unless you have a specific reason to use a
non-zero service_id (typically: mirroring a `metadata_klva(service_id)`
PMT descriptor's `service_id` byte so receivers see consistent values
across the PMT advertisement and the wire AU cell).

The parameter is silently ignored on `KlvStreamType::PrivateData`
streams — those pass payload through verbatim with no AU cell wrap.
Sync streams (`SynchronousMetadata`) consume it.

### Multi-cell (fragmented) AUs

The demuxer detects fragmented AUs (CFI != Complete) and emits
`NonConformantIssue::MultiCellAu { pid, dropped_bytes }` as a
detect-only event. **Reassembly is not implemented** — the partial
payload is dropped. ST 0601 records fit well below the fragmentation
threshold; consumers don't see this in the wild yet, but the
observability hook lands so upstream senders that fragment surface
in telemetry.

## Wire-format details: PES `stream_id`

Per H.222.0 § 2.4.3.7 Table 2-22, the PES `stream_id` byte distinguishes
metadata stream class from private-data class:

| Stream class | `stream_type` | PES `stream_id` |
|---|---|---|
| Async / asynchronous KLV | `0x06` (PrivateData) | `0xBD` (private_stream_1) |
| Sync metadata KLV | `0x15` (SynchronousMetadata) | `0xFC` (metadata) |

`0xFC` is reserved for stream_type `0x15` only; async KLV rides
`private_stream_1` (matching ffmpeg + GStreamer convention). The
muxer selects the correct `stream_id` based on the configured
`KlvStreamType` automatically — callers don't supply it.

For the full demux ↔ mux stream-type mapping — including how demuxed
`StreamKind::KlvSync` / `KlvAsync` map to muxer `add_klv` parameters
in transmux workflows — see
[guides/mpegts-mux.md](/docs/guides/mpegts-mux.md#rebuilding-a-muxer-config-from-a-demuxed-program).

## KLVA registration descriptor auto-emit

The muxer emits a `registration_descriptor` (tag `0x05`,
`format_identifier = "KLVA"`) on every KLV stream's PMT entry,
regardless of `stream_type`. Both `PrivateData` (`0x06`) and
`SynchronousMetadata` (`0x15`) streams get the descriptor — receivers
gate KLV classification on the descriptor regardless of stream_type
(matching ffmpeg `mpegtsenc.c`). Sync KLV with `metadata_descriptor`
(tag `0x26`) doesn't *replace* KLVA; both descriptors coexist on the
PMT entry.

Suppression rule: caller-supplied `registration_descriptor` (any
`format_identifier`) on the KLV PID via `stream_descriptors_for_klv`
suppresses the auto-emit. A non-`KLVA` caller-supplied Registration
logs a `tracing::warn!` since receivers may not classify the stream
as KLV.

## Substrate walking

The lowest-level decode entry point is `klv::pack::Iter::local_set(body)`,
which iterates raw `RawField<'a> { tag, value }` triples without
committing to typed parsing. Useful for debugging, for custom local
sets, and for gateway translation that just shuffles tags between
producers.

```rust,no_run
use tst_core::klv::Iter;
use tst_core::klv::length::read_ber;

fn dump_fields(buf: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    // Skip the 16-byte UL, then read the outer BER length to find the body.
    let (_outer_len, body) = read_ber(&buf[16..])?;
    for r in Iter::local_set(body) {
        let f = r?;
        println!("tag={} len={} value={:02X?}", f.tag, f.value.len(), f.value);
    }
    Ok(())
}
```

`RawField<'a>` borrows `value` from the input buffer (zero-alloc).
Switch to `OwnedRawField` (via `OwnedRawField::from(field)`) when you
need to stash a parsed field beyond the lifetime of the input — `Iter`
is happy to feed either side.

## Bounded numeric encoding

ST 0601's ranged numeric tags use one of two related-but-distinct
schemes to map a bounded floating-point range into a fixed-width
integer on the wire. The typed layer applies the right scheme per tag
based on the spec — callers don't normally call either directly.

- `klv::st0601::mapping` (`LinearRange` — `U16Range` / `U32Range` /
  `S16Range` / `S32Range`). Uniform linear mapping with step
  `(max - min) / 2^bits`. This is what every ranged tag in the typed
  ST 0601 table currently uses (see
  [../crates/tst-core/src/klv/st0601/tags.rs](/crates/tst-core/src/klv/st0601/tags.rs)).
- `klv::imapb` (ST 1201.5 IMAPB). Power-of-two-aligned scale factor
  with INT_MIN reserved as INVALID — a different scheme; not
  interchangeable with `LinearRange`. Exposed as substrate so callers
  working with custom local sets or future ST 0601 tags that adopt
  IMAPB can call it directly: `klv::imapb::encode_imapb(&params, value,
  out)` / `klv::imapb::decode_imapb(&params, bytes)` with
  `ImapbParams { min, max, length }`. No tag in the current typed
  ST 0601 table uses IMAPB.

A concrete example: ST 0601 Tag 5 (Platform Heading) is a `LinearRange`
mapping of `0..360°` into 2 bytes unsigned. The step size is
`360 / 65535 ≈ 5.49e-3 °/step` — a heading of 217.456° quantizes to
one of two adjacent codepoints around that resolution and recovers
within `~5e-3°` on decode. Per-tag precision is documented in
[../crates/tst-core/src/klv/st0601/tags.rs](/crates/tst-core/src/klv/st0601/tags.rs);
[../examples/klv-metadata/klv_encode_minimal.rs](/examples/klv-metadata/klv_encode_minimal.rs)
walks four representative `LinearRange` tags with their step
calculations spelled out in comments.

## Working with real captures

Three steps to take a captured `.ts`, pull the KLV out, and decode it.

1. Extract KLV blobs from a `.ts` file:

   ```bash
   cargo run -p tst-examples --example extract_klv -- capture.ts /tmp/klv_out
   ```

   The second argument is a filename prefix; the example writes files
   into the input file's parent directory as `<prefix>_NNNN.klv`
   (`enumerate()`-indexed, 4-digit zero-padded — so the first blob is
   `_0000.klv`). Passing an absolute path as the prefix (e.g.
   `/tmp/klv_out`) works on Unix because `Path::join` replaces the base
   when the second argument is absolute — the files land in `/tmp/` as
   `/tmp/klv_out_0000.klv`, `/tmp/klv_out_0001.klv`, ...

2. Decode one blob through the strictness ladder:

   ```bash
   cargo run -p tst-examples --example klv_decode_file -- /tmp/klv_out_0000.klv
   ```

   The example tries `decode_strict_compliance` first and walks down to
   `decode_unchecked` if needed, reporting which rung accepted and what
   the previous rung rejected.

3. Inspect `record.field_errors` and `record.unknown.len()` for
   surprises. A non-empty `field_errors` means at least one tag had a
   malformed value the decoder skipped over; a non-empty `unknown`
   means the producer emitted tags this build's typed table doesn't
   cover (which is fine — the bytes still survive a re-encode through
   the unknown-pass-through path).

## What's deferred

Each item below maps to an entry in
[project/deferred-features.md](/docs/project/deferred-features.md).

- ST 0102 universal-set form — the LS form ships in `klv::st0102`;
  the parallel Universal Set encoding (16-byte UL per item) is not
  implemented. See [project/deferred-features.md](/docs/project/deferred-features.md).
- ST 0102 country-code validation — codes pass through as `String`;
  no validation against ISO 3166 / GENC / FIPS 10-4 / STANAG 1059 /
  CAPCO tables. See [project/deferred-features.md](/docs/project/deferred-features.md).
- Other typed sets (ST 0903 VMTI, ST 0806 RVT, ...) — the substrate
  supports them; per-tag tables are missing without a driving consumer.
  See [project/deferred-features.md](/docs/project/deferred-features.md).
- `serde` integration for typed records — wire format and JSON aren't
  isomorphic; needs an explicit decision on unknown-tag representation.
  See [project/deferred-features.md](/docs/project/deferred-features.md).
- `no_std` support — every shipping target has `std`; flipping to
  `no_std` means replacing `Vec` / `String` / `format!` with allocator
  equivalents. See [project/deferred-features.md](/docs/project/deferred-features.md).
- Streaming / chunked decode — today is buffer-in / buffer-out; a
  growable streaming decoder lands behind an explicit consumer ask.
  See [project/deferred-features.md](/docs/project/deferred-features.md).

## See also

- **Runnable example:** `cargo run -p tst-examples --example extract_klv` — [examples/klv-metadata/extract_klv.rs](/examples/klv-metadata/extract_klv.rs)
- **Runnable example:** `cargo run -p tst-examples --example klv_encode_minimal` — [examples/klv-metadata/klv_encode_minimal.rs](/examples/klv-metadata/klv_encode_minimal.rs)
- [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) — pushing KLV through the muxer.
- [guides/mpegts-demux.md](/docs/guides/mpegts-demux.md) — receiving KLV from a TS stream.
