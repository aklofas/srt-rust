# tstrans

Python bindings (via PyO3) for the [ts-transformer](https://github.com/aklofas/ts-transformer) Rust workspace.

**Status:** alpha. v1 covers `.ts` file inspection and construction; live SRT and RTP transports ship in v2 / v3.

## Install

```
pip install tstrans
```

## Quickstart

```python
from tstrans.io import parse_file

for event in parse_file("capture.ts"):
    print(event)
```

See [docs/guide-python.md](https://github.com/aklofas/ts-transformer/blob/main/docs/guide-python.md) for the full guide.
