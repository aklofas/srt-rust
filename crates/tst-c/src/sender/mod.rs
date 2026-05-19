//! Sender-side C ABI surface — write-side entry points.
//!
//! Submodules:
//! - [`mux_sender`] — `tst_mux_sender_t` + `tst_managed_mux_sender_t`.
//! - [`muxer`]      — standalone `tst_muxer_t` (no transport).
//! - [`raw_sender`] — `tst_raw_sender_t` + `tst_managed_raw_sender_t`.
//! - [`ts_sender`]  — `tst_sender_t` + `tst_managed_sender_t` (TS-bytes-in).
//!
//! `connect` is `pub(crate)`-internal — connect-side boilerplate shared
//! by mux/ts/raw senders.

pub mod mux_sender;
pub mod muxer;
pub mod raw_sender;
pub mod ts_sender;
pub(crate) mod connect;
