//! Shared scaffolding for tst-core integration tests.
//! Pure parsing only — no network, no SRT, no mock transports.

#![allow(dead_code)]

pub mod synthetic_nal;
pub mod ts_parser;
