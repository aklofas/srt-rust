//! Network primitives shared across transport crates.
//!
//! This module hosts low-level helpers used by `tst-rtp`, `tst-udp`, and
//! future transport crates. Public for binding-crate use; not part of
//! the user-facing API.

pub mod udp_socket;
