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

Cookbook: [Pair sync-KLV with video AUs by nearest PTS](../../docs/cookbook/pairing/pair-klv-by-pts.md).

## 2. `pair_klv_pipeline.rs` — using `pipeline::Pairer`

```sh
cargo run -p tst-examples --example pair_klv_pipeline -- path/to/capture.ts
```

Diff from §1: replace the hand-rolled matcher with `Pairer::with_config`
(in `Realtime` mode). Pairer takes care of bounded history, typed
projections (`VideoSample` / `KlvSample`), and telemetry counters. This
is the production shape — less boilerplate, same result.

Cookbook recipes:

- [Pair sync-KLV with video AUs via `Pairer::with_config` (Realtime)](../../docs/cookbook/pairing/pairer-realtime.md)
- [Pair sync-KLV in batch mode (`PairerMode::Buffered`)](../../docs/cookbook/pairing/pairer-batch.md)
- [Sample-and-hold async KLV via `Pairer::last_before_pts`](../../docs/cookbook/pairing/pairer-last-before-pts.md)
- [EO + IR composition with shared async-KLV](../../docs/cookbook/pairing/eo-ir-shared-klv-pairer.md)
