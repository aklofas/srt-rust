//! `tst_tcp_*` C ABI entry points. Gated on `feature = "tcp"`.
//!
//! Exposes constructors for the dual-trait `TcpTransport` (impls
//! both `Transport` + `RecvTransport`; role chosen by which pipeline
//! shell consumes it) plus the `TcpListener` accept loop. `tcps://`
//! TLS callers go through the same constructors; the URL scheme picks
//! the TLS path.
