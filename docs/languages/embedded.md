# Embedded (no_std / bare-metal)

> **Who this is for:** You are building firmware that runs without an OS, or on a real-time OS like FreeRTOS, and need the MPEG-TS/KLV core — or full SRT video egress — inside the microcontroller, not in a host process.

> **You will learn:**
> - The three consumption paths (no_std Rust, C staticlib, FreeRTOS reference port)
> - Cargo snippet for pulling in the core without the standard library
> - What is and is not QEMU-gated in CI
> - The non-obvious gotchas before you start

## Overview

ts-transformer has three embedded consumption paths:

| Path | What you get | Reference |
|---|---|---|
| **no_std Rust** | `tst-core` + `tst-pipeline` sender shells, `#![no_std]` + `alloc`, compile-gated for two bare-metal targets | [`embedded/baremetal-qemu/`](/embedded/baremetal-qemu/) |
| **C firmware staticlib** | Offline mux/demux C ABI (the `tst-c-core` crate) bundled as `libtstrans_firmware.a` | [`embedded/baremetal-qemu-c/`](/embedded/baremetal-qemu-c/) |
| **FreeRTOS reference port** | Full libsrt + the MPEG-TS muxer on FreeRTOS + FreeRTOS-Plus-POSIX + lwIP — SRT video egress from a microcontroller, plain and AES-128 | [`embedded/freertos-srt/`](/embedded/freertos-srt/) |

All three run under QEMU (`mps2-an386`, Cortex-M4) — no vendor hardware or BSPs are required or provided. See [`/docs/reference/compatibility.md`](/docs/reference/compatibility.md) for the platform-support tiers.

## Path 1 — no_std Rust

Pull in `tst-core` and the pipeline sender shells with the standard library disabled:

```toml
[dependencies]
tst-core     = { version = "0.2.0", default-features = false }
tst-pipeline = { version = "0.2.0", default-features = false }
```

`default-features = false` drops the `std` feature on both crates. What you get:

- `tst-core` — the full MPEG-TS mux/demux engine, KLV substrate and MISB typed sets, in-crate codec parameter-set parsers, and the `Transport`/`RecvTransport` traits. The `std` feature (default-on) gates the `net` helpers, file I/O, and `serde_json`/`toml` serialization — none of that is present in a `no_std` build.
- `tst-pipeline` sender path — `MuxSender`, `Sender`, and `RawSender`. The receiver shells (`Receiver`, `DemuxReceiver`, `RawReceiver`), the `Managed*` reconnect wrappers, and `ext::pairing` require `std` and are not available in a `no_std` build.

### QEMU reference

[`embedded/baremetal-qemu/`](/embedded/baremetal-qemu/) is the runnable reference. It exercises three checks on a Cortex-M4 under QEMU:

1. Bare `Muxer` — mux the `video-roundtrip` fixture and byte-compare to the committed golden.
2. `MuxSender` over an in-memory `Vec`-backed transport — same golden, proves the pipeline shell runs on bare metal.
3. `MuxSender` through a real `no_std` smoltcp IPv4/UDP stack over a loopback device.

```bash
# Run locally (needs qemu-system-arm + thumbv7em-none-eabihf target):
cd embedded/baremetal-qemu && cargo run --release --locked

# Or via the CI gate script (skips cleanly if QEMU is absent):
bash embedded/scripts/check/qemu-runtime.sh
```

### Gotchas

- **`std` is on by default.** A bare `tst-core = { version = "0.2.0" }` pulls in the standard library. Always add `default-features = false` for bare-metal targets — the compiler will reject the build if you forget (the `*-none-*` target has no `std`).
- **Final binaries need a `#[global_allocator]`.** Both crates are `#![no_std]` + `alloc`, which requires a heap. Library crates that consume `tst-core` do not need to provide one; the final binary (your firmware) does. `embedded/baremetal-qemu/` uses `embedded-alloc` + `cortex-m-rt` as one approach.
- **The `embedded/` tree is workspace-excluded on purpose.** The sub-projects pin bare-metal targets and profiles that are incompatible with a host-target workspace build. They consume the workspace crates by path and carry their own committed `Cargo.lock` files; CI builds them with `--locked`.

## Path 2 — C firmware staticlib

