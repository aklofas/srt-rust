//! Crate-private libsrt initialization. Filled in Task 7.

use std::sync::OnceLock;

#[allow(dead_code)]
static SRT_INITIALIZED: OnceLock<()> = OnceLock::new();
