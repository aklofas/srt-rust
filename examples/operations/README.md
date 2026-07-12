# Operations

Patterns for production deployments — surviving flaky transports,
fanning bytes to multiple consumers, observability. Two examples:

## 1. `managed_reconnect.rs` — `ManagedTransport` reconnect with backoff

```sh
cargo run -p tst-examples --example managed_reconnect
```

Wrap any `Transport` in `ManagedTransport` and the pipeline survives
disconnects: bounded backoff, gap-buffer for in-flight bytes, and
connection-state telemetry on the `tracing` facade. Drop a sender mid-
flight, reconnect a few seconds later, watch the example recover.

Cookbook: [Survive a flaky transport with reconnect + gap buffer](../../docs/cookbook/operations/managed-transport-reconnect.md).

## 2. `tee_disk_and_demux.rs` — fan-out byte-sink pattern

```sh
cargo run -p tst-examples --example tee_disk_and_demux -- <input.ts> <output.ts>
```

Use `add_byte_sink` on a `DemuxReceiver` to fan TS bytes out to multiple
consumers without copying: write to disk and feed a live demux loop in
one pass. The shape that wires the receiver into both archival and
analysis paths.

Adjacent cookbook recipes: [Print live `Stats` from a sender](../../docs/cookbook/operations/print-live-stats.md),
[Inject WebVTT POI cues into a live MPEG-TS uplink](../../docs/cookbook/operations/inject-webvtt-cues.md).
