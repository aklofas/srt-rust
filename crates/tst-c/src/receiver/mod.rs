//! Receiver-side C ABI surface — read-side entry points.
//!
//! Submodules:
//! - [`demux_receiver`] — `tst_demux_receiver_t` (split into 4 sub-files).
//! - [`raw_receiver`]   — `tst_raw_receiver_t` + `tst_managed_raw_receiver_t`.
//! - [`ts_receiver`]    — `tst_receiver_t` + `tst_managed_receiver_t`.
//!
//! `listen` is `pub(crate)`-internal — listener-side boilerplate shared
//! by raw/ts/demux receivers.

pub mod demux_receiver;
pub mod raw_receiver;
pub mod ts_receiver;
pub(crate) mod listen;
