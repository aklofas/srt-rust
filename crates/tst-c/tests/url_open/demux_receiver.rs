//! C-ABI URL parsing tests for `tst_demux_receiver_*` (plain + managed).
//!
//! Placeholder: receiver-side `_open` entry points accept the same SRT URL
//! grammar (parsed by `tst_srt::SrtUrl::parse`, which already round-trips
//! mode=listener — see `url_mode_listener_parse_accepted` in `ts_sender.rs`).
//! Dedicated demux-receiver URL roundtrip tests live in
//! `tests/demux_receiver_loopback.rs`; this file is reserved for future
//! URL-specific demux-receiver assertions.
