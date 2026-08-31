# baremetal-qemu

`no_std` runtime smoke test for the `tst-core` muxer + `tst-pipeline` shells.
Runs the `video-roundtrip` mux sequence — plus a `no_std` receiver round-trip
— under QEMU on **two architectures**, ARM (Cortex-M4) and RISC-V, and asserts
the output byte-matches the committed golden
(`../../crates/tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts`).
Passing on both is the "arch-agnostic no_std" proof: the same source runs
unmodified on two unrelated instruction sets.

This is the runtime counterpart to the `no_std` *compile* gate
(`embedded/scripts/check/no-std-baremetal.sh`): the compile gate proves the
library crates build for bare metal; this proves the muxer/pipeline shells
*run* there (heap, panic, codegen) on real target hardware emulation.

## Why it is excluded from the workspace

It is a `#![no_std] #![no_main]` binary (`cortex-m-rt` on ARM, `riscv-rt` on
RISC-V) that cannot link for the host target, so it is in the root
`Cargo.toml` `[workspace] exclude` list and has its own `.cargo/config.toml`
pinning both target triples and their QEMU runners. `cargo run`/`cargo build`
with no `--target` builds ARM (the `[build] target` default); pass
`--target riscv32imac-unknown-none-elf` for the RISC-V leg.

## Run locally

    sudo apt install qemu-system-arm qemu-system-misc   # one-time
    rustup target add thumbv7em-none-eabihf riscv32imac-unknown-none-elf
    cd embedded/baremetal-qemu
    cargo run --release --locked                                       # ARM
    cargo run --release --locked --target riscv32imac-unknown-none-elf # RISC-V

A `PASS` line + exit 0 (on each arch) means the on-device muxer/pipeline
reproduced the golden. Or run the repo-root mirror, which runs both targets
and skips cleanly (per-target) without the matching QEMU binary:

    bash embedded/scripts/check/qemu-runtime.sh

## Two architectures, one source

`Cargo.toml` splits the ARM-only (`cortex-m-rt`/`cortex-m`/`cortex-m-semihosting`/
`panic-semihosting`) and RISC-V-only (`riscv-rt`/`riscv`/`riscv-semihosting`)
runtime crates into `[target.'cfg(target_arch = "...")'.dependencies]`
sections; `src/main.rs` cfg's the handful of entry-point/semihosting-output/
panic-handler symbols that differ by name between the two runtimes (`entry`,
`debug`, `hprintln`), with everything else — including all four checks below
— shared verbatim. `build.rs` picks the matching linker memory script
(`memory-arm.x` / `memory-riscv.x`) per `$TARGET` and copies it into
`OUT_DIR` as `memory.x`, which both `cortex-m-rt`'s and `riscv-rt`'s generated
`link.x` pull in via `INCLUDE memory.x`. The two source files are
deliberately **not** named `memory.x` in the crate root: GNU ld's (and
rust-lld's `-flavor gnu`) `INCLUDE` directive searches the linker's *working
directory* (the crate root cargo invokes it from) before it searches `-L`
paths, so a literal `memory.x` sitting in the crate root would silently win
over the per-target `OUT_DIR` copy for every target — see the comment at the
top of `build.rs` for the full story (this is exactly the bug hit and fixed
while bringing up the RISC-V leg).

## Checks

Four on-device checks run in sequence; a failure in any one exits with a
non-zero code and prints a `FAIL[…]` line identifying which check failed.

1. **`muxer`** — drives the bare `tst-core` `Muxer` directly and byte-compares
   the output to the `video-roundtrip` golden (ROADMAP P7 part (a)).

2. **`mux_sender`** — drives `MuxSender` over an in-memory `Vec`-backed
   `Transport` (`Sink`) and byte-compares the accumulated output to the same
   golden. Proves the `tst-pipeline` shell works end-to-end on bare metal
   (ROADMAP P7 part (b)).

3. **`udp_loopback`** — drives `MuxSender` through a real `no_std` smoltcp
   IPv4/UDP stack over a `phy::Loopback` device; each TS chunk is sent as a UDP
   datagram, looped back, recovered, and the concatenated payload is
   byte-compared to the same `video-roundtrip` golden. Proves the
   transport-off-MCU seam at runtime (ROADMAP P7 part (c)).

4. **`udp_recv`** — drives a `no_std` `DemuxReceiver` over its own dedicated
   smoltcp loopback `RecvTransport` and recovers the pushed video AU(s)
   byte-exact. Proves the receiver-path `no_std` shells (`Receiver`/
   `DemuxReceiver`) run correctly on target — the runtime counterpart to the
   receiver-path `no_std` compile gate. See `src/recv.rs` for why this check
   pushes 3 AUs rather than reusing check 3's single-AU golden (the
   `Receiver`'s sync-lock heuristic needs more confirming TS packets than
   that golden has).
