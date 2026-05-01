// crates/srt-core/src/pipeline/ts_sender/mod.rs
//! `TsSender<T: Transport>` — pre-muxed TS bytes → SRT, with framing.
//!
//! See `framing.rs` for the sync-acquisition / loss-detection state
//! machine. `TsSender` itself (defined in Task 7) just composes that
//! with a `Transport`.

mod framing;

pub use framing::{TsFraming, TsFramingError, TsFramingMode, TsSenderStats};

use crate::pipeline::transport::Transport;

/// Construction-time knobs for [`TsSender`].
#[derive(Debug, Clone)]
pub struct TsSenderConfig {
    pub framing_mode: TsFramingMode,
    /// Bytes consumed while UNSYNCED before sender enters terminal failed
    /// state. Default 18,800 = 100 packets' worth.
    pub max_unsynced_bytes: usize,
}

impl Default for TsSenderConfig {
    fn default() -> Self {
        Self {
            framing_mode: TsFramingMode::Recover,
            max_unsynced_bytes: 18_800,
        }
    }
}

/// Pre-muxed TS bytes → SRT transport with sync framing/recovery.
///
/// Filled in by Task 7.
pub struct TsSender<T: Transport> {
    _phantom: std::marker::PhantomData<T>,
}
