# srt-rust

Cross-platform SRT-based libraries for live video streaming from **gimbaled platforms** — drones (rotary and fixed-wing UAVs), manned fixed-wing aircraft with sensor pods, helicopters with EO/IR turrets, and other manned/unmanned platforms carrying stabilized imaging payloads.

**Status:** early development. The `srt-sys` (raw FFI + encryption), `srt-core::srt` (safe `Socket`/`Listener` API), and `srt-core::klv` (MISB ST 0601 typed codec + generic substrate) crates/modules are implemented; the binding crates and the MPEG-TS muxer are not yet started.

## Scope (v0)

- Container: **MPEG-TS**
- Metadata: **MISB ST 0601 KLV** (multiplexed per MISB ST 1402 / ST 1910)
- Transport: **SRT** (Haivision libsrt 1.5.5, vendored)
- Encryption: **mbedTLS 3.6.x LTS** (vendored, statically linked, on by default)

## Architecture

A Rust core wrapping libsrt via FFI, with bindings for JVM (JNI, JDK 17+), iOS/Android (UniFFI), and embedded Linux (cdylib + cbindgen). MPEG-TS demux is deferred — receivers use FFmpeg/JavaCV/Bento4/platform demuxers and feed extracted KLV bytes through this crate's future `klv::Decoder`.

## Workspace layout

```
crates/
  srt-sys/      raw libsrt FFI (bindgen-generated against libsrt 1.5.5)  ✅ done
  srt-core/     safe Rust API — srt:: ✅ done, klv:: ✅ done, mpegts::/pipeline:: planned
  srt-c/        cdylib + cbindgen header — embedded, future Panama/FFM    planned
  srt-jni/      JNI bindings — JAR for JDK 17+ JVM consumers              planned
  srt-uniffi/   Swift/Kotlin via UniFFI — iOS/Android frameworks          planned
vendor/
  srt/          libsrt git submodule, pinned at v1.5.5
  mbedtls/      mbedTLS git submodule, pinned at v3.6.x LTS
```

## Crates

### `srt-sys` — raw FFI bindings to libsrt

Bindgen-generated, edition 2024, MSRV 1.85. Exposes ~72 `srt_*` functions and the full `SRT_*` constant/type surface. Encryption is wired in via mbedTLS by default; opt out with `--no-default-features`.

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

Built on `srt-sys`. Provides `Socket`, `Listener`, `SocketConfig`, `ListenerConfig`, fluent builders, and a per-call-category error model. Sync blocking API in v0; async/reactor are deferred.

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

The `srt-core` crate also includes a KLV codec: a generic substrate (BER/BER-OID lengths,
SMPTE Universal Labels, ST 1201 IMAPB, generic local-set/universal-set pack-and-iterate)
plus a typed MISB ST 0601 layer (`UasDatalinkLs` with ~41 typed tags + escape hatch). See
`crates/srt-core/src/klv/` and the design doc at `docs/specs/2026-04-30-srt-core-klv-design.md`
in the parent workspace.

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
```

A clean rebuild compiles libsrt and mbedTLS from source — expect 3–5 minutes the first time, seconds on warm builds.

### CI

Linux x86_64 CI runs fmt + clippy (`-D warnings`) + the test suite in both feature modes. See [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Project conventions

- **Direct pushes to `main`.** Single-developer linear history; no feature branches by default.
- **Subject-only commit messages** unless the why is non-obvious. No AI-attribution trailers.
- **Submodules pinned by tag** (libsrt v1.5.5, mbedTLS v3.6.x LTS). Submodule advances are deliberate, separate commits.

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
