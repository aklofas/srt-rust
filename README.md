# srt-rust

Cross-platform SRT-based libraries for live video streaming from **gimbaled platforms** — drones (rotary and fixed-wing UAVs), manned fixed-wing aircraft with sensor pods, helicopters with EO/IR turrets, and other manned/unmanned platforms carrying stabilized imaging payloads.

**Status:** active development. `srt-sys` (raw FFI + mbedTLS encryption), `srt-core::srt` (safe `Socket` / `Listener` / config + builder API), and `srt-core::klv` (typed MISB ST 0601 + ST 0605 with four decode strictness levels and a generic SMPTE / MISB substrate) are implemented and exercised by ~130 unit tests plus a real-world `.klv` fixture suite. The MPEG-TS muxer (`mpegts::mux`) and the binding crates (`srt-c`, `srt-jni`, `srt-uniffi`) are next on the roadmap.

## Scope

- Container: **MPEG-TS**
- Metadata: **MISB ST 0601 KLV** (multiplexed per MISB ST 1402 / ST 1910)
- Transport: **SRT** (Haivision libsrt 1.5.5, vendored)
- Encryption: **mbedTLS 3.6.6 LTS** (vendored, statically linked, on by default)

For a feature-by-feature support matrix — SRT options, MISB specs, typed ST 0601 items, and what's planned vs. out of scope — see [`docs/compatibility.md`](docs/compatibility.md).

## Architecture

A Rust core wrapping libsrt via FFI, with bindings for JVM (JNI, JDK 17+), iOS/Android (UniFFI), and embedded Linux (cdylib + cbindgen). MPEG-TS demux stays out of scope — receivers use FFmpeg / JavaCV / Bento4 / platform demuxers and feed extracted KLV bytes through `srt_core::klv::st0601::decode` (or `st0605::decode` for Precision Time Stamp Packs).

## Workspace layout

```
crates/
  srt-sys/      raw libsrt FFI (bindgen-generated against libsrt 1.5.5)        ✅ done
  srt-core/     safe Rust API — srt:: ✅, klv:: ✅, mpegts::/pipeline::         ⏳ planned
  srt-c/        cdylib + cbindgen header — embedded, future Panama/FFM         ⏳ planned
  srt-jni/      JNI bindings — JAR for JDK 17+ JVM consumers                    ⏳ planned
  srt-uniffi/   Swift/Kotlin via UniFFI — iOS/Android frameworks               ⏳ planned
vendor/
  srt/          Haivision libsrt git submodule, pinned at v1.5.5
  mbedtls/      mbedTLS git submodule, pinned at v3.6.6 (LTS)
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
srt-sys = { git = "https://github.com/aklofas/srt-rust" }

# Or, to skip the mbedTLS build and disable encryption:
srt-sys = { git = "https://github.com/aklofas/srt-rust", default-features = false }
```

`srt-sys` is intended as a foundation for higher-level crates in this workspace. Most callers should use `srt-core`'s safe API rather than write `unsafe` against the raw bindings directly.

### `srt-core` — safe Rust API

Built on `srt-sys`. Provides:

- **SRT transport** (`srt_core::srt`) — `Socket`, `Listener`, `SocketConfig` / `ListenerConfig`, fluent `SocketBuilder` / `ListenerBuilder`, AES-128/192/256 passphrase-based encryption, packet-filter strings, latency / bandwidth / TLPKTDROP / flow-window / SRTO_STREAMID tunables, and `Stats` snapshots. Per-call-category error model. Sync blocking API today; async / reactor are deferred.
- **KLV codec** (`srt_core::klv`) — generic SMPTE / MISB substrate plus typed MISB ST 0601 and ST 0605 layers (see below).

#### Features

| Feature   | Default | What it does                                                                          |
| --------- | ------- | ------------------------------------------------------------------------------------- |
| `mbedtls` | ✅ on    | Propagates to `srt-sys/mbedtls`. Disable with `--no-default-features` for unencrypted libsrt. |
| `log`     | ✅ on    | Forward libsrt's internal logging through the `log` facade for `env_logger` etc.     |

