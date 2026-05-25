# Getting started

The first example to run. Builds 1 MPEG-TS frame containing 1 H.264
access unit and 1 ST 0601 KLV record entirely in memory — no SRT, no
files, no real codec output.

## Read in this order

### 1. `hello_world.rs` — smallest possible mux + KLV

```sh
cargo run -p tst-examples --example hello_world
```

What you'll see:

- The `Muxer` + `MuxerConfigBuilder` API in their simplest form
  (`MuxerProgramConfigBuilder::new(program_number, pmt_pid)` →
  `add_video` / `add_klv` → `build` → bind onto
  `MuxerConfigBuilder::add_program` → `build`).
- ST 0601 KLV LS encoding via `UasDatalinkLs::default()` +
  `encode_to_vec`.
- The packet-pull loop that drains the muxer (`Muxer::pull` writing
  188-byte TS packets into a caller-owned buffer).

When you're ready to do something real:

- Send over SRT: [`../sending/pipeline_send_to_socket.rs`](../sending/pipeline_send_to_socket.rs)
- Write to a file: [`../muxing/mux_to_file.rs`](../muxing/mux_to_file.rs)
- Carry real H.265 video: [`../muxing/mux_h265_with_klv.rs`](../muxing/mux_h265_with_klv.rs)
- Decode the KLV blob back: [`../klv-metadata/klv_decode_file.rs`](../klv-metadata/klv_decode_file.rs)

Cookbook backlink:
[Recipe 0 — Send a single TS packet to any `Transport`](../../docs/cookbook/sending/00-send-single-packet.md)
(the inline form of the same shape; this example is the no-transport
mux-only twin).

The C twin lives at
[`../../crates/tst-c/examples/c/getting-started/hello_world.c`](../../crates/tst-c/examples/c/getting-started/hello_world.c)
and produces a byte-identical output (752 bytes / 4 packets).
