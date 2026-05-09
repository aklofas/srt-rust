# TS Transformer (`ts-transformer`)

Cross-platform SRT-based libraries for live video streaming from **gimbaled platforms** — drones (rotary and fixed-wing UAVs), manned fixed-wing aircraft with sensor pods, helicopters with EO/IR turrets, and other manned/unmanned platforms carrying stabilized imaging payloads.

**Status:** Sender + receiver pipelines complete on Linux x86_64. `srt-sys` (raw FFI + vendored mbedTLS), `tst-srt` (safe `Socket` / `Listener` / config + builder API), `tst-core::klv` (typed MISB ST 0601 + ST 0605 over a generic SMPTE / MISB substrate), `tst-core::mpegts::mux` (single-program TS muxer with H.264 / H.265 / H.266 / AV1 video + ST 0601 KLV per ST 1402 / ST 1910), `tst-core::mpegts::demux` (receiver-side TS demuxer — bytes in, typed `DemuxEvent` out; lenient by default with four-tier `StrictMode` ladder; AU-cell unwrap; consumer-side pairing via cookbook recipes), and `tst-pipeline` (composition layer: `Transport` + `RecvTransport` traits, `MuxSender` / `Sender` / `RawSender`, `DemuxReceiver` / `Receiver` / `RawReceiver`, reconnecting `ManagedTransport` + `ManagedReceiveTransport`, `add_byte_sink` fan-out for tee patterns) are all implemented. `tst-c` exposes the sender pipeline as a stable C ABI (`cdylib` + `staticlib` + cbindgen-generated `tstrans.h` + `tstrans.pc`). Workspace ships hundreds of tests across both feature modes. The remaining binding crates (`srt-jni`, `srt-uniffi`) are next on the roadmap.

## Scope

- Container: **MPEG-TS**
- Metadata: **MISB ST 0601 KLV** (multiplexed per MISB ST 1402 / ST 1910)
- Transport: **SRT** (Haivision libsrt 1.5.5, vendored)
- Encryption: **mbedTLS 3.6.6 LTS** (vendored, statically linked, on by default)

For a feature-by-feature support matrix — SRT options, MISB specs, typed ST 0601 items, and what's planned vs. out of scope — see [`docs/compatibility.md`](docs/compatibility.md).

## Hello, MPEG-TS

Mux one H.264 access unit + one KLV blob into 188-byte TS packets — no SRT, no peer:

```rust
use tst_core::mpegts::mux::{Muxer, MuxerConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Default config: one program, H.264 video on PID 0x1011, async KLV on 0x1031.
    let mut muxer = Muxer::new(MuxerConfig::default())?;

    // Minimal Annex-B H.264 IDR NAL: start code + nal_header + filler.
    let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0xA5, 0xA5, 0xA5];
    muxer.push_video(&nal, /* pts_90khz */ 0, /* key_frame */ true)?;
    muxer.push_klv(&[0x06, 0x0E, 0x2B, 0x34, 0xDE, 0xAD, 0xBE, 0xEF], 0, 0x00)?;

    let mut buf = [0u8; 1316];
    let mut packets = Vec::new();
    loop {
        let n = muxer.pull(&mut buf);
        if n == 0 { break; }
        packets.extend_from_slice(&buf[..n]);
    }
    println!("muxed {} bytes ({} TS packets)", packets.len(), packets.len() / 188);
    Ok(())
}
```

`cargo run --example mux_to_file -- out.ts 5` runs the longer 5-second version. To send those bytes over SRT, see [`docs/getting-started.md`](docs/getting-started.md).

## Architecture

A Rust core wrapping libsrt via FFI, with bindings for JVM (JNI, JDK 17+), iOS/Android (UniFFI), and embedded Linux (cdylib + cbindgen). Both directions are first-class: `mpegts::mux` muxes encoded NALs + KLV into a TS stream; `mpegts::demux` reverses it (bytes in, typed `DemuxEvent` stream out — with sync recovery, AU cell unwrap, NAL split, and lenient handling of real-world non-conformance). Consumers can still feed extracted bytes to FFmpeg / JavaCV / Bento4 if they prefer, but Rust-native demux is now the default.

## Documentation

The repo's documentation lives under [`docs/`](docs/):

