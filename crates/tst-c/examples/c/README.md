# tst-c examples (C)

Runnable C examples for the `tst-c` C ABI. Linux x86_64 only by build
convention (cbindgen-generated header + cdylib / staticlib).

If you're new to the project and want to start in Rust, the equivalent
curriculum lives at [`../../../examples/`](../../../examples/) — the C
examples here mirror that taxonomy.

## Build

```sh
cd crates/tst-c
cargo build                # produces target/debug/libtstrans.{so,a} + include/tstrans.h
gcc -I include -L ../../target/debug \
    -Wall -Werror -o /tmp/<name> \
    examples/c/<category>/<name>.c -ltstrans
LD_LIBRARY_PATH=../../target/debug /tmp/<name>
```

For pkg-config-using build systems, `tstrans.pc` is emitted alongside
the cdylib once `tstrans.pc.in` has been substituted by the build —
add `target/debug` to `PKG_CONFIG_PATH` and use
`pkg-config --cflags --libs tstrans`.

## Read in this order

### 1. `getting-started/hello_world.c` — smallest possible mux + KLV

The C twin of the Rust [`hello_world.rs`](../../../../examples/getting-started/hello_world.rs).
Builds 1 MPEG-TS frame containing 1 video AU + 1 KLV record using the
`tst_muxer_t` muxer-only handle, drains the bytes, prints the count
(byte-identical to the Rust version: 752 bytes / 4 packets).

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/hello_world
```

### 2. `muxing/send_synthetic.c` — sender + synthetic frames over SRT

Open a `tst_mux_sender_t` against an SRT URL, push 5 synthetic H.264 +
KLV frames, close. The C analogue of the Rust
[`pipeline_send_to_socket.rs`](../../../../examples/sending/pipeline_send_to_socket.rs).

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/send_synthetic 127.0.0.1:9000
```

### 3. `muxing/mux_dual_camera.c` — multi-stream within one program

Diff from §2: two video streams (EO + IR) sharing one program + KLV.
Demonstrates `tst_mux_config_add_video_stream` returning per-stream
handles and `tst_muxer_push_video_to(handle, ...)` for fan-out.
Mirrors the Rust [`mux_dual_camera.rs`](../../../../examples/muxing/mux_dual_camera.rs).

### 4. `muxing/mux_two_programs.c` — multi-program

Diff from §3: two PMTs in one PAT, each with its own video + KLV
streams. Shows the `tst_program_handle_t` flow and the
`tst_*_to(prog_handle, ...)` siblings. No Rust twin yet — the equivalent
recipe lives in the cookbook ([§16](../../../../docs/cookbook.md#16-repack-two-single-program-inputs-into-one-multi-program-ts))
under a different shape (demux + re-mux instead of synthetic frames).

## Why no receiver-side examples?

The C ABI is sender-only today (per the project's P0 backlog item).
Receiver-side C examples land alongside the future tst-c receiver
surface plan. Until then, run the Rust receiving examples
([`../../../../examples/receiving/`](../../../../examples/receiving/))
against C-side senders.
