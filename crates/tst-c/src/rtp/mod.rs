//! `tst_rtp_*` C ABI entry points. Gated on `feature = "rtp"`.
//!
//! This module exposes constructors that open RTP transports and
//! return the existing tst_*_handle types — once open, callers use
//! the same send/recv entry points as SRT-backed handles.
//!
//! Entry points land in Task 3 (Wave A). This stub satisfies the
//! module declaration in `lib.rs` for the bootstrap commit.
