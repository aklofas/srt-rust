# Python bindings (`tstrans`)

**Status (2026-05-23):** Phase 0+1 + Phase 2 + Phase 3 shipped on `main`.
The package exposes a working `Demuxer` + `DemuxEvent` hierarchy plus
`tstrans.io.parse_file(path)` for iterating events from a `.ts` file,
and full KLV typed decode for all 4 MISB sets (ST 0601 UAS Datalink,
ST 0102 Security, ST 0605 Precision Time Stamp, ST 0903 VMTI) under
`tstrans.klv`. `Muxer` (Phase 4) and per-frame codec parsing (Phase 5)
follow as separate plans.

## Install (when published)

```
pip install tstrans
```

Minimum Python is 3.10 (bumped from 3.9 mid-Phase-2 to enable PEP 604
union syntax and `match` statements without compat hacks).

## Quickstart

Phase 2 ships file inspection:

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

### KLV typed decode (Phase 3)

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

Phase 3 surfaces all 4 MISB typed sets (ST 0601 UAS Datalink, ST 0102
Security, ST 0605 Precision Time Stamp, ST 0903 VMTI) with the same
decoder semantics as the Rust crate: lenient mode tolerates broken
input and accumulates per-field errors on `.field_errors`; strict
mode raises `tstrans.exceptions.KlvError`. See `tstrans.klv` module
docstring for the full type listing.

### pandas + NumPy adapters (Phase 6, optional)

Install the optional extra to enable DataFrame adapters and zero-copy
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

- v1
  - Phase 0+1 — scaffolding + exception hierarchy. SHIPPED 2026-05-22.
  - Phase 2 — Demuxer wrap + `io.parse_file` + `io.probe`. SHIPPED 2026-05-22.
  - Phase 3 — KLV typed decode (`Klv0601`, `parse_klv_universal`). SHIPPED 2026-05-23.
  - Phase 4 — Muxer wrap + `Muxer.write_file`. UP NEXT.
  - Phase 5 — codec parsers (`H264Frame`, `H265Frame`, `Av1Frame`, ...).
  - Phase 6 — pandas / NumPy adapters.
  - Phase 7 — CI wheels + ratchets.
  - Phase 8 — PyPI publish.
- v2 — add live SRT (Sender / Receiver / MuxSender / DemuxReceiver shells).
- v3 — add RTP transport (MPEG-TS-over-RTP per RFC 2250).
