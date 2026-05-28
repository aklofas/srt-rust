//! `tst_udp_*` C ABI entry points. Gated on `feature = "udp"`.
//!
//! Exposes constructors that open UDP transports (unicast + multicast,
//! IPv4 + IPv6) and return the existing `tst_*_handle` types — once
//! open, callers use the same send/recv entry points as SRT-backed and
//! RTP-backed handles.
