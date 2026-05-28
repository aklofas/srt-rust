//! `tst_hls_*` + `tst_mux_publisher_*` C ABI entry points. Gated on
//! `feature = "hls"`.
//!
//! Exposes the HLS publisher (builder + concrete `HlsPublisher` handle
//! that runs an internal tokio HTTP server) and the new
//! `MuxPublisher<P>` shell projection over `tst_core::publisher::Publisher`.
//! KLV stays inside the .ts segments — no WebVTT sidecar, no
//! EXT-X-DATERANGE metadata in v1.
