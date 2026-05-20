# Receiving

Receivers that pull TS bytes from a transport (SRT or file-replay) and
either dump, demux, or re-mux them. Five examples, in read-order:

## 1. `srt_caller_dump.rs` — caller-mode SRT receiver

```sh
cargo run -p tst-examples --example srt_caller_dump -- 127.0.0.1:9000 /tmp/dump.ts
```

The simplest receiver: connect as a caller, write raw TS bytes to disk.
Pair with [`../sending/srt_serve_ts_file.rs`](../sending/srt_serve_ts_file.rs)
or any other listener.

Cookbook: [§5 — Receive into a file](../../docs/cookbook.md#5-receive-into-a-file).

## 2. `srt_listener_to_file.rs` — listener-mode SRT receiver

```sh
cargo run -p tst-examples --example srt_listener_to_file -- 127.0.0.1:9000 /tmp/out.ts
```

Diff from §1: take the listener role instead of caller. Pair with any
caller-mode sender (e.g.
[`../sending/pipeline_send_to_socket.rs`](../sending/pipeline_send_to_socket.rs)).

## 3. `srt_recv_typed.rs` — typed-event receiver via `DemuxReceiver<T>`

```sh
cargo run -p tst-examples --example srt_recv_typed -- 9000
```

Diff from §1/§2: instead of writing raw bytes, run them through the
`DemuxReceiver<T>` shell so you get typed `DemuxEvent` items
(per-stream samples, PSI events, errors). The shape every consumer
that wants real-time access to the parsed stream uses.

## 4. `ts_relay_from_file.rs` — file → SRT

```sh
cargo run -p tst-examples --example ts_relay_from_file -- input.ts 127.0.0.1:9000
```

Reads a pre-muxed `.ts` from disk and relays it over SRT via the
TS-bytes-in `pipeline::Sender` path (the input is already muxed
upstream, e.g. by ffmpeg). Useful for testing SRT options against a
captured stream without standing up a live encoder. Pair with
`srt-live-transmit srt://:9000 file:///tmp/out.ts` (or another
SRT-aware listener) on the receiver side.

Cookbook: [§4 — Relay a captured `.ts` file over SRT](../../docs/cookbook.md#4-relay-a-captured-ts-file-over-srt).

## 5. `demux_to_events.rs` — full file-feed demux loop

```sh
cargo run -p tst-examples --example demux_to_events -- /path/to/capture.ts
```

Reads a `.ts` file end-to-end and prints every `DemuxEvent` variant —
PSI + per-stream sample headers + non-conformant issues. Use this as
the diagnostic example when you want to see what the demuxer thinks
of a capture.

Cookbook: [§21 — Extract subtitle PES bytes from a captured `.ts` file](../../docs/cookbook.md#21-extract-subtitle-pes-bytes-from-a-captured-ts-file)
(adjacent recipe — the same demux loop with subtitle filtering).
