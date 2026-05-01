//! Test-only mock `Transport` that records sent messages and can be
//! programmed to fail in deterministic patterns. Used across pipeline_*
//! integration tests.

use srt_core::pipeline::{Transport, TransportError};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum FailMode {
    /// Always succeed (default).
    Never,
    /// Return Broken on the next N sends, then succeed.
    BrokenForN(usize),
    /// Return Backpressure on the next N sends, then succeed.
    BackpressureForN(usize),
}

pub struct MockTransport {
    max_payload: usize,
    closed: bool,
    log: Arc<Mutex<Vec<Vec<u8>>>>,
    fail_mode: Arc<Mutex<FailMode>>,
}

impl MockTransport {
    pub fn new(max_payload: usize) -> Self {
        Self {
            max_payload,
            closed: false,
            log: Arc::new(Mutex::new(Vec::new())),
            fail_mode: Arc::new(Mutex::new(FailMode::Never)),
        }
    }

    pub fn log(&self) -> Arc<Mutex<Vec<Vec<u8>>>> {
        Arc::clone(&self.log)
    }

    pub fn fail_handle(&self) -> Arc<Mutex<FailMode>> {
        Arc::clone(&self.fail_mode)
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
        let mut mode = self.fail_mode.lock().unwrap();
        match &mut *mode {
            FailMode::BrokenForN(n) if *n > 0 => {
                *n -= 1;
                return Err(TransportError::Broken("mock broken".into()));
            }
            FailMode::BackpressureForN(n) if *n > 0 => {
                *n -= 1;
                return Err(TransportError::Backpressure("mock backpressure".into()));
            }
            _ => {}
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
