//! Program-level mux config entry points.
//!
//! Houses `tst_mux_config_add_program`, the only entry point that pushes a
//! new `MuxerProgramConfig` onto the in-progress mux config. All subsequent
//! stream-add and descriptor-set entries (which live in sibling modules)
//! index into the `programs` vec by the `TstProgramHandle` returned here.

use super::{TST_INVALID_PROGRAM_HANDLE, TstMuxConfig, TstProgramHandle};
use crate::error::{TstError, set_last_error};
use crate::panic::ffi_catch;
use tst_core::mpegts::mux::MuxerProgramConfig;

/// Begin a new program in this multiplex. Returns a handle used as the
/// `program` argument to subsequent stream-add and descriptor-set entry
/// points. Programs are numbered in insertion order starting at 0.
///
/// `program_number` is the PAT program_number field (must be > 0 and unique
/// within the config). `pmt_pid` is the PID on which this program's PMT will
/// be carried (must be unique within the config and not collide with any
/// stream PID).
///
/// Returns `TST_INVALID_PROGRAM_HANDLE` and sets last-error on null `cfg`.
/// Validation (duplicate program_number, colliding PMT PID, etc.) is deferred
/// to `tst_muxer_open` / `tst_*_sender_open` time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_program(
    cfg: *mut TstMuxConfig,
    program_number: u16,
    pmt_pid: u16,
) -> TstProgramHandle {
    ffi_catch(TST_INVALID_PROGRAM_HANDLE, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TST_INVALID_PROGRAM_HANDLE;
        };
        cfg.programs
            .push(MuxerProgramConfig::new(program_number, pmt_pid));
        TstProgramHandle((cfg.programs.len() - 1) as u32)
    })
}
