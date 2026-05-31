# baremetal-qemu

`no_std` runtime smoke test for the `tst-core` muxer. Runs the
`video-roundtrip` mux sequence on a Cortex-M4 under QEMU and asserts the
output byte-matches the committed golden
(`../tst-integration/tests/fixtures/scenarios/video-roundtrip/output.ts`).

This is the runtime counterpart to the `no_std` *compile* gate
(`scripts/check-no-std-baremetal.sh`): the compile gate proves `tst-core`
builds for bare metal; this proves it *runs* there (heap, panic, codegen).

## Why it is excluded from the workspace

It is a `#![no_std] #![no_main]` `cortex-m-rt` binary that cannot link for the
host target, so it is in the root `Cargo.toml` `[workspace] exclude` list and
has its own `.cargo/config.toml` pinning the `thumbv7em-none-eabihf` target
and the QEMU runner.

## Run locally

    sudo apt install qemu-system-arm   # one-time
    rustup target add thumbv7em-none-eabihf
    cd crates/baremetal-qemu && cargo run

A `PASS` line + exit 0 means the on-device muxer reproduced the golden. Or run
the repo-root mirror, which skips cleanly without QEMU:

    bash scripts/check-qemu-runtime.sh

## Scope

v1 verifies the raw `tst-core` muxer only. The next milestone brings
`tst-pipeline` to `no_std` and adds a `MuxSender`-over-mock-`Transport`
scenario here (see the design spec's Sequencing section).
