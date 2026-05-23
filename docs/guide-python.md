# Python bindings (`tstrans`)

**Status (2026-05-22):** Phase 0+1 (crate scaffolding + module skeleton) shipped.
The package is importable but does not yet wrap any `tst-core` types.
v1 (file inspection + construction) is in progress.

## Install (when published)

```
pip install tstrans
```

## Quickstart (preview — landing in Phase 2+ plans)

```python
from tstrans.io import parse_file

for event in parse_file("capture.ts"):
    print(event)
```

## Design

See [docs/specs/2026-05-22-tst-py-design.md](../../docs/specs/2026-05-22-tst-py-design.md)
(at parent-level project tree, outside the published repo).

## Roadmap

- v1 — inspect + build `.ts` files. In progress.
- v2 — add live SRT (Sender / Receiver / MuxSender / DemuxReceiver shells).
- v3 — add RTP transport (MPEG-TS-over-RTP per RFC 2250).
