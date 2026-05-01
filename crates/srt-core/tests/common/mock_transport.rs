//! Test-only mock `Transport` that records sent messages and can be
//! programmed to fail in deterministic patterns. Used across pipeline_*
//! integration tests.

use srt_core::pipeline::{Transport, TransportError};
use std::sync::{Arc, Mutex};

pub struct MockTransport {
    max_payload: usize,
    closed: bool,
    log: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MockTransport {
    pub fn new(max_payload: usize) -> Self {
        Self {
            max_payload,
            closed: false,
            log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Shared handle for inspecting captured sends from the test thread.
    pub fn log(&self) -> Arc<Mutex<Vec<Vec<u8>>>> {
        Arc::clone(&self.log)
    }
}

impl Transport for MockTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if self.closed {
            return Err(TransportError::Closed);
        }
        if msg.len() > self.max_payload {
            return Err(TransportError::TooLarge {
                len: msg.len(),
                max: self.max_payload,
            });
        }
        self.log.lock().unwrap().push(msg.to_vec());
        Ok(())
    }

    fn max_payload(&self) -> usize {
        self.max_payload
    }

    fn is_alive(&self) -> bool {
        !self.closed
    }

    fn close(&mut self) {
        self.closed = true;
    }
}
