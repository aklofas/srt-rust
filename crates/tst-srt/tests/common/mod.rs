//! Shared scaffolding for tst-srt integration tests.
//! Loopback only; no external network.

#![allow(dead_code)]

use std::time::Duration;

pub mod mock_transport;
pub mod synthetic_nal;
pub mod ts_parser;

/// Small wall-clock pause to give a listener thread time to enter accept().
pub fn settle() {
    std::thread::sleep(Duration::from_millis(100));
}
