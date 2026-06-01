//! no_std glue staticlib for the C-firmware QEMU embedded test.
//!
//! Links `tst-c-core`'s offline C ABI (built `--no-default-features`) into a
//! single `libtstrans_firmware.a` that a C firmware (`firmware/main.c`) links.
//! This is the ONLY place allocator + panic + critical-section policy lives —
//! `tst-c-core` itself stays agnostic. The allocator forwards to the C
//! firmware's newlib heap (`memalign`/`free`), so Rust and C share one heap.
#![no_std]

extern crate alloc;

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

// Force the offline tst-c-core #[no_mangle] symbols to be reachable from the
// staticlib root so they are retained + exported (rustc only guarantees export
// of reachable no_mangle symbols from a staticlib).
pub use tst_c_core::config::*;
pub use tst_c_core::demux_config::*;
pub use tst_c_core::demuxer::*;
pub use tst_c_core::error::*;
pub use tst_c_core::event::*;
pub use tst_c_core::muxer::*;
pub use tst_c_core::stats::*;
pub use tst_c_core::{TST_ABI_VERSION_MINOR, tst_get_abi_version_minor};

// Register the critical-section impl (single-core) tst-c-core's no_std
// last-error depends on. `use ... as _` forces cortex-m to be linked.
use cortex_m as _;

extern "C" {
    fn memalign(align: usize, size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
    fn abort() -> !;
}

struct CAlloc;

// SAFETY: forwards to newlib's memalign/free. memalign honors any power-of-two
// alignment; flooring to 8 keeps tiny allocations on newlib's natural boundary.
unsafe impl GlobalAlloc for CAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { memalign(layout.align().max(8), layout.size()) as *mut u8 }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe { free(ptr as *mut c_void) }
    }
}

#[global_allocator]
static ALLOC: CAlloc = CAlloc;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // No unwinding on bare metal — a library panic is fatal. abort() routes
    // through newlib -> _exit -> semihosting, turning the QEMU run RED.
    unsafe { abort() }
}