[`embedded/baremetal-qemu-c/`](/embedded/baremetal-qemu-c/) shows how to link the offline C ABI surface into C firmware. It builds `tst-c-core` as a `no_std` staticlib (`libtstrans_firmware.a`) and provides the three embedder-supplied pieces that `tst-c-core` deliberately omits:

- `#[global_allocator]` forwarding to the C firmware's newlib heap (`memalign`/`free`)
- `#[panic_handler]` calling `abort()`
- The `critical-section` single-core implementation

The resulting archive exposes the offline mux/demux C ABI — config, push, pull, event, error — usable from plain C firmware with no Rust toolchain in the firmware build system itself (only in the glue-crate build step).

Network transports (SRT, RTP, UDP, TCP) are host-only; they require `std` and are not available through `tst-c-core`.

```bash
bash embedded/scripts/check/firmware-qemu.sh
```

## Path 3 — FreeRTOS reference port

[`embedded/freertos-srt/`](/embedded/freertos-srt/) is the flagship: a complete reference port of **libsrt** + the MPEG-TS muxer onto FreeRTOS + FreeRTOS-Plus-POSIX + lwIP. It demonstrates SRT video egress from a microcontroller, including byte-exact ARQ recovery under ~20% packet loss and AES-128 encryption.

It is **not built by default** — it is not a Cargo workspace member, and its gates skip locally when the cross-toolchain or QEMU is absent. In CI, every `freertos-srt` target runs as a fail-closed hard gate under QEMU.

```bash
# From the workspace root:
bash embedded/scripts/check/freertos-srt.sh loopback-arq        # SRT byte-exact + AES-128
bash embedded/scripts/check/freertos-srt.sh malloc-stress       # 4 tasks × 20000 malloc/free + per-task errno isolation
bash embedded/scripts/check/freertos-srt.sh fault-smoke         # deliberate fault → labeled FAIL + fast exit
```

Gate targets:

| Target | Proves |
|---|---|
| `exceptions` | Per-task C++ exceptions isolated on FreeRTOS |
| `lwip-loopback` | FreeRTOS + lwIP substrate round-trips a UDP golden |
| `libsrt-smoke` | Cross-compiled libsrt boots on the substrate |
| `loopback-arq` | SRT recovers byte-exact under ~20% loss, plain and AES-128 |
| `arq-connfail` | Caller at a dead port fails fast with a labeled verdict |
| `fault-smoke` | HardFault/assert prints labeled `FAIL[…]` and exits, not hangs |
| `malloc-stress` | 4 tasks × 20000 malloc/free + concurrent exception handling + per-task errno isolation |
| `example` | Real-NIC SRT caller streams the golden to a host listener, plain and AES-128 |

### Production crypto warning

> **The AES-128 path in this reference port uses _deterministic_ entropy.** The `ENCRYPT=1` builds wire mbedTLS, but the entropy hooks are a fixed-seed LCG chosen for QEMU/CI reproducibility. **This is not cryptographically secure.** Before enabling SRT encryption in production firmware, replace both hooks with a hardware RNG or your board's approved entropy source. See the warning in [`/embedded/freertos-srt/README.md`](/embedded/freertos-srt/README.md) for exactly which hooks to replace.

## Prerequisites

```bash
git submodule update --init --recursive   # embedded/vendor/* + vendor/{srt,mbedtls}
sudo apt install qemu-system-arm gcc-arm-none-eabi libnewlib-arm-none-eabi \
                 libstdc++-arm-none-eabi-newlib cmake python3
rustup target add thumbv7em-none-eabihf riscv32imac-unknown-none-elf
```

## Where to go next

- [`/embedded/README.md`](/embedded/README.md) — sub-project layout, all gate commands, and fatal-path diagnostics summary.
- [`/embedded/baremetal-qemu/README.md`](/embedded/baremetal-qemu/README.md) — Rust no_std runtime reference, check sequence.
- [`/embedded/baremetal-qemu-c/README.md`](/embedded/baremetal-qemu-c/README.md) — C staticlib glue reference.
- [`/embedded/freertos-srt/README.md`](/embedded/freertos-srt/README.md) — FreeRTOS reference port internals and newlib locking.
- [`/docs/reference/compatibility.md`](/docs/reference/compatibility.md) — platform-support tiers, including the bare-metal Tier 2 entries.