- **[`getting-started.md`](docs/getting-started.md)** — install, first send, first receive in 10 minutes.
- **[`architecture.md`](docs/architecture.md)** — crate graph, pipeline composition model, sync vs. async stance.
- **[`guide-codec.md`](docs/guide-codec.md)** — typed codec parameter-set parsers (`codec::h264`, `codec::h265`, `codec::h266`, `codec::av1`); stateless SPS / VPS / PPS / Sequence Header parsing from demuxer NAL / OBU bytes; shared color and frame-rate types.
- **[`guide-klv.md`](docs/guide-klv.md)** — generic substrate plus typed ST 0601 / ST 0605 / ST 1910 layers; the four-rung decode strictness ladder.
- **[`guide-mpegts-demux.md`](docs/guide-mpegts-demux.md)** — `Demuxer`, `DemuxEvent`, `StrictMode` ladder, override surface, robustness behaviours, decoupled-pairing rationale.
- **[`guide-mpegts-mux.md`](docs/guide-mpegts-mux.md)** — `Config` / `ConfigBuilder`, codec + KLV-mode selection, PCR/PSI cadence, push/pull contract.
- **[`guide-pipeline.md`](docs/guide-pipeline.md)** — picking among `MuxSender` / `Sender` / `RawSender` (send) and `DemuxReceiver` / `Receiver` / `RawReceiver` (receive); the `Transport` and `RecvTransport` traits; `ManagedTransport` / `ManagedReceiveTransport` reconnect; `add_byte_sink` fan-out.
- **[`guide-srt.md`](docs/guide-srt.md)** — `Socket` / `Listener`, encryption, latency, stats, error model.
- **[`cookbook.md`](docs/cookbook.md)** — recipes linking to runnable examples.
- **[`troubleshooting.md`](docs/troubleshooting.md)** — diagnose build failures, connection failures, KLV rejection, TS framing issues, reconnect loops.
- **[`deferred-features.md`](docs/deferred-features.md)** — what's not yet supported and the trigger conditions to revisit.
- **[`compatibility.md`](docs/compatibility.md)** — feature-by-feature support matrix.
- **[`binding-authors.md`](docs/binding-authors.md)** — entry point for `srt-jni`, `srt-uniffi`, `tst-pyo3` authors.

Runnable Rust examples live at [`examples/`](examples/) — a workspace-level `tst-examples` crate organized into 8 task-oriented subfolders. Run any with `cargo run -p tst-examples --example <name>`. See `cookbook.md` for which example illustrates which recipe.

## Workspace layout

```
crates/
  srt-sys/       raw libsrt FFI (bindgen-generated against libsrt 1.5.5)             ✅ done
  tst-core/      parsing + codec core — klv:: ✅, mpegts::mux:: ✅, mpegts::demux:: ✅
  tst-pipeline/  pipeline shells — MuxSender/Sender/RawSender, DemuxReceiver, etc.   ✅ done
  tst-srt/       SRT transport + top-level examples                                  ✅ done
  tst-c/         cdylib + staticlib + cbindgen header — Linux x86_64                 ✅ done
  srt-jni/       JNI bindings — JAR for JDK 17+ JVM consumers                       ⏳ planned
  srt-uniffi/    Swift/Kotlin via UniFFI — iOS/Android frameworks                    ⏳ planned
vendor/
  srt/           Haivision libsrt git submodule, pinned at v1.5.5
  mbedtls/       mbedTLS git submodule, pinned at v3.6.6 (LTS)
```

## Crates

### `srt-sys` — raw FFI bindings to libsrt

Bindgen-generated against libsrt 1.5.5, edition 2024, MSRV 1.85. Exposes ~72 `srt_*` functions and the full `SRT_*` constant / type surface. Encryption is wired in via mbedTLS by default; opt out with `--no-default-features`. The build script discovers an installed libsrt via `pkg-config` first and falls back to compiling the vendored submodule (force vendored with `SRT_FORCE_VENDORED=1`).

#### Features

| Feature   | Default | What it does                                                                                |
| --------- | ------- | ------------------------------------------------------------------------------------------- |
| `mbedtls` | ✅ on    | Build vendored mbedTLS and link libsrt with `USE_ENCLIB=mbedtls` + `ENABLE_ENCRYPTION=ON`. |

#### Usage

```toml
[dependencies]
srt-sys = { git = "https://github.com/aklofas/ts-transformer" }

# Or, to skip the mbedTLS build and disable encryption:
srt-sys = { git = "https://github.com/aklofas/ts-transformer", default-features = false }
```

