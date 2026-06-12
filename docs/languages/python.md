# Python bindings (`tstrans`)

> **Who this is for:** You write Python and want to inspect, build, or
> process MPEG-TS + KLV files. Note: live SRT transport is not available
> in v1 — use the Rust core or C bindings for live streaming.

> **You will learn:**
> - How to install `tstrans` (with or without the pandas extra)
> - How to read a `.ts` file and inspect typed `DemuxEvent` items in ~5 lines
> - How to build a `.ts` file by pushing video + KLV through the `Muxer`
> - How to encode and decode all 4 MISB KLV sets (ST 0601 / 0102 / 0605 / 0903)
> - How to drive bulk KLV → pandas DataFrame ETL with the optional `[pandas]` extra
> - The Python-specific gotchas: GIL release, dataclass strictness, optional extras
> - How this binding differs from the Rust core (file-only, no live SRT)

## Install

```bash
pip install tstrans
```

Optional extras:

```bash
pip install 'tstrans[pandas]'   # pandas DataFrame adapters + NumPy snapshot views
```

**Minimum Python is 3.10** (bumped from 3.9 mid-Phase-2 to enable PEP 604
union syntax and `match` statements without compat hacks).

The compiled extension is imported as `tstrans._native`. Public API lives
on `tstrans` and its topic submodules — `tstrans.io`, `tstrans.mpegts`,
`tstrans.klv`, `tstrans.codec`, and the optional `tstrans.pandas`. Don't
reach into `tstrans._native` directly; it may reorganize between versions.

> **Status (Phase 6 shipped, 2026-05-23):** `tstrans` is feature-complete
> for v1: file inspection + construction (`Demuxer` / `Muxer` /
> `MuxerFileSink`), typed KLV decode + encode for ST 0601 / ST 0102 /
> ST 0605 / ST 0903 (with `VTargetPack`), codec parsers for H.264 /
> H.265 / H.266 / AV1 / AAC / MPEG-2 audio, and optional pandas
> DataFrame adapters + NumPy snapshot views via
> `pip install tstrans[pandas]`. ~582 pytest tests. Live SRT (v2) and
> RTP (v3) transports remain on the roadmap.

## Hello world

Read a `.ts` file and print the type of each event in five lines:

```python
import tstrans

for event in tstrans.io.parse_file("capture.ts"):
    print(type(event).__name__)
```

## First send

Build a single-program H.264 `.ts` file by pushing one access unit through
the `Muxer`:

```python
from tstrans.mpegts import (
    Muxer,
    MuxerConfigBuilder,
    MuxerProgramConfigBuilder,
    Pts90khz,
    VideoCodec,
)

prog = (
    MuxerProgramConfigBuilder(program_number=1, pmt_pid=0x100)
    .add_video(0x101, VideoCodec.H264)
    .add_klv(0x102)
    .build()
)
cfg = MuxerConfigBuilder().add_program(prog).build()
m = Muxer(cfg)

with m.write_file("out.ts") as proxy:
    proxy.push_video(nal_bytes, pts=Pts90khz.from_raw(900_000))
    proxy.push_klv(klv_bytes, pts=Pts90khz.from_raw(900_000))
```

`MuxerFileSink` (the object returned by `write_file`) is a context
manager — `__exit__` flushes and finalizes the file; no explicit
`close()` ceremony is needed. Note the pushes go through `proxy` (the
object the `with` statement yields), not through `m` — only proxy
pushes drain to the file as they go.

## First receive

Demux a file and dispatch on typed events with a `match` statement:

```python
from tstrans.io import parse_file
from tstrans.mpegts import DemuxEvent

for event in parse_file("capture.ts"):
    match event:
        case DemuxEvent.ProgramMap(programs=pms):
            print(f"PSI: {len(pms)} programs")
        case DemuxEvent.Video(pts=p, codec=c, raw=b) as ev:
            # raw-first: `raw` is the exact encoded access unit. Splitting
            # it into typed NAL/OBU units is opt-in via `ev.parse()`.
            print(f"Video {c.name} pts={p.ms}ms len={len(b)} units={len(ev.parse())}")
        case DemuxEvent.Klv(pts=p, payload=b):
            print(f"KLV pts={p.ms}ms len={len(b)} (use tstrans.klv to decode)")
```

For a quick summary without iterating every event, use `probe`:

```python
from tstrans.io import probe

r = probe("capture.ts")
print(r.video_codecs, r.audio_codecs, r.has_klv)
```

