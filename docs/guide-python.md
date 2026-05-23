# Python bindings (`tstrans`)

**Status (2026-05-22):** Phase 0+1 + Phase 2 shipped on `main`. The
package exposes a working `Demuxer` + `DemuxEvent` hierarchy plus
`tstrans.io.parse_file(path)` for iterating events from a `.ts` file.
KLV typed decode (Phase 3), `Muxer` (Phase 4), and per-frame codec
parsing (Phase 5) follow as separate plans.

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
            print(f"KLV pts={p.ms}ms len={len(b)} (use tstrans.klv to decode in Phase 3)")
```

## Design

See [docs/specs/2026-05-22-tst-py-design.md](../../docs/specs/2026-05-22-tst-py-design.md)
(at parent-level project tree, outside the published repo).

## Roadmap

- v1
  - Phase 0+1 — scaffolding + exception hierarchy. SHIPPED 2026-05-22.
  - Phase 2 — Demuxer wrap + `io.parse_file` + `io.probe`. SHIPPED 2026-05-22.
  - Phase 3 — KLV typed decode (`Klv0601`, `parse_klv_universal`). UP NEXT.
  - Phase 4 — Muxer wrap + `Muxer.write_file`.
  - Phase 5 — codec parsers (`H264Frame`, `H265Frame`, `Av1Frame`, ...).
  - Phase 6 — pandas / NumPy adapters.
  - Phase 7 — CI wheels + ratchets.
  - Phase 8 — PyPI publish.
- v2 — add live SRT (Sender / Receiver / MuxSender / DemuxReceiver shells).
- v3 — add RTP transport (MPEG-TS-over-RTP per RFC 2250).
