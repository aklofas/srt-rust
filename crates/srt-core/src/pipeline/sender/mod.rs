// crates/srt-core/src/pipeline/sender/mod.rs
//! `Sender<T: Transport>` — pre-muxed TS bytes → SRT, with framing.
//!
//! See `framing.rs` for the sync-acquisition / loss-detection state
//! machine. `Sender` composes `TsFraming` with a `Transport`.

mod framing;

pub use framing::{TsFraming, TsFramingError, TsFramingMode, SenderStats};

use crate::pipeline::transport::Transport;

/// Construction-time knobs for [`Sender`].
#[derive(Debug, Clone)]
pub struct SenderConfig {
    pub framing_mode: TsFramingMode,
    /// Bytes consumed while UNSYNCED before sender enters terminal failed
    /// state. Default 18,800 = 100 packets' worth.
    pub max_unsynced_bytes: usize,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            framing_mode: TsFramingMode::Recover,
            max_unsynced_bytes: 18_800,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error(transparent)]
    Framing(#[from] TsFramingError),
    #[error(transparent)]
    Transport(#[from] crate::pipeline::TransportError),
}

/// Pre-muxed TS bytes → SRT transport with sync framing/recovery.
pub struct Sender<T: Transport> {
    framing: TsFraming,
    transport: T,
    closed: bool,
    mode: TsFramingMode,
}

impl<T: Transport> Sender<T> {
    pub fn new(transport: T, config: SenderConfig) -> Self {
        Self {
            framing: TsFraming::new(config.max_unsynced_bytes),
            transport,
            closed: false,
            mode: config.framing_mode,
        }
    }

    /// Push pre-muxed TS bytes. RECOVER mode silently skips/recovers; in
    /// STRICT mode returns an error on misalignment.
    pub fn send_ts(&mut self, bytes: &[u8]) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(
                crate::pipeline::TransportError::Closed,
            ));
        }
        let bundles = if self.mode == TsFramingMode::Recover {
            let (bundles, _stats) = self.framing.push(bytes);
            bundles
        } else {
            self.framing.push_strict(bytes)?
        };
        for bundle in bundles {
            self.transport
                .send_bytes(&bundle)
                .map_err(SenderError::Transport)?;
        }
        Ok(())
    }

    /// Emit any buffered partial bundle.
    pub fn flush(&mut self) -> Result<(), SenderError> {
        if self.closed {
            return Err(SenderError::Transport(
                crate::pipeline::TransportError::Closed,
            ));
        }
        let bundles = self.framing.flush();
        for bundle in bundles {
            self.transport
                .send_bytes(&bundle)
                .map_err(SenderError::Transport)?;
        }
        Ok(())
    }

    pub fn stats(&self) -> &SenderStats {
        self.framing.stats()
    }

    /// Zero all stats counters. The framing state machine is untouched —
    /// only the counters on top of it.
    pub fn reset_stats(&mut self) {
        self.framing.reset_stats();
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.transport.close();
    }

    pub fn is_alive(&self) -> bool {
        !self.closed && self.transport.is_alive()
    }

    /// Snapshot of the underlying transport's cancel handle. See
    /// [`crate::pipeline::MuxSender::cancel_handle`] for the rationale.
    pub fn cancel_handle(&self) -> Option<Box<dyn crate::pipeline::transport::TransportCancel>> {
        self.transport.cancel_handle()
    }
}

impl<T: Transport> Drop for Sender<T> {
    fn drop(&mut self) {
        if !self.closed {
            // Best-effort flush; ignore errors.
            let _ = self.flush();
            self.transport.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::{Transport, TransportError};

    struct Mem;
    impl Transport for Mem {
        fn send_bytes(&mut self, _: &[u8]) -> Result<(), TransportError> {
            Ok(())
        }
        fn max_payload(&self) -> usize {
            1316
        }
        fn close(&mut self) {}
        fn is_alive(&self) -> bool {
            true
        }
    }

    #[test]
    fn reset_stats_zeros_counters_in_ts_sender() {
        let mut s = Sender::new(Mem, SenderConfig::default());
        // One 188-byte TS packet starting with the sync byte.
        let mut pkt = vec![0x47u8];
        pkt.extend(vec![0u8; 187]);
        s.send_ts(&pkt).unwrap();
        assert!(s.stats().bytes_pushed > 0);
        s.reset_stats();
        let st = s.stats();
        assert_eq!(st.bytes_pushed, 0);
        assert_eq!(st.bytes_skipped_for_sync, 0);
        assert_eq!(st.resync_events, 0);
        assert_eq!(st.packets_sent, 0);
    }
}