To pull typed KLV records directly:

```python
from tstrans.io import extract_klv
from tstrans.klv import UasDatalinkLs, parse_klv_universal

# Iterate typed KLV records from a .ts file
for pts, record in extract_klv("capture.ts", parsed=True, with_pts=True):
    if isinstance(record, UasDatalinkLs):
        pos = record.sensor_position()
        if pos is not None:
            print(
                f"{pts.ms}ms platform={record.platform_designation} "
                f"@ {pos.lat_deg:.5f},{pos.lon_deg:.5f} alt={pos.alt_m:.1f}m"
            )

# Or dispatch a single record by UL
record = parse_klv_universal(raw_klv_bytes)
# record is UasDatalinkLs | SecurityLs | PrecisionTimeStampPack | VmtiLs | None
```

For the common ST-0601-only case there is a dedicated iterator that
also carries each record's file-order KLV index (it counts every KLV
event, so indices line up with a later re-mux pass over the same
file), and the typed sets support copy-update via `with_()` — they
are frozen dataclasses, so attribute assignment raises
`FrozenInstanceError`:

```python
from tstrans.io import iter_uas_datalink

for pts, klv_index, record in iter_uas_datalink("capture.ts"):
    corrected = record.with_(sensor_lat_deg=33.5)  # frozen → copy-update
```

KLV demux events decode in place too: `ev.parse()` on a
`DemuxEvent.Klv` dispatches by universal label — the KLV counterpart
of the raw-first `Video.parse()` / `Audio.parse()`.

All 4 MISB typed sets (ST 0601 UAS Datalink, ST 0102 Security,
ST 0605 Precision Time Stamp, ST 0903 VMTI) decode with the same
semantics as the Rust crate: lenient mode tolerates broken input and
accumulates per-field errors on `.field_errors`; strict mode raises
`tstrans.exceptions.KlvError`. Symmetric encoders (`encode_*_lenient`
/ `encode_*_strict`) round-trip parsed records back to wire bytes.
See the `tstrans.klv` module docstring for the full type listing.

## Transmux: edit metadata, copy everything else

`tstrans.io.transmux` bridges a demuxer and a muxer: iterate the source's
events and write back the ones to keep. Video/audio are copied
byte-for-byte via their raw encoded AUs; KLV can be substituted — pair
with `tstrans.klv.patch_uas_datalink` for byte-faithful tag edits. The
output muxer is built lazily from the first `ProgramMap`, so the
source's program topology (PIDs, codecs, program number) is reproduced.

```python
import tstrans.io as tio
from tstrans import klv
from tstrans.mpegts import DemuxEvent

with tio.transmux("in.ts", "out.ts", atomic=True) as tx:
    for ev in tx:
        if isinstance(ev, DemuxEvent.Klv):
            patched = klv.patch_uas_datalink(
                ev.payload, {"frame_center_lat_deg": 37.7749}
            )
            tx.write_klv(ev, patched)
        else:
            tx.write(ev)  # video/audio copied byte-for-byte
```

Strict by default: streams the muxer cannot represent (DVB
subtitling/teletext) raise `MuxError` naming the offenders.
Private/application data streams (unknown stream types) pass through
byte-faithfully: `MuxerConfig.from_program_map` reproduces their PMT
entry (raw stream_type byte + descriptor loop verbatim) and each
`DemuxEvent.UnknownSample` payload is re-emitted as-is via
`push_data_to`. Re-muxed data streams always carry PTS and the
demuxer substitutes 0 for a PTS-less source PES, so a source sample
with no PTS re-emerges with a literal PTS of 0.
Pass kinds in `drop=` (e.g. `drop=(StreamKindTag.UNKNOWN,)`) to
exclude streams instead; their events are then skipped by `write`. v1
supports single-program sources (a second program raises
`ValueError`).
`atomic=True` writes through a same-directory `*.partial` temp file and
`os.replace`s it into place only on clean exit, so no partial output can
appear at the destination.

## Language-specific gotchas

