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

### 2. `getting-started/version_check.c` — verify loaded library matches header

Cross-validates every `(runtime accessor, header macro)` pair at process
startup. Queries `tst_get_version_major/minor/patch/packed/string` and
`tst_get_abi_version_major/minor`, compares each against the corresponding
`TST_*_VERSION_*` compile-time macro from `<tstrans.h>`, and exits 0 only
when all values agree.

The canonical pattern for binding authors to copy into their own startup
checks (e.g. `JNI_OnLoad` for `srt-jni`, the UniFFI init hook for
`srt-uniffi`):

```c
if (tst_get_abi_version_major() != TST_ABI_VERSION_MAJOR) {
    fprintf(stderr, "tstrans ABI major mismatch\n");
    exit(1);
}
if (tst_get_abi_version_minor() < TST_ABI_VERSION_MINOR) {
    fprintf(stderr, "tstrans ABI minor too old\n");
    exit(1);
}
```

Expected output (versions will match your build):

```text
Package version (matches Cargo.toml):
  TST_VERSION_MAJOR          runtime=0  header=0  [OK]
  TST_VERSION_MINOR          runtime=1  header=1  [OK]
  TST_VERSION_PATCH          runtime=0  header=0  [OK]
  packed (M<<16|m<<8|p)      runtime=256  header=256  [OK]
  version string             runtime="0.1.0"

ABI contract version (breaking-change cadence):
  TST_ABI_VERSION_MAJOR      runtime=0  header=0  [OK]
  TST_ABI_VERSION_MINOR      runtime=1  header=1  [OK]

After tst_clear_last_error():
  tst_get_last_error()     = 0  (expect 0 = TST_E_SUCCESS)
  tst_get_last_error_str() = ""  (expect empty)

OK: all runtime/header pairs match. Loaded libtstrans is consistent with the tstrans.h compiled into this binary.
```

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/version_check
```

### 3. `muxing/send_synthetic.c` — sender + synthetic frames over SRT

Open a `tst_mux_sender_t` against an SRT URL, push 5 synthetic H.264 +
KLV frames, close. The C analogue of the Rust
[`pipeline_send_to_socket.rs`](../../../../examples/sending/pipeline_send_to_socket.rs).

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/send_synthetic 127.0.0.1:9000
```

### 4. `muxing/mux_dual_camera.c` — multi-stream within one program

Diff from §3: two video streams (EO + IR) sharing one program + KLV.
Demonstrates `tst_mux_config_add_video_stream` returning per-stream
handles and `tst_muxer_push_video_to(handle, ...)` for fan-out.
Mirrors the Rust [`mux_dual_camera.rs`](../../../../examples/muxing/mux_dual_camera.rs).

### 5. `muxing/mux_two_programs.c` — multi-program

Diff from §4: two PMTs in one PAT, each with its own video + KLV
streams. Shows the `tst_program_handle_t` flow and the
`tst_*_to(prog_handle, ...)` siblings. No Rust twin yet — the equivalent
recipe lives in the cookbook ([§16](../../../../docs/cookbook.md#16-repack-two-single-program-inputs-into-one-multi-program-ts))
under a different shape (demux + re-mux instead of synthetic frames).

### 6. `operations/socket_stats_poll.c` — live libsrt wire-stats polling

Push 5 seconds of synthetic video through a `tst_mux_sender_t` and
print RTT + bandwidth + loss + retransmits every 500 ms using
`tst_mux_sender_get_socket_stats`. Shows the operational-telemetry
counterpart to the app-level `tst_mux_sender_get_stats` — wire-level
visibility into what the network actually did (vs what you asked
libsrt to do). The C-side analogue of the
[`operations/`](../../../../examples/operations/) Rust examples.

```sh
LD_LIBRARY_PATH=../../target/debug /tmp/socket_stats_poll srt://127.0.0.1:9000
```

## Receiver-side C examples

The C ABI covers both sender (mux + raw/TS sender) and receiver (demux +
raw/TS receiver) surfaces. Receiver-side C examples ship under
[`receiving/`](receiving/):

- [`receiving/recv_ts_to_file.c`](receiving/recv_ts_to_file.c) — TS-bytes
  receiver: drain TS packets straight to a file via the C ABI's Receiver
  shape.
- [`receiving/recv_demux_to_console.c`](receiving/recv_demux_to_console.c)
  — DemuxReceiver event walk: print one line per demux event to stdout.
- [`receiving/recv_klv_to_stdout.c`](receiving/recv_klv_to_stdout.c) — KLV
  extraction: dump KLV payloads as hex / file on the side of a Demuxer.
- [`receiving/recv_raw_to_file.c`](receiving/recv_raw_to_file.c) — raw
  socket bytes (no TS framing): the lowest-level RawReceiver shape.