`srt-sys` is intended as a foundation for higher-level crates in this workspace. Most callers should use `tst-core` / `tst-srt`'s safe API rather than write `unsafe` against the raw bindings directly.

### `tst-core`, `tst-pipeline`, `tst-srt` — safe Rust API

Three crates built on `srt-sys`. `tst-core` holds parsing and codec logic; `tst-pipeline` holds the pipeline shells; `tst-srt` ties them together and provides examples. Together they provide:

- **SRT transport** (`tst_srt`) — `Socket`, `Listener`, `SocketConfig` / `ListenerConfig`, fluent `SocketBuilder` / `ListenerBuilder`, AES-128/192/256 passphrase-based encryption, packet-filter strings, latency / bandwidth / TLPKTDROP / flow-window / SRTO_STREAMID tunables, and `Stats` snapshots. Per-call-category error model. Sync blocking API today; async / reactor are deferred. Plus `srt://host:port?key=value&...` URL parsing (libsrt vocabulary, URL-wins-over-builder precedence) — see [`docs/guide-srt.md#url-parsing`](docs/guide-srt.md#url-parsing).
- **KLV codec** (`tst_core::klv`) — generic SMPTE / MISB substrate plus typed MISB ST 0601 and ST 0605 layers (see below). Also includes AU-cell wrap/unwrap for sync KLV in TS.
- **MPEG-TS muxer** (`tst_core::mpegts::mux`) — sender-side `Muxer` for single-program TS with H.264 / H.265 / H.266 / AV1 video and ST 0601 KLV multiplexed per ST 1402 (async via `stream_type 0x06` + KLVA registration descriptor, or sync via ST 1910 AU cell). Multi-stream-shaped `Config` (Path 2) ready for additive Path 3 expansion. PCR/PTS/PSI cadence configurable; deterministic output (no wall-clock dependency).
- **Pipeline composition** (`tst_pipeline`) — composes the muxer with a `Transport` (the SRT socket, or any custom transport) into ergonomic sender shells: `MuxSender` (NAL+KLV → TS → SRT, internally synchronized, lossless across transient failures), `Sender` (pre-muxed TS → SRT with sync-byte framing/recovery in RECOVER or STRICT mode), `RawSender` (one byte-blind message per send). All three are generic over `Transport`; wrap any of them with `ManagedTransport<T>` for reconnect + gap-buffer behavior with configurable backoff and overflow policy.

#### Features

| Feature   | Default | What it does                                                                          |
| --------- | ------- | ------------------------------------------------------------------------------------- |
| `mbedtls` | ✅ on    | Propagates to `srt-sys/mbedtls`. Disable with `--no-default-features` for unencrypted libsrt. |
| `log`     | ✅ on    | Forward libsrt's internal logging through the `log` facade for `env_logger` etc.     |

#### Usage

```toml
[dependencies]
tst-srt = { git = "https://github.com/aklofas/ts-transformer" }
```

```rust
use tst_srt::{ListenerBuilder, SocketBuilder, Passphrase};
use std::time::Duration;

// Listener side
let mut listener = ListenerBuilder::new()
    .passphrase(Passphrase::new("my-shared-secret-1234")?)
    .latency_ms(120)
    .bind("0.0.0.0:1234")?;
let (socket, peer) = listener.accept()?;

// Caller side
let mut socket = SocketBuilder::new()
    .passphrase(Passphrase::new("my-shared-secret-1234")?)
    .latency_ms(120)
    .recv_timeout(Duration::from_secs(5))
    .connect("aircraft:1234")?;
socket.send(b"hello")?;
```

The `SocketConfig` / `ListenerConfig` structs are the canonical configuration types — bindings (UniFFI, JNI, cbindgen) consume them as plain dictionaries / POJOs / C structs. The builders are sugar over the same types.

#### KLV codec (`tst_core::klv`)

A two-layer codec living in `tst-core`:

- **Generic substrate** — 16-byte SMPTE Universal Labels (`UniversalLabel` with byte-level introspection + family check), BER short / long and BER-OID length codecs (`klv::length`), MISB ST 1201.5 IMAPB integer ↔ float mapping (`klv::imapb`), 16-bit running-sum checksum (`klv::checksum`), and zero-allocation iterators over local-set / universal-set packs (`klv::pack::Iter`, `RawField`, `OwnedRawField`). Honours the MISB ST 0107.5 future-proof skip rule — unknown tags pass through as `OwnedRawField` instead of being dropped or causing failure.
- **Typed MISB ST 0601** (`klv::st0601`) — `UasDatalinkLs` flat struct mirroring the wire format with **49 of 143 typed items** (timestamp, platform attitude / airspeed, sensor lat/lon/altitude/FOV/azimuth/elevation/roll, slant range, frame center + ellipsoid heights, full-resolution corner lat/lon, full-resolution platform pitch / roll, security-LS pass-through, version, and more). Composite views: `GeoPoint`, `Attitude`, `FieldOfView`, `Corners`. Four decode entry points trade off strictness — `decode` (checksum-verified, any UL), `decode_unchecked` (skips checksum), `decode_strict` (gates on the ST 0601-family UL), and `decode_strict_compliance` (also enforces ST 0601.8-09/-11/-12 mandatory structure rules). Encoder auto-emits Tag 1 checksum and Tag 65 version when unset.
- **Typed MISB ST 0605** (`klv::st0605`) — Precision Time Stamp Pack (`PrecisionTimeStampPack` + `TimeStatus(u8)` newtype with `is_locked` / `has_discontinuity` / `is_reverse_jump` / `reserved_bits_valid` accessors per MISB ST 0603.5 §7.4). Decode and encode for the 26-byte pack commonly multiplexed alongside ST 0601 records in real captures.

For the full feature-by-feature matrix — SRT options, MISB specs, every typed ST 0601 item, decode strictness ladder, planned vs. out of scope — see [`docs/compatibility.md`](docs/compatibility.md).

### `tst-c` — stable C ABI for the sender pipeline

A binding crate that exposes `tst-pipeline` to non-Rust callers (C / C++ directly, plus anything else that can call into a C ABI). Built as `cdylib` + `staticlib` so consumers either dynamic-link `libtstrans.so` or static-link `libtstrans.a`; libsrt and mbedTLS are statically embedded into both, so consumers don't need libsrt installed system-wide. The C header (`tstrans.h`) is generated by cbindgen, committed at [`crates/tst-c/include/tstrans.h`](crates/tst-c/include/tstrans.h), and CI verifies no drift.

#### What's exposed

Seven opaque-handle types covering the send-side pipeline:

- **`tst_muxer_t`** — standalone TS muxer utility (no transport). Push NALs and KLV blobs, pull TS bytes; useful for callers that send TS through their own transport (UDP, file, ffmpeg pipe, etc.).
- **`tst_mux_sender_t`** / **`tst_managed_mux_sender_t`** — canonical NAL+KLV → TS → SRT sender. Plain L1 connects once; managed L2 wraps with reconnect + gap buffer.
- **`tst_ts_sender_t`** / **`tst_managed_ts_sender_t`** — pre-muxed TS bytes → SRT, with sync framing/recovery (RECOVER auto-resync or STRICT fail-fast). Plus `tst_ts_sender_get_stats()` for `bytes_pushed` / `bytes_skipped_for_sync` / `resync_events` / `packets_sent`.
- **`tst_raw_sender_t`** / **`tst_managed_raw_sender_t`** — byte-blind one-shot primitive. One `_send` call = one outbound SRT message of the exact length passed in.

Configuration is via opaque builders (`tst_mux_config_t`, `tst_ts_sender_config_t`, `tst_raw_sender_config_t`, `tst_reconnect_policy_t`) — `_new` / setters / `_free`. Errors follow libsrt's idiom: every fallible call returns `0` on success or a negative `TST_E_*` code, with thread-local detail accessible via `tst_get_last_error()` / `tst_get_last_error_str()`.

#### Distribution (Linux x86_64)

`cargo build -p tst-c --release` produces:

- `target/release/libtstrans.so` — runtime cdylib (~1.6 MB, libsrt + mbedTLS + libstdc++ statically embedded).
- `target/release/libtstrans.a` — static library.
- `target/release/include/tstrans.h` — cbindgen header.
- `target/release/tstrans.pc` — pkg-config metadata (`pkg-config --cflags --libs tstrans`).

#### Usage

```c
#include "tstrans.h"

tst_mux_config_t* cfg = tst_mux_config_new();
tst_mux_config_add_video(cfg, 0x1011, TST_VIDEO_CODEC_H264);
tst_mux_config_add_klv(cfg, 0x1031, TST_KLV_STREAM_TYPE_PRIVATE_DATA, false);

tst_mux_sender_t* s = tst_mux_sender_open("srt://10.0.0.1:9000", cfg);
tst_mux_config_free(cfg);
if (!s) {
    fprintf(stderr, "open failed: %s\n", tst_get_last_error_str());
    return 1;
}

tst_mux_sender_send_video(s, nal_bytes, nal_len, /* pts_90khz */ 0, /* key_frame */ true);
tst_mux_sender_send_klv(s, klv_bytes, klv_len, /* pts_90khz */ 0);

tst_mux_sender_close(s);
```

A complete C example lives at [`crates/tst-c/examples/c/send_synthetic.c`](crates/tst-c/examples/c/send_synthetic.c). Multi-platform builds (macOS, Windows, Linux aarch64) and the JVM (`srt-jni`) / iOS-Android (`srt-uniffi`) sibling crates are next on the roadmap.

## Building

### Prerequisites

- Rust 1.85+ (MSRV declared in workspace `Cargo.toml`; `rust-toolchain.toml` pins to `stable` for local development).
- A C/C++ toolchain (libsrt and mbedTLS are compiled from source).
- `cmake` and `pkg-config` on `PATH`.
- Python 3 (mbedTLS's build system uses it for code generation; default-on Ubuntu/Debian/macOS).

On Debian/Ubuntu:

```bash
sudo apt-get install -y build-essential cmake pkg-config python3
```

### Clone with submodules

```bash
git clone --recurse-submodules https://github.com/aklofas/ts-transformer.git
cd ts-transformer
# Or, if already cloned without submodules:
git submodule update --init --recursive
```

### Build & test

By default the build script tries `pkg-config srt ≥ 1.5.0` first, falling back to compiling the vendored `vendor/srt`. Force the vendored path with `SRT_FORCE_VENDORED=1` (recommended for reproducible builds):

```bash
SRT_FORCE_VENDORED=1 cargo test --workspace                        # ~313 tests (default features)
SRT_FORCE_VENDORED=1 cargo test --workspace --no-default-features  # ~308 tests (no encryption)
SRT_FORCE_VENDORED=1 cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

A clean rebuild compiles libsrt and mbedTLS from source — expect 3–5 minutes the first time, seconds on warm builds. The `--no-default-features` test path skips the mbedTLS build entirely (~1–2 min faster on cold cache).

To build just the C ABI artifacts (cdylib, staticlib, header, pc-file):

```bash
SRT_FORCE_VENDORED=1 cargo build -p tst-c --release
ls target/release/libtstrans.so target/release/libtstrans.a \
   target/release/include/tstrans.h target/release/tstrans.pc
```

### KLV examples & fixtures

```bash
# Regenerate the synthetic ST 0601 fixtures committed under
# crates/tst-core/tests/fixtures/st0601/
cargo run --example gen_synthetic_fixtures

# Extract every KLV blob from a .ts file (writes one .klv per record).
cargo run --example extract_klv -- /path/to/capture.ts
```

Drop sensitive real-world `.ts` / `.klv` captures into `crates/tst-srt/tests/fixtures/local/` (gitignored). `tests/local_fixtures.rs` picks them up automatically and applies shape-keyed assertions documented in `crates/tst-srt/tests/TEST_CORPUS.md`.

Fuzz the ST 0601 decoder (requires nightly + `cargo-fuzz`):

```bash
cd crates/tst-core
cargo +nightly fuzz run klv_st0601_decode
cargo +nightly fuzz run klv_iter
```

### CI

Linux x86_64 CI runs `cargo fmt --check`, `cargo clippy -D warnings`, and the test suite in both feature modes (default + `--no-default-features`) against the vendored libsrt + mbedTLS builds. See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Project conventions

- **Direct pushes to `main`.** Single-developer linear history; no feature branches by default.
- **Subject-only commit messages** unless the why is non-obvious. No AI-attribution trailers.
- **Submodules pinned by tag** (libsrt v1.5.5, mbedTLS v3.6.6 LTS). Submodule advances are deliberate, separate commits.
- **Edition 2024, MSRV 1.85.** Bindgen emits `unsafe extern "C"` blocks (`.rust_edition(Edition2024)`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Vendored dependencies

- `vendor/srt` — Haivision libsrt, MPL-2.0.
- `vendor/mbedtls` — Mbed TLS, dual-licensed Apache-2.0 / GPL-2.0-or-later.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
