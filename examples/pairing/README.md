# Pairing video + KLV

Match KLV records to video access units by PTS. Two examples — the
same task, two ways. Read both: §1 makes the matching logic visible,
§2 is what you'd actually ship.

## 1. `pair_sync_klv.rs` — manual matching via `DemuxEvent`

```sh
cargo run -p tst-examples --example pair_sync_klv -- path/to/capture.ts
```

Drive the demuxer directly, watch the `DemuxEvent` stream, hand-roll
the nearest-PTS match. The "see exactly what's happening" recipe —
useful when you want to understand the matching shape before reaching
for the helper.

Cookbook: [§12 — Pair sync-KLV with video AUs by nearest PTS](../../docs/cookbook.md#12-pair-sync-klv-with-video-aus-by-nearest-pts).

## 2. `pair_klv_pipeline.rs` — using `pipeline::Pairer`

```sh
cargo run -p tst-examples --example pair_klv_pipeline -- path/to/capture.ts
```

Diff from §1: replace the hand-rolled matcher with `Pairer::with_config`
(in `Realtime` mode). Pairer takes care of bounded history, typed
projections (`VideoSample` / `KlvSample`), and telemetry counters. This
is the production shape — less boilerplate, same result.

Cookbook recipes:

- [§24 — Pair sync-KLV with video AUs via `Pairer::with_config` (Realtime)](../../docs/cookbook.md#24-pair-sync-klv-with-video-aus-via-pairerwith_config-realtime)
- [§25 — Pair sync-KLV in batch mode (`PairerMode::Buffered`)](../../docs/cookbook.md#25-pair-sync-klv-in-batch-mode-pairermodebuffered)
- [§26 — Sample-and-hold async KLV via `Pairer::last_before_pts`](../../docs/cookbook.md#26-sample-and-hold-async-klv-via-pairerlast_before_pts)
- [§27 — EO + IR composition with shared async-KLV](../../docs/cookbook.md#27-eo--ir-composition-with-shared-async-klv)
