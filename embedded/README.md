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
bash embedded/scripts/check/freertos-srt.sh fault-smoke    # deliberate fault produces labeled FAIL token + fast exit (gate asserts the failure)
bash embedded/scripts/check/freertos-srt.sh malloc-stress  # 4 tasks × 20000 malloc/free + EH + errno isolation
```

Each sub-project's own README covers internals and design rationale.

## Fatal-path diagnostics

Every fatal path in `freertos-srt/` prints a labeled token and exits non-zero
via semihosting, rather than hanging silently. Fault handlers (HardFault /
UsageFault / BusFault / MemManage) emit `FAIL[hardfault] pc=0x…` including the
stacked PC and LR; `configASSERT` fires `FAIL[assert]` with file and line;
task/thread creation failures print `FAIL[task-…]` and exit immediately.
All output uses direct ARM semihosting `SYS_WRITE0` — no newlib stdio, no heap,
no locks — so the path is safe from fault context and from pre-scheduler
initialisation. Because QEMU routes semihosting writes to stderr, the gate
scripts fold stderr into the captured transcript (`2>&1`), and the `fault-smoke`
gate asserts the expected `FAIL[hardfault]` token rather than a `PASS` token.

## Newlib locking and per-task reentrancy

`freertos-srt/` enables `configUSE_NEWLIB_REENTRANT = 1` so FreeRTOS allocates a
separate `struct _reent` for each task. This makes `errno` and the C library's
internal file-pointer state task-local, eliminating cross-task bleed under
preemption. The xpack `arm-none-eabi` newlib is built with `_RETARGETABLE_LOCKING`:
the library references `__retarget_lock_*` symbols and ships no-op archive
fallbacks. `substrate/newlib_lock.c` provides strong definitions that back each
lock with a FreeRTOS recursive mutex, so `malloc`/`free`, `stdio`, and `env`
become fully preemption-safe. Before `vTaskStartScheduler()`, the scheduler is
not running and all acquire/release operations are no-ops — the same pattern as
`pthread_key_shim.c`.
