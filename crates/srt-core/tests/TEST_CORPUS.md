# Test corpus shape catalog

Real-world MPEG-TS captures from gimbaled platforms exhibit more variation
than the v0 synthetic golden fixtures cover. This document catalogs the
structural shapes and content variants we've observed in the wild, so
fixtures dropped into the gitignored `tests/fixtures/local/` slot can be
named and asserted against shape rather than against any specific recording.

This file ships with the public repo. It is intentionally anonymized: no
aircraft identifiers, operator names, incident codes, sensor product names,
or geographic locations appear here. Each shape is keyed by its on-the-wire
structural signature, not by who recorded it.

## How fixtures are loaded

`tests/local_fixtures.rs` walks `tests/fixtures/local/` at test time:

- `*.klv` files are decoded directly through `srt_core::klv::st0601::decode`
  (with `decode_unchecked` as a checksum-relaxed fallback).
- `*.ts` files exercise the streaming demux path (planned: `mpegts::demux`).

The directory is gitignored. Tests pass silently with zero fixtures — the
corpus is for opt-in real-world coverage, not a CI gate.

Recommended fixture naming:

| Pattern | What it exercises |
|---|---|
| `shape-a-*.ts` | simple PMT (KLV + h264 ± audio) |
| `shape-b-*.ts` | Shotover-ARS-style PMT (KLV + h264 + private/sync metadata) |
| `shape-c-*.ts` | HEVC-pipeline PMT (alternate live-stream path) |
| `multi-record-pes-*.klv` | wrapper UL + ST 0601 LS in a single PES payload |
| `decode-unchecked-only-*.klv` | ST 0601 record with broken/missing checksum |
| `field-error-*.klv` | tags with truncated lengths or out-of-range values |
| `corrupt-pcr-*.ts` | structurally valid TS with damaged PCR/PTS timing |

Each fixture's filename should be self-descriptive and free of any
operationally identifying tokens (no callsigns, no incidents).

---

## Shape A — minimal HDMV+KLVA

Most common shape. Universal across multiple unrelated platforms.

```
PMT:
  PID 0x0?  stream_type 0x06  desc: tag=0x05 fmt="KLVA"
  PID 0x0?  stream_type 0x1B  desc: tag=0x05 fmt="HDMV"
 [PID 0x0?  stream_type 0x03  (audio MPEG-1, optional)]
 [PID 0x0?  stream_type 0x0F  (audio AAC, optional)]
```

Properties:

- Single KLV record per PES payload (the simple case).
- Video: H.264, profiles High and Main, both 1280×720 and 1920×1080 seen.
- Audio: zero, one, or two streams (AAC LC, MPEG-1, MP3).

Test goals against shape A:

- `decode()` succeeds on every PES payload extracted from the KLV PID.
- KLV PID is identified via the registration descriptor with format
  identifier `KLVA`; the demuxer must not assume a specific PID number.
- Decoder must handle PES payloads with PES-header optional fields
  (PTS-only, PTS+DTS, ESCR) — the `pes_header_data_length` byte is the
  authoritative skip count.

## Shape B — Shotover ARS encoder family

Distinctive multi-PID layout used by gimbal encoders that emit private
auxiliary data alongside the standard KLV stream. **Not unique to one
platform** — multiple platforms share this encoder family.

```
PMT:
  PID 0x0?  stream_type 0x06  desc: tag=0x05 fmt="KLVA", tag=0x26, tag=0x27, tag=0xFF
  PID 0x0?  stream_type 0x0F  desc: tag=0xFF
  PID 0x0?  stream_type 0x15  desc: tag=0x26, tag=0x27, tag=0xFF        ← private synchronous metadata
  PID 0x0?  stream_type 0x1B  desc: tag=0xFF
  PID 0x0?  stream_type 0xF0  desc: tag=0xFF                            ← user-private (1× to 4×)
  [PID 0x0?  stream_type 0xF1  desc: tag=0xFF]                          ← user-private (sometimes absent)
```

Properties:

- KLV PID's elementary-stream descriptor list contains four entries (KLVA
  + three private tags). The descriptor walker must iterate the full list,
  not stop at the first match.
- Some captures from this encoder family carry a KLV PID **without** an
  ST 0601 record — the elementary stream is present but its PES payloads
  contain only the private wrapper records and no ST 0601 LS to decode.
- The `0x15` PID's tag-set `{0x26, 0x27, 0xFF}` reliably identifies this
  encoder family even when the KLV PID is absent or missing the KLVA
  registration.

Test goals against shape B:

- Multi-descriptor parsing in PMT.
- KLV decode iteration (see "Multi-record PES" below) — captures from this
  encoder family are the primary source of wrapper-prefixed PES payloads.
- Demuxer should accept the unusual `stream_type 0xF0`/`0xF1` PIDs as
  opaque user-private (don't crash, don't claim them as anything specific).

## Shape C — HEVC live-stream pipeline

Alternate output path used by some platforms in parallel with their
ARS-recorded shape-B output. Distinct PMT, distinct codec.

```
PMT:
  PID 0x0?  stream_type 0x24                                            ← HEVC video
 [PID 0x0?  stream_type 0x15  (KLV-tagged private metadata, optional)]
 [PID 0x0?  stream_type 0x0F  (AAC audio, optional)]
```

Properties:

- HEVC video, Main profile, yuv420p, 1920×1080.
- KLV PID, when present, may **not** carry ST 0601 records (the PID
  exists but elementary-stream content is non-ST-0601).
- Some captures carry no audio.

Test goals against shape C:

- Demuxer recognizes stream_type 0x24 as HEVC video and does not require
  any specific descriptors.
- KLV demux is best-effort: presence of a KLV-tagged PID does not imply
  ST 0601 content.

## Wrapper-prefixed PES payloads (multi-record)

Some captures (most often shape B) emit PES payloads where the first
KLV record is **not** ST 0601. The byte layout is:

```
[ wrapper UL (16B) ][ BER length ][ wrapper body ][ ST 0601 UL (16B) ][ BER length ][ ST 0601 body ]
└── record 0 (skip) ──────────────┘└── record 1 (decode as ST 0601) ────────────────────────────────┘
```

Observed wrapper UL prefix:
`06 0E 2B 34 02 05 01 01  0E 01 01 03 11 00 00 00`

This is a SMPTE-registered set with a different `registry` byte (`0x05`)
than the ST 0601 LS family (`0x0B`). A naive decoder calling
`decode(&pes_payload)` reads the wrapper UL, attempts to parse the
9-byte body as ST 0601 fields, and fails with `Truncated`.

Required handling:

- Iterate KLV records over the PES payload (read 16-byte UL, BER length,
  body; advance; repeat until exhausted).
- For each record, gate on `UniversalLabel::is_st0601_family()` before
  attempting `decode`. Skip non-ST-0601 records silently.
- The ST 0601 record may appear at any offset within the PES, not just
  byte 0.

Test goals:

- A `multi-record-pes-*.klv` fixture (PES payload extracted to a `.klv`
  file) should:
  - Cause `decode(&fixture)` to **fail** (it tries to decode the wrapper).
  - Parse cleanly when iterated record-by-record and decoded only on
    ST 0601-family records.
- Document this in the public API of `klv::st0601` — either by adding a
  `decode_records_iter` helper or by being explicit that `decode`
  expects exactly one record at offset 0.

## ST 0601 field-presence reality

Across the corpus, every KLV-bearing capture decoded cleanly via
`decode()` or `decode_unchecked()`. However:

- **Tag 4 (`platform_tail_number`) and Tag 3 (`mission_id`) were absent
  in 100% of the corpus.** Decoder must treat both as fully optional;
  any unwrap or `expect` on these fields is a bug.
- **Tag 11 (`image_source_sensor`) values are freeform** — short tokens
  (`MWIR`, `EO`, `RGB`, `IR`), product strings with spaces and hyphens,
  and abbreviations. The field encoding is plain UTF-8; the decoder must
  preserve whitespace and casing verbatim.
- **Tag 2 (`timestamp_us`) is reliably present** when KLV is present.
  Use it as the authoritative clock when filename or mtime conflict.

Test goals:

- A no-tail / no-mission fixture round-trips through `decode → encode →
  decode` without spurious field generation.
- Sensor field with embedded spaces and hyphens preserves byte-exact.

## Decode-unchecked path

Some captures have records whose checksum (Tag 1) does not match the
ST 0601 running-sum-16 computation. `decode()` rejects them with
`ChecksumMismatch`; `decode_unchecked()` succeeds. The vast majority of
records in the corpus pass `decode()` cleanly — the unchecked path is a
minority but real fallback.

Test goals:

- A `decode-unchecked-only-*.klv` fixture exercises the fallback branch.
- `decode_strict()` (which also requires the ST 0601 family UL) is
  separately covered against synthetic fixtures.

## Container-damage edge case

At least one capture in the corpus has a structurally valid TS header
but severely truncated PCR/PTS timing — `ffprobe` reports a duration
under one second on a multi-hundred-megabyte file. The PMT and KLV PID
parse normally; the demuxer should not infinite-loop or panic on such
input.

Test goals (planned, alongside `mpegts::demux`):

- Bounded-byte demux: a `--max-bytes` ceiling on the streaming demuxer
  ensures pathological files don't read forever.
- Resync-on-lost-sync-byte: scan forward for the next 0x47 with
  packet-size alignment rather than aborting.

## Multi-PES decode at scale

KLV records are emitted at roughly 25–30 Hz, so a 30-second file carries
~750 PESs on the KLV PID, and a 30-minute file carries ~50 000. The
streaming probe binary in `testfiles/_tools/probe_ts/` (workspace-internal
scratch, not part of this repo) exercises this end-to-end and stops after
a configurable record count.

Test goals (for `mpegts::demux`):

- Demux of 100 000+ KLV records from a single fixture without quadratic
  memory growth or double-buffering bugs.
- Per-PES reassembly handles `payload_unit_start_indicator` semantics
  and adaptation-field skip correctly under typical CC drops.

## Coverage gaps (synthetic fixtures recommended)

Variants we have not seen in the real-world corpus — synthetic golden
fixtures should fill these:

- ST 0601 records with **all** typed tags populated (the existing
  synthetic golden in `tests/fixtures/st0601/` already covers this).
- Tag 4 + Tag 3 set to non-empty UTF-8 strings.
- KLV PES with adaptation-field bytes spanning the PSI section.
- KLV records >1 MiB requiring BER long-form length encoding (4-byte
  length).
- Asynchronous KLV streaming per ST 1909 §7 (multi-segment records
  spanning PESes).

These are listed for future test authors; not blocking for v0.
