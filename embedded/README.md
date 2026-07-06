# `embedded/` — bare-metal & RTOS support

Self-contained embedded sub-project: firmware-side proofs and reference ports
that exercise the workspace's `no_std` surface on microcontroller targets. All
of it runs under QEMU — no hardware required.

> **Most consumers do not need this directory.** If you run on an OS, use the
> Rust crates or the C/Python/JVM bindings. This tree is for the narrow case of
> running the MPEG-TS/KLV core — and optionally full SRT — directly in
> bare-metal or FreeRTOS firmware.

## Sub-projects

| Directory | What it proves | Stack |
|---|---|---|
| `baremetal-qemu/` | the `tst-core` muxer + `tst-pipeline` `MuxSender` run `no_std` on a Cortex-M4 and byte-match the committed golden, including smoltcp UDP loopback egress | Rust `no_std`, QEMU `mps2-an386` |
| `baremetal-qemu-c/` | the offline C ABI (`tst-c-core`) works from C firmware via a `no_std` staticlib (`libtstrans_firmware.a`) | Rust staticlib + arm-none-eabi C firmware, QEMU |
| `freertos-srt/` | **reference product**: libsrt + the muxer on FreeRTOS + FreeRTOS-Plus-POSIX + lwIP — SRT video egress from a microcontroller, plain and AES-128 | C/C++ substrate, arm-none-eabi GCC, QEMU |

## Layout

- `vendor/` — embedded-only submodules: `freertos-kernel`, `freertos-plus-posix`,
  `lwip`. (The shared `srt` and `mbedtls` submodules stay at the workspace-level
  `vendor/` — `srt-sys` builds them for host targets too.)
- `scripts/check/` — the embedded CI gate scripts (below).
- `scripts/lib/` — helpers shared by the gates.

The two `baremetal-qemu*` projects are excluded from the cargo workspace (they
pin bare-metal targets and profiles) and consume the workspace crates by path
(`crates/tst-core`, `crates/tst-pipeline`, `bindings/c/core`).

## Prerequisites

```bash
sudo apt install qemu-system-arm gcc-arm-none-eabi libnewlib-arm-none-eabi \
                 libstdc++-arm-none-eabi-newlib cmake
rustup target add thumbv7em-none-eabihf riscv32imac-unknown-none-elf
```

## Running the gates

All gates run from the workspace root. Missing tools skip cleanly by default;
CI sets `FREERTOS_SRT_REQUIRE_TOOLS=1` to fail closed instead:

```bash
bash embedded/scripts/check/no-std-baremetal.sh   # no_std compile proof (3 crates x 2 targets)
bash embedded/scripts/check/qemu-runtime.sh       # baremetal-qemu golden byte-match under QEMU
bash embedded/scripts/check/firmware-qemu.sh      # C firmware via libtstrans_firmware.a
bash embedded/scripts/check/freertos-srt.sh exceptions     # C++ exceptions on FreeRTOS
bash embedded/scripts/check/freertos-srt.sh lwip-loopback  # lwIP UDP loopback round-trip
bash embedded/scripts/check/freertos-srt.sh libsrt-smoke   # cross-built libsrt boots
bash embedded/scripts/check/freertos-srt.sh loopback-arq   # SRT ARQ + AES-128 over a lossy netif
bash embedded/scripts/check/freertos-srt.sh example        # NIC egress to a host listener
```

Each sub-project's own README covers internals and design rationale.
