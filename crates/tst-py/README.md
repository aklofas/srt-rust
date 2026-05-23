# tstrans

Python bindings (via PyO3) for the [ts-transformer](https://github.com/aklofas/ts-transformer) Rust workspace.

**Status:** alpha. v1 covers `.ts` file inspection and construction; live SRT and RTP transports ship in v2 / v3.

## Install

```
pip install tstrans
```

## Quickstart

**Status (2026-05-22):** Phase 0+1 (crate scaffolding) shipped. `tstrans` is importable but does not yet wrap any `tst-core` types — `parse_file` / `Muxer` / KLV / codec parsers land in Phase 2-5. See the project [ROADMAP](https://github.com/aklofas/ts-transformer/blob/main/ROADMAP.md) for the current v1 milestones.

Once v1 ships:

```python
from tstrans.io import parse_file

for event in parse_file("capture.ts"):
    print(event)
```

See [docs/guide-python.md](https://github.com/aklofas/ts-transformer/blob/main/docs/guide-python.md) for the full guide (added in Phase 7).
