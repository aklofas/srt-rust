// crates/srt-core/src/pipeline/reconnect/mod.rs
//! `ManagedTransport<T>` — Transport decorator with reconnect + gap buffer.
//!
//! Wraps any inner Transport (most commonly `SrtTransport`); on send
//! failure with `Broken` semantics, queues the bytes in a fixed-size
//! gap buffer and attempts to re-establish the inner transport with
//! configurable backoff. On reconnect success, drains the gap buffer
//! before resuming new sends.

mod gap_buffer;

pub use gap_buffer::{GapBuffer, OverflowPolicy};

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackoffStrategy {
    /// Fixed wait between attempts.
    Constant(Duration),
    /// Exponential: wait = base * 2^(attempt-1), capped at max.
    Exponential { base: Duration, max: Duration },
}

impl Default for BackoffStrategy {
    fn default() -> Self {
        BackoffStrategy::Exponential {
            base: Duration::from_millis(100),
            max: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReconnectPolicy {
    /// Maximum reconnect attempts before giving up. None = retry forever.
    /// Default: `Some(10)`.
    pub max_attempts: Option<u32>,

    /// Backoff strategy between attempts. Default: exponential 100ms..=10s.
    pub backoff: BackoffStrategy,

    /// Gap-buffer capacity in messages. Default 256.
    pub gap_buffer_capacity: usize,

    /// What to do when gap buffer is full and a new message arrives.
    /// Default: drop oldest message.
    pub overflow_policy: OverflowPolicy,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            max_attempts: Some(10),
            backoff: BackoffStrategy::default(),
            gap_buffer_capacity: 256,
            overflow_policy: OverflowPolicy::DropOldest,
        }
    }
}

// Filled in by Task 9.
pub struct ManagedTransport<T: crate::pipeline::Transport> {
    _inner: T,
}