#### Usage

```toml
[dependencies]
srt-core = { git = "https://github.com/aklofas/srt-rust" }
```

```rust
use srt_core::srt::{ListenerBuilder, SocketBuilder, Passphrase};
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

#### KLV codec (`srt_core::klv`)

A two-layer codec living entirely inside `srt-core`:

- **Generic substrate** — 16-byte SMPTE Universal Labels (`UniversalLabel` with byte-level introspection + family check), BER short / long and BER-OID length codecs (`klv::length`), MISB ST 1201.5 IMAPB integer ↔ float mapping (`klv::imapb`), 16-bit running-sum checksum (`klv::checksum`), and zero-allocation iterators over local-set / universal-set packs (`klv::pack::Iter`, `RawField`, `OwnedRawField`). Honours the MISB ST 0107.5 future-proof skip rule — unknown tags pass through as `OwnedRawField` instead of being dropped or causing failure.
- **Typed MISB ST 0601** (`klv::st0601`) — `UasDatalinkLs` flat struct mirroring the wire format with **49 of 143 typed items** (timestamp, platform attitude / airspeed, sensor lat/lon/altitude/FOV/azimuth/elevation/roll, slant range, frame center + ellipsoid heights, full-resolution corner lat/lon, full-resolution platform pitch / roll, security-LS pass-through, version, and more). Composite views: `GeoPoint`, `Attitude`, `FieldOfView`, `Corners`. Four decode entry points trade off strictness — `decode` (checksum-verified, any UL), `decode_unchecked` (skips checksum), `decode_strict` (gates on the ST 0601-family UL), and `decode_strict_compliance` (also enforces ST 0601.8-09/-11/-12 mandatory structure rules). Encoder auto-emits Tag 1 checksum and Tag 65 version when unset.
- **Typed MISB ST 0605** (`klv::st0605`) — Precision Time Stamp Pack (`PrecisionTimeStampPack` + `TimeStatus(u8)` newtype with `is_locked` / `has_discontinuity` / `is_reverse_jump` / `reserved_bits_valid` accessors per MISB ST 0603.5 §7.4). Decode and encode for the 26-byte pack commonly multiplexed alongside ST 0601 records in real captures.

For the full feature-by-feature matrix — SRT options, MISB specs, every typed ST 0601 item, decode strictness ladder, planned vs. out of scope — see [`docs/compatibility.md`](docs/compatibility.md).

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
git clone --recurse-submodules https://github.com/aklofas/srt-rust.git
cd srt-rust
# Or, if already cloned without submodules:
git submodule update --init --recursive
```

### Build & test

By default the build script tries `pkg-config srt ≥ 1.5.0` first, falling back to compiling the vendored `vendor/srt`. Force the vendored path with `SRT_FORCE_VENDORED=1` (recommended for reproducible builds):

```bash
SRT_FORCE_VENDORED=1 cargo test --workspace
SRT_FORCE_VENDORED=1 cargo test --workspace --no-default-features  # unencrypted variant
SRT_FORCE_VENDORED=1 cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

A clean rebuild compiles libsrt and mbedTLS from source — expect 3–5 minutes the first time, seconds on warm builds. The `--no-default-features` test path skips the mbedTLS build entirely (~1–2 min faster on cold cache).

### KLV examples & fixtures

```bash
# Regenerate the synthetic ST 0601 fixtures committed under
# crates/srt-core/tests/fixtures/st0601/
cargo run --example gen_synthetic_fixtures

# Extract every KLV blob from a .ts file (writes one .klv per record).
cargo run --example extract_klv -- /path/to/capture.ts
```

Drop sensitive real-world `.ts` / `.klv` captures into `crates/srt-core/tests/fixtures/local/` (gitignored). `tests/local_fixtures.rs` picks them up automatically and applies shape-keyed assertions documented in `crates/srt-core/tests/TEST_CORPUS.md`.

Fuzz the ST 0601 decoder (requires nightly + `cargo-fuzz`):

```bash
cd crates/srt-core
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
