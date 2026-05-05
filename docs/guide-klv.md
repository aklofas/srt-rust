# KLV Codec Guide

## Introduction

This guide covers `srt_core::klv` — the KLV codec, bidirectional. Encoding
and decoding for the MISB ST 0601 UAS Datalink Local Set and the ST 0605
Precision Time Stamp Pack, layered on a generic SMPTE KLV substrate
(Universal Labels, BER lengths, IMAPB, checksum, pack iteration).
MPEG-TS sync-metadata AU cell carriage lives at `mpegts::au_cell` (per
ITU-T H.222.0 V9 § 2.12.4.2) — the muxer auto-wraps for
`KlvStreamType::SynchronousMetadata` streams.

What this module is *not*: a TS demuxer. Pulling KLV out of a captured
`.ts` file is done with FFmpeg / Bento4 / `cargo run --example extract_klv`
(see Section 11). TS demux in the Rust core is on the deferred list — see
[`mpegts::demux`](deferred-features.md) in `deferred-features.md`.

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
encoding of ST 0601 / ST 0605 / ST 1910. `klv::st0601::UasDatalinkLs` is
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
  accessor `version_byte()` returns the raw byte for legacy interop.
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
use srt_core::klv::UniversalLabel;

fn inspect(buf: &[u8; 16]) {
    let ul = UniversalLabel::new(*buf);
    println!("oid={:02X?} category={:02X} registry={:02X}",
             ul.oid(), ul.category(), ul.registry());
    println!("structure={:02X} byte_13={:02X}",
             ul.structure(), ul.version_byte());
    if ul.is_st0601_family() {
        println!("ST 0601 family — byte_13={:02X} (canonical 0x00)",
                 ul.version_byte());
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
use srt_core::klv::st0601::{
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
[../crates/srt-core/examples/klv_decode_file.rs](../crates/srt-core/examples/klv_decode_file.rs).

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
pass-through — see [compatibility.md](compatibility.md).

## Encoding

`encode_to_vec(&record) -> Result<Vec<u8>, KlvEncodeError>` is the happy
path. It allocates a buffer the right size, encodes, and returns the
bytes. The encoder auto-emits Tag 1 (the 16-bit running-sum checksum,
mandated last by ST 0601 §6.3) and Tag 65 (UAS LS Version Number,
mandated present by ST 0601.8 §10.3) when the caller didn't set them
explicitly — so a default-constructed record with a few typed fields
produces wire bytes that satisfy `decode_strict_compliance` out of the
box.

`encode_with(&record, &EncodeOptions, &mut [u8]) -> Result<usize, KlvEncodeError>`
is the in-place form for callers who want to control the output buffer
or override the Universal Label or version byte. `EncodeOptions` carries
two fields: `universal_label: UniversalLabel` (defaults to
`UniversalLabel::ST_0601_LS`) and `version: u8` (defaults to the version
byte of the default UL, `0x13` = ST 0601.19). Pre-size the output buffer
with `encoded_len(&record)` if you want to allocate exactly.

```rust,no_run
use srt_core::klv::st0601::{encode_to_vec, UasDatalinkLs};

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
[../crates/srt-core/examples/klv_encode_minimal.rs](../crates/srt-core/examples/klv_encode_minimal.rs).

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

## Sync metadata AU cell carriage

Synchronous KLV in MPEG-TS uses a 5-byte `Metadata_AU_cell` header per
ITU-T H.222.0 V9 § 2.12.4.2 (Tables 2-155+2-156) to wrap each KLV
record. The wrapper is an MPEG-TS systems-layer construct, not a KLV
substrate concern — see [guide-mpegts-mux.md](guide-mpegts-mux.md) for
the carriage details. The muxer auto-wraps for
`KlvStreamType::SynchronousMetadata` streams; the demuxer surfaces the
parsed header fields on `MetadataKind::KlvSyncAuCell`.

The wrapper substrate lives at `mpegts::au_cell`
(`AuCellHeader`, `CellFragmentIndication`, `write_metadata_au_cell`,
`read_metadata_au_cell`) for callers that need to construct or parse
AU cells outside the mux/demux machinery.

## Substrate walking

The lowest-level decode entry point is `klv::pack::Iter::local_set(body)`,
which iterates raw `RawField<'a> { tag, value }` triples without
committing to typed parsing. Useful for debugging, for custom local
sets, and for gateway translation that just shuffles tags between
producers.

```rust,no_run
use srt_core::klv::Iter;
use srt_core::klv::length::read_ber;

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
  [../crates/srt-core/src/klv/st0601/tags.rs](../crates/srt-core/src/klv/st0601/tags.rs)).
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
[../crates/srt-core/src/klv/st0601/tags.rs](../crates/srt-core/src/klv/st0601/tags.rs);
[../crates/srt-core/examples/klv_encode_minimal.rs](../crates/srt-core/examples/klv_encode_minimal.rs)
walks four representative `LinearRange` tags with their step
calculations spelled out in comments.

## Working with real captures

Three steps to take a captured `.ts`, pull the KLV out, and decode it.

1. Extract KLV blobs from a `.ts` file:

   ```bash
   cargo run --example extract_klv -- capture.ts /tmp/klv_out
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
   cargo run --example klv_decode_file -- /tmp/klv_out_0000.klv
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
[deferred-features.md](deferred-features.md).

- `klv::st0102` typed Security Local Set — Tag 48 currently passes
  through as `Option<Vec<u8>>`; no consumer reads the typed shape
  today. See [deferred-features.md](deferred-features.md).
- Other typed sets (ST 0903 VMTI, ST 0806 RVT, ...) — the substrate
  supports them; per-tag tables are missing without a driving consumer.
  See [deferred-features.md](deferred-features.md).
- `serde` integration for typed records — wire format and JSON aren't
  isomorphic; needs an explicit decision on unknown-tag representation.
  See [deferred-features.md](deferred-features.md).
- `no_std` support — every shipping target has `std`; flipping to
  `no_std` means replacing `Vec` / `String` / `format!` with allocator
  equivalents. See [deferred-features.md](deferred-features.md).
- Streaming / chunked decode — today is buffer-in / buffer-out; a
  growable streaming decoder lands behind an explicit consumer ask.
  See [deferred-features.md](deferred-features.md).
