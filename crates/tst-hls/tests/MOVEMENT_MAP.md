# tst-hls integration-test map

The tst-hls test suite uses standalone binaries — one file per domain.
This file documents what each binary covers.

## Test binaries

| Binary | Feature gates | What it covers |
|---|---|---|
| `hls_e2e` | `serve` | `MuxPublisher<HlsPublisher>` over TCP; an `ffmpeg` pull client fetches `/playlist.m3u8` and verifies byte-identity with the source TS. Skipped gracefully if `ffmpeg` is absent. Moved from `crates/tst-tcp/tests/hls_e2e.rs`. |

## Equivalence note

`hls_e2e` was originally in `crates/tst-tcp/tests/` (gated `#![cfg(feature = "hls")]`).
It was relocated here when the HLS module moved from `tst-tcp` into `tst-hls`;
the feature gate was updated from `hls` → `serve` to match tst-hls's feature names.
