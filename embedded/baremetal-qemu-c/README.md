# baremetal-qemu-c

A `no_std` staticlib glue crate that links `tst-c-core`'s offline C ABI surface
(muxer, demuxer, config, error, event, stats) into a single archive
`libtstrans_firmware.a` for use in the C-firmware QEMU embedded test. It
provides the three things `tst-c-core` deliberately omits: a `#[global_allocator]`
forwarding to the C firmware's newlib heap (`memalign`/`free`), a
`#[panic_handler]` that calls `abort()`, and the `critical-section` single-core
implementation that the no_std last-error layer needs. Built and exercised by
`embedded/scripts/check/firmware-qemu.sh`. The gate byte-matches the muxer output against the committed golden, then
demuxes it back and validates the typed event structs (`tst_event_t`,
`tst_stream_info_t`, `tst_nal_t`) field-by-field — runtime struct-crossing
coverage for the 32-bit ABI layout pins, which compile enabled (no
`-DTST_SKIP_ABI_ASSERTS`). This crate is workspace-excluded and
carries its own committed `Cargo.lock`.
