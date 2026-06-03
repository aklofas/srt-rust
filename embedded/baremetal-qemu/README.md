# baremetal-qemu

`no_std` runtime smoke test for the `tst-core` muxer. Runs the
`video-roundtrip` mux sequence on a Cortex-M4 under QEMU and asserts the
output byte-matches the committed golden
(`../tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts`).

This is the runtime counterpart to the `no_std` *compile* gate
(`scripts/check/embedded/no-std-baremetal.sh`): the compile gate proves `tst-core`
builds for bare metal; this proves it *runs* there (heap, panic, codegen).

## Why it is excluded from the workspace

It is a `#![no_std] #![no_main]` `cortex-m-rt` binary that cannot link for the
host target, so it is in the root `Cargo.toml` `[workspace] exclude` list and
has its own `.cargo/config.toml` pinning the `thumbv7em-none-eabihf` target
and the QEMU runner.

## Run locally

    sudo apt install qemu-system-arm   # one-time
    rustup target add thumbv7em-none-eabihf
    cd embedded/baremetal-qemu && cargo run

A `PASS` line + exit 0 means the on-device muxer reproduced the golden. Or run
the repo-root mirror, which skips cleanly without QEMU:

    bash scripts/check/embedded/qemu-runtime.sh

## Checks

Three on-device checks run in sequence; a failure in any one exits with a
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
