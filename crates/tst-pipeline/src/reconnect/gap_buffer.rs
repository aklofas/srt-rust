//! Fixed-capacity ring buffer of outbound messages, used by
//! `ManagedTransport` to hold messages that couldn't be sent during a
//! transport outage.
//!
//! Overflow policy is configurable: `DropOldest` (the default) discards
//! the front of the queue to make room; `Reject` returns an error
//! signaling the caller to back off.
//!
//! Drop policy is uniform across all sender types — drop oldest message.
//! The previously-considered drop-oldest-GOP policy is deferred; it would
//! require IDR-boundary metadata in the mux path and byte scanning in
//! the ts/raw paths.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowPolicy {
    /// Drop the oldest queued message to make room (default).
    #[default]
    DropOldest,
    /// Refuse to enqueue; return an error to the caller.
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapBufferError {
    Full,
}

pub struct GapBuffer {
    capacity: usize,
    overflow: OverflowPolicy,
    queue: VecDeque<Vec<u8>>,
    /// Bytes dropped (oldest-first) due to overflow; for stats.
    pub bytes_dropped: u64,
    /// Messages dropped due to overflow.
    pub messages_dropped: u64,
}

impl GapBuffer {
    pub fn new(capacity: usize, overflow: OverflowPolicy) -> Self {
        Self {
            capacity,
            overflow,
            queue: VecDeque::with_capacity(capacity),
            bytes_dropped: 0,
            messages_dropped: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Enqueue a message. Returns Ok if added; `Err(Full)` if the
    /// overflow policy is `Reject` and the buffer is full.
    pub fn enqueue(&mut self, msg: Vec<u8>) -> Result<(), GapBufferError> {
        if self.queue.len() >= self.capacity {
            match self.overflow {
                OverflowPolicy::DropOldest => {
                    if let Some(dropped) = self.queue.pop_front() {
                        self.bytes_dropped += dropped.len() as u64;
                        self.messages_dropped += 1;
                    }
                }
                OverflowPolicy::Reject => return Err(GapBufferError::Full),
            }
        }
        self.queue.push_back(msg);
        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<Vec<u8>> {
        self.queue.pop_front()
    }

    pub fn front(&self) -> Option<&Vec<u8>> {
        self.queue.front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_oldest_policy_evicts_front() {
        let mut buf = GapBuffer::new(2, OverflowPolicy::DropOldest);
        buf.enqueue(vec![1]).unwrap();
        buf.enqueue(vec![2]).unwrap();
        buf.enqueue(vec![3]).unwrap();
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.pop_front().unwrap(), vec![2]);
        assert_eq!(buf.pop_front().unwrap(), vec![3]);
        assert_eq!(buf.messages_dropped, 1);
        assert_eq!(buf.bytes_dropped, 1);
    }

    #[test]
    fn reject_policy_returns_error_when_full() {
        let mut buf = GapBuffer::new(1, OverflowPolicy::Reject);
        buf.enqueue(vec![1]).unwrap();
        let result = buf.enqueue(vec![2]);
        assert_eq!(result, Err(GapBufferError::Full));
        assert_eq!(buf.messages_dropped, 0);
    }

    #[test]
    fn fifo_order_preserved() {
        let mut buf = GapBuffer::new(10, OverflowPolicy::DropOldest);
        for i in 0..5 {
            buf.enqueue(vec![i]).unwrap();
        }
        for i in 0..5 {
            assert_eq!(buf.pop_front().unwrap(), vec![i]);
        }
        assert!(buf.is_empty());
    }
}
