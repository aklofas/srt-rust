# baremetal-qemu-c

A `no_std` staticlib glue crate that links `tst-c-core`'s offline C ABI surface
(muxer, demuxer, config, error, event, stats) into a single archive
`libtstrans_firmware.a` for use in the C-firmware QEMU embedded test. It
provides the three things `tst-c-core` deliberately omits: a `#[global_allocator]`
forwarding to the C firmware's newlib heap (`memalign`/`free`), a
`#[panic_handler]` that calls `abort()`, and the `critical-section` single-core
implementation that the no_std last-error layer needs. Built and exercised by
`embedded/scripts/check/firmware-qemu.sh`. This crate is workspace-excluded and
carries its own committed `Cargo.lock`.