- **GIL released in `push_*` methods.** Long-running CPU work (large NAL
  parses, big KLV blobs) doesn't block other Python threads. The
  `add_subtitle()` and `push_subtitle*()` methods also release the GIL
  (added in plan #96 Wave C).
- **Subtitle config dataclasses reject `bool`-as-`int`.** PyO3 strictness
  means `True` is not silently coerced to integer `1` for fields that
  expect an integer codec selector. Same with `bytearray` vs `bytes`.
  (Came from plan #96 validation pass.)
- **`MuxerFileSink` is a context manager — push on the proxy it
  yields.** Use `with m.write_file("out.ts") as proxy: ...` and route
  every `push_*` through `proxy`. Only proxy pushes drain to the file;
  pushing on the original Muxer (`m.push_video(...)`) inside the block
  bypasses the per-push drain and raises `MuxError(BACKPRESSURE)` once
  `buffer_packets` (default 10 000) accumulate — a footgun that only
  fires in long push loops. The `__exit__` flushes + finalizes the
  file. No explicit `close()` ceremony is needed (and a double-close on
  the underlying handle would panic).
- **Video / Audio events are raw-first; parsing is opt-in.** A
  `DemuxEvent.Video` / `DemuxEvent.Audio` carries `.raw` (the exact encoded
  bytes). Call `ev.parse()` to get typed units: for H.264 / H.265 / H.266
  video it's `list[NalUnit]`; for AV1 it's `list[Obu]`; for AAC ADTS it's
  `list[AdtsFrame]`; for MPEG-2 Audio it's `list[Mpeg2AudioFrame]`. For
  subtitles + AAC-LATM there's no typed parser — use `.raw` directly. The
  free functions `tstrans.codec.split_units(raw, codec)` and
  `tstrans.codec.parse_audio(raw, codec)` do the same split and additionally
  return the conformance-issue list.
- **abi3 build limitation.** `bytes`-like extraction uses a two-path
  approach (one for true `bytes`, one for `memoryview` / `bytearray`)
  because PyO3's abi3 doesn't expose a unified buffer protocol. The
  Python API is uniform — you can pass `bytes`, `bytearray`, or a
  `memoryview` and it works.
- **`tstrans._native` is private.** Use `tstrans.X` (or `tstrans.mpegts.X`
  / `tstrans.klv.X` / ...) — never `tstrans._native.X`. The `_native`
  submodule may reorganize between versions.

### Pandas + NumPy adapters

Optional pandas DataFrame adapters and NumPy snapshot views (one
Rust-to-Python `bytes` copy per access; see [Snapshot vs zero-copy](#snapshot-vs-zero-copy)
below) for the `tstrans` Python package. Requires the `[pandas]` extra:

```bash
pip install 'tstrans[pandas]'
```

Without the extra, `tstrans` works as documented in the core modules above. Calling any pandas adapter or any
NumPy `.payload_np` / `.raw_rbsp_np` / `.raw_np` accessor without the
extra raises:

```
ImportError: tstrans pandas adapters require: pip install 'tstrans[pandas]'
```

#### Quick start

```python
import tstrans.io
import tstrans.pandas

# Parse a .ts file into events
events = list(tstrans.io.parse_file("capture.ts"))

# Convert to DataFrame for analysis
df = tstrans.pandas.events_to_dataframe(events)
print(df.kind.value_counts())
#  Sample                  1234
#  Metadata                  56
#  ProgramMap                12
```

#### DataFrame adapters

##### KLV records — `klv_to_dataframe`

```python
from tstrans.io import extract_klv

records = list(extract_klv("capture.ts", parsed=True))
df = tstrans.pandas.klv_to_dataframe(records)
df.head()
```

`klv_to_dataframe` is polymorphic — it dispatches on the record type
and produces a per-set schema. Input must be homogeneous (one set type
per call); mixed input raises `TypeError`. Supported types: `UasDatalinkLs`
(ST 0601), `SecurityLs` (ST 0102), `PrecisionTimeStampPack` (ST 0605),
`VmtiLs` (ST 0903).

KLV DataFrames are indexed by `pd.DatetimeIndex` (with `tz="UTC"`,
named `pts`) derived from the per-record timestamp where present:
ST 0601 / ST 0903 use the `timestamp_us` field (microseconds since
the 1970 UTC epoch), ST 0605 uses its own precision timestamp. If a
record lacks a timestamp the row's index entry is `pd.NaT`; if NO
record in the batch has one the DataFrame falls back to
`pd.RangeIndex`.

**Column shape.** ST 0601 (UasDatalinkLs) flattens to its full set of
~50 scalar fields — fields like `frame_center_lat_deg`,
`frame_center_lon_deg`, `frame_center_elev_m`, `sensor_lat_deg`,
`sensor_lon_deg`, `sensor_alt_m`, `platform_heading_deg`,
`platform_pitch_deg`, `platform_roll_deg` are direct top-level columns
(no dotted composite namespacing). Enum-valued fields collapse to their
variant name string (e.g. `"FullyEncrypted"`). Per-field parse errors
(Phase 3 `KlvFieldError`) collapse to a single string `field_errors`
column using a `|` joiner with the per-error format
`tag<N>:<kind>:<message>` — the `|` (not `,`) joiner keeps the column
parseable even when an error `message` contains commas.

**ST 0903 (VmtiLs) supports two modes:**

- `mode="summary"` (default): one row per VMTI record, with a
  `num_targets` column counting `VTargetPack` entries. Indexed by
  `pd.DatetimeIndex` of record timestamps.
- `mode="targets"`: one row per `VTargetPack`, indexed by
  `pd.MultiIndex` with levels `[pts, target_id]`.

```python
# Aggregate targets across the full capture
targets = tstrans.pandas.klv_to_dataframe(vmti_records, mode="targets")
```

##### DemuxEvents — `events_to_dataframe`

```python
df = tstrans.pandas.events_to_dataframe(events)
```

Union schema across all event kinds. Video / Audio / Subtitle events
collapse to `kind="Sample"`; KLV events collapse to `kind="Metadata"`;
ProgramMap / NonConformant / Discontinuity / ReconnectDiscontinuity
keep their own labels. (`Pat` is folded into `ProgramMap` by the
demuxer; it never appears as a separate kind.)

| Column | Type | Description |
|---|---|---|
| kind | str | `Sample` / `Metadata` / `ProgramMap` / `NonConformant` / `Discontinuity` / `ReconnectDiscontinuity` |
| pts_raw | u64 | `Pts90khz.raw` ticks |
| pts_ms | float | `Pts90khz.ms` (PTS in milliseconds) |
| dts_ms | float | DTS in ms (Sample events that carry it; otherwise NaN) |
| pid | u16 | Source PID (NaN for global events) |
| stream_type | str | `StreamKind` variant name (`Video` / `Audio` / `Klv` / `Subtitle`) |
| codec | str | Codec tag (`H264` / `H265` / `H266` / `Av1` / `Aac` / `Mpeg2Audio` / `WebVtt` / ...) |
| payload_len | int | byte length of the event payload — `len(raw)` for video / audio rows, `len(payload)` for KLV / subtitle rows |
| nal_count | int | Video-only — the per-AU unit count, obtained by running the opt-in `event.parse()` on each `_VideoEvent` row (NAL units, or OBUs for AV1). NaN on audio rows and on non-Sample rows |
| random_access | bool | TS adaptation-field RAI bit (video samples) |
| has_codec_parse_error | bool | Vestigial column, always `None` under the raw-first surface — the eager `codec_parse_error` field was dropped; conformance issues now surface via `event.parse(strict=True)` / `tstrans.codec.split_units`. Kept for schema stability. |
| issue | str | `NonConformant` event's issue text |
| issue_kind | str | `NonConformant` event's `.kind` enum variant name |

Payloads themselves stay on the original event objects — they're not
materialised in the DataFrame.

##### NAL / OBU lists — `nals_to_dataframe` / `obus_to_dataframe`

```python
# Extract NALs from a single video Sample (opt-in split via `.parse()`)
sample = next(e for e in events if type(e).__name__ == "_VideoEvent")
df = tstrans.pandas.nals_to_dataframe(sample.parse(), pts=sample.pts.ms)
df.nal_type_name.value_counts()
```

NAL type names are decoded via H.264 §Table 7-1 / H.265 §Table 7-1 /
H.266 V4 §Table 5 lookup keyed on `nal.kind`. Unknown types fall back
to `unknown_{n}`.

Columns: `kind`, `nal_type`, `nal_type_name`, `ref_idc` (H.264 only;
NaN elsewhere), `layer_id` (H.265/H.266 only; NaN on H.264),
`temporal_id_plus1`, `payload_len`, and `pts_ms` if the optional `pts`
argument was supplied.

```python
# AV1 sample (`.parse()` returns the OBU list)
df = tstrans.pandas.obus_to_dataframe(sample.parse(), pts=sample.pts.ms)
```

OBU schema: `obu_type`, `obu_type_name`, `temporal_id`, `spatial_id`
(both from the optional OBU extension; NaN when absent), `payload_len`,
and `pts_ms` if supplied.

##### Audio frames — `audio_frames_to_dataframe`

```python
from tstrans.codec import parse_aac_frames

frames = parse_aac_frames(buf)
df = tstrans.pandas.audio_frames_to_dataframe(frames)
df.plot(x="byte_offset", y="frame_length_bytes")
```

Polymorphic — detects `AdtsFrame` vs `Mpeg2AudioFrame` from the first
element. Mixed-type input raises `TypeError`. Enum-valued fields
collapse to their bare variant name (e.g. `"LC"`, not `"AacProfile.LC"`;
`"III"` for MPEG-2 Audio Layer III; `"JOINT_STEREO"` for the channel
mode). Struct-valued `AacChannelLayout` is kept as its `repr`.

`byte_offset` is the running cumulative offset of each parsed frame
inside the input buffer, computed by summing `frame_length_bytes`
from zero. For inputs produced by `parse_*_frames_with_resync`, this
does NOT account for skipped (garbage) bytes between recovered frames
— if you need absolute offsets across a resync boundary, pre-compute
them from the resync output itself.

#### NumPy snapshot views

Every byte-bearing class (NalUnit, Obu, AdtsFrame, Mpeg2AudioFrame, all
H.264/H.265/H.266 SPS/PPS/VPS/SliceHeaderLight, AV1 sequence/frame
headers) carries a `.payload_np` / `.raw_rbsp_np` / `.raw_np` accessor
that returns a `numpy.ndarray(dtype=uint8)` snapshot — each access
copies from Rust-owned storage into a fresh Python `bytes`, which
NumPy then views without further copy:

```python
import numpy as np
from tstrans.codec import parse_h264_sps

sps = parse_h264_sps(rbsp_bytes)
arr = sps.raw_rbsp_np   # snapshot np.ndarray(dtype=np.uint8)
```

Mapping:

- `.payload_np` — `NalUnit`, `Obu`, `AdtsFrame`, `Mpeg2AudioFrame`
- `.raw_rbsp_np` — H.264 / H.265 / H.266 `Sps` / `Pps` / `Vps` /
  `SliceHeaderLight`
- `.raw_np` — `Av1SequenceHeader`, `Av1FrameHeaderLight` (the field is
  named `raw`, not `raw_rbsp`)

These accessors are **read-only** views — `np.frombuffer` sets
`writeable=False` on Python `bytes`. Mutating attempts raise
`ValueError: assignment destination is read-only` by design.

##### Snapshot vs zero-copy

Each `.payload_np` / `.raw_rbsp_np` / `.raw_np` access materializes a
fresh Python `bytes` from Rust-owned storage (one copy), then NumPy
views that bytes object with no further copy. Per-access cost is
`O(payload_length)`; the view itself is a true zero-copy view over the
bytes object, but the bytes object is freshly allocated each time. For
repeated access on the same frame/NAL, cache the result manually:

```python
arr = nal.payload_np  # one copy from Rust
# use `arr` repeatedly — no further copy
```

A future plan may implement the Python buffer protocol directly on the
Rust types, eliminating the bytes copy. This is non-trivial because
each of the ~15 PyClass types would need `__getbuffer__` /
`__releasebuffer__` magic methods over stable Rust-owned storage.
Tracked as a v2 optimization.

For users who don't want the `.payload_np` indirection, the snapshot
is one line of stdlib NumPy:

```python
import numpy as np
arr = np.frombuffer(nal.payload, dtype=np.uint8)
```

Both forms are equivalent.

#### Common recipes

##### Plot platform altitude over time

```python
df = tstrans.pandas.klv_to_dataframe(uas_records)
df["sensor_alt_m"].plot()
# Or, if you want the framed-scene centre instead of the sensor itself:
df["frame_center_elev_m"].plot()
```

##### Filter Sample events by codec

```python
df = tstrans.pandas.events_to_dataframe(events)
h264_samples = df[(df.kind == "Sample") & (df.codec == "H264")]
```

##### NAL type histogram across an entire capture

```python
all_nals = []
for ev in events:
    if type(ev).__name__ == "_VideoEvent":
        all_nals.extend(ev.parse())  # opt-in split of each AU into NAL units
df = tstrans.pandas.nals_to_dataframe(all_nals)
df.nal_type_name.value_counts().plot.bar()
```

##### Audio frame-length over byte offset

```python
frames = list(parse_aac_frames(buf))
df = tstrans.pandas.audio_frames_to_dataframe(frames)
df.set_index("byte_offset")["frame_length_bytes"].plot()
```

#### Troubleshooting

**`TypeError: klv_to_dataframe requires homogeneous record types`** — your
input mixes ST sets (e.g. `UasDatalinkLs` + `SecurityLs`). Split into
per-set lists:

```python
from tstrans.klv import UasDatalinkLs, SecurityLs
uas = [r for r in records if isinstance(r, UasDatalinkLs)]
sec = [r for r in records if isinstance(r, SecurityLs)]
df_uas = tstrans.pandas.klv_to_dataframe(uas)
df_sec = tstrans.pandas.klv_to_dataframe(sec)
```

**KLV DataFrame falls back to `RangeIndex` instead of `DatetimeIndex`** —
none of your records had a populated timestamp. Common with legacy
ST 0102 SecurityLs (no internal timestamp) or partial captures whose
records pre-date the precision-timestamp tag.

**`field_errors` looks empty / non-empty unexpectedly** — lenient KLV
decode (the default) keeps a per-record `field_errors` list of
`KlvFieldError` entries for tags that failed to parse. The DataFrame
collapses these to a `|`-joined string. Empty `field_errors` becomes
the empty string `""`, not `NaN`. If you need a boolean instead, use
`df.field_errors.astype(bool)`.

**`nal_count` is `NaN` on audio rows** — by design.
`audio_frames_to_dataframe` is the audio-frame adapter; `nal_count` is
populated only on video Sample rows. See the column table above.

**`byte_offset` doesn't match the absolute byte position I expected** —
the cumulative offset is a running sum of `frame_length_bytes` starting
at zero, so it represents the offset within the contiguous-frame slice
the adapter saw. For `*_with_resync` flows, gaps caused by skipped
garbage bytes between frames are NOT reflected. Use the resync API
output directly when absolute byte offsets matter.

## Where this binding differs from the Rust core

- **File I/O only in v1.** `tstrans` v1 ships file inspection
  (`tstrans.io.parse_file`) and offline `.ts` construction (`Muxer` +
  `MuxerFileSink`). Live SRT transport lands in v2 (the Rust core is
  ready; only the Python wrap is the work). RTP transport lands in v3.
- **No raw `RawSender` / `RawReceiver`.** Only the composed `Muxer` /
  `Demuxer` surface is wrapped. (Live SRT in v2 will add `Sender` /
  `Receiver` wraps.)
- **Subtitle Mux API is dataclass-driven.** Rust uses struct-variant
  enums for subtitle codec config; Python wraps each variant as a
  separate dataclass (`DvbSubtitlingConfig`, `DvbTeletextConfig`,
  `Cea708StandaloneConfig`, `WebVttInTsConfig`).
- **`add_subtitle()` and `push_subtitle*()` release the GIL.** Added in
  plan #96 Wave C.
- **No bindings for low-level `mpegts::demux::low_level::*`.** The Rust
  core exposes extension points there; the Python wrap omits them
  (would need PyO3 wrapping of trait objects).
- **Optional `[pandas]` extra is opt-in.** The `tstrans.pandas`
  submodule is only available if `pip install tstrans[pandas]` was
  used. Importing without the extra raises `ImportError` with a clear
  message.

(For pandas + NumPy specifics, see the
[Pandas + NumPy adapters](#pandas--numpy-adapters) sub-section under
"Language-specific gotchas" above.)

## Design

See [docs/specs/2026-05-22-tst-py-design.md](../../docs/specs/2026-05-22-tst-py-design.md)
(at parent-level project tree, outside the published repo).

## Roadmap

- v1 — SHIPPED 2026-05-23 (Phases 0-6).
  - Phase 0+1 — scaffolding + exception hierarchy. SHIPPED 2026-05-22.
  - Phase 2 — Demuxer wrap + `io.parse_file` + `io.probe`. SHIPPED 2026-05-22.
  - Phase 3 — KLV typed decode (`UasDatalinkLs`, `parse_klv_universal`). SHIPPED 2026-05-23.
  - Phase 4 — Muxer wrap + `Muxer.write_file` + symmetric KLV encoders. SHIPPED 2026-05-23.
  - Phase 5 — codec parsers (`NalUnit`, `Obu`, `AdtsFrame`, `Mpeg2AudioFrame`). SHIPPED 2026-05-23.
  - Phase 6 — pandas / NumPy adapters via `[pandas]` extra. SHIPPED 2026-05-23.
  - Phase 7 — CI wheels + PyPI publish. UP NEXT.
- v2 — add live SRT (Sender / Receiver / MuxSender / DemuxReceiver shells).
- v3 — add RTP transport (MPEG-TS-over-RTP per RFC 2250).
