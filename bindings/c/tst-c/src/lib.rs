//! `tst-c` — the C ABI artifact crate (`libtstrans.so` / `libtstrans.a` / `tstrans.h`).
//!
//! All C-ABI logic lives in the embeddable `tst-c-core` rlib; this leaf crate
//! exists to produce the cdylib/staticlib + the cbindgen header. Re-exporting
//! `tst_c_core::*` makes every `#[no_mangle]` entry point reachable from this
//! crate root, so it is retained + exported in the cdylib/staticlib.
pub use tst_c_core::*;
