# Python bindings (`tstrans`)

> **Status (Phase 6 shipped, 2026-05-23):** `tstrans` is feature-complete
> for v1: file inspection + construction (`Demuxer` / `Muxer` /
> `MuxerFileSink`), typed KLV decode + encode for ST 0601 / ST 0102 /
> ST 0605 / ST 0903 (with `VTargetPack`), codec parsers for H.264 /
> H.265 / H.266 / AV1 / AAC / MPEG-2 audio, and optional pandas
> DataFrame adapters + NumPy snapshot views via
> `pip install tstrans[pandas]`. ~582 pytest tests. Live SRT (v2) and
> RTP (v3) transports remain on the roadmap. Minimum Python 3.10.

## Install (when published)

```
pip install tstrans
```

Minimum Python is 3.10 (bumped from 3.9 mid-Phase-2 to enable PEP 604
union syntax and `match` statements without compat hacks).

## Quickstart

Inspect a `.ts` file:

```python
from tstrans.io import parse_file, probe
from tstrans.mpegts import DemuxEvent

# Quick summary
r = probe("capture.ts")
print(r.video_codecs, r.audio_codecs, r.has_klv)

# Full event stream
for event in parse_file("capture.ts"):
    match event:
        case DemuxEvent.ProgramMap(programs=pms):
            print(f"PSI: {len(pms)} programs")
        case DemuxEvent.Video(pts=p, codec=c, payload=b):
            print(f"Video {c.name} pts={p.ms}ms len={len(b)}")
        case DemuxEvent.Klv(pts=p, payload=b):
            print(f"KLV pts={p.ms}ms len={len(b)} (use tstrans.klv to decode)")
```

### KLV typed decode

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

All 4 MISB typed sets (ST 0601 UAS Datalink, ST 0102 Security,
ST 0605 Precision Time Stamp, ST 0903 VMTI) decode with the same
semantics as the Rust crate: lenient mode tolerates broken input and
accumulates per-field errors on `.field_errors`; strict mode raises
`tstrans.exceptions.KlvError`. Symmetric encoders (`encode_*_lenient`
/ `encode_*_strict`) round-trip parsed records back to wire bytes.
See the `tstrans.klv` module docstring for the full type listing.

### pandas + NumPy adapters (optional)

Install the optional extra to enable DataFrame adapters and snapshot
NumPy views over NAL / OBU / parameter-set payloads:

```bash
pip install 'tstrans[pandas]'
```

See [guide-python-pandas.md](guide-python-pandas.md) for the full
integration guide.

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
