# Sending

Senders that push pre-muxed TS bytes (or muxed-on-the-fly TS) over a
transport. Five examples, in read-order:

## 1. `pipeline_send_to_socket.rs` — basic SRT sender

```sh
cargo run -p tst-examples --example pipeline_send_to_socket -- 127.0.0.1:9000
```

Opens a `MuxSender` shell over an SRT caller socket, pushes synthetic
H.264 + KLV at 30 fps. The receiver-side analogue is
[`../receiving/srt_caller_dump.rs`](../receiving/srt_caller_dump.rs).

Cookbook: [§0 — Send a single TS packet](../../docs/cookbook.md#0-send-a-single-ts-packet-to-any-transport).

## 2. `encrypted_send_recv.rs` — passphrase encryption

```sh
cargo run -p tst-examples --example encrypted_send_recv
```

Diff from §1: pass a passphrase + key length on the `SocketBuilder`. The
listener side runs in the same process so the example is self-contained.

Cookbook: [§1 — Send video + KLV with passphrase encryption](../../docs/cookbook.md#1-send-video--klv-with-passphrase-encryption).

## 3. `sender_from_url.rs` — config from `srt://...?...` URL

```sh
cargo run -p tst-examples --example sender_from_url
```

Diff from §1: derive every SRT option from a URL with query parameters,
the way `tst-c::tst_*_open` consumes URLs. This is the surface that
makes the same configuration trivially shareable across language
bindings.

Cookbook: [§11 — Open a sender from an `srt://...?...` URL](../../docs/cookbook.md#11-open-a-sender-from-an-srt-url).

## 4. `srt_serve_ts_file.rs` — file-replay listener

```sh
cargo run -p tst-examples --example srt_serve_ts_file -- /path/to/source.ts 9000
```

Plays a captured `.ts` file as a listener over SRT, respecting PCR
cadence. Receivers that connect to it look like a live source. Different
shape from §1 — it uses the listener role, not the caller role.

## 5. `custom_transport.rs` — implement the `Transport` trait

```sh
cargo run -p tst-examples --example custom_transport
```

Build your own non-SRT wire by implementing `tst_core::Transport`. The
example wraps an in-memory `VecDeque` so you can see the shape; in
production you'd wrap a real socket / framing layer.

Cookbook: [§8 — Use a custom (non-SRT) transport](../../docs/cookbook.md#8-use-a-custom-non-srt-transport).
