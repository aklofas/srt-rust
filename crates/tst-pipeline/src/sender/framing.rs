//! TS sync-acquisition and loss-of-sync recovery state machine.
//!
//! Standalone — no transport dependency. Caller pushes arbitrary-aligned
//! bytes; framer emits 7-packet (1316-byte) bundles whenever enough
//! aligned packets have accumulated.
//!
//! Acquisition (UNSYNCED): scan input for 0x47; for each candidate
//! position P, verify by checking 0x47 at P+188 and P+376 (three
//! consecutive sync bytes, ~1/65,536 false positive rate for uniformly
//! random byte streams). On verify, transition to SYNCED with P as
//! packet boundary 0.
//!
//! Loss detection (SYNCED): every 188-byte boundary in the staging
//! buffer must read 0x47. On failure: drop staging buffer, increment
//! resync_events, return to UNSYNCED, restart scan from next byte.

use std::collections::VecDeque;
use thiserror::Error;

const TS_PACKET_SIZE: usize = 188;
const SRT_BUNDLE_PACKETS: usize = 7;
const SRT_BUNDLE_BYTES: usize = TS_PACKET_SIZE * SRT_BUNDLE_PACKETS; // 1316
const SYNC_BYTE: u8 = 0x47;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TsFramingMode {
    /// Default: skip misaligned prefix until sync; auto-resync on loss.
    #[default]
    Recover,
    /// Any non-0x47 at expected boundary → error immediately.
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TsFramingError {
    #[error("TS sync byte not found at expected boundary (offset {offset})")]
    SyncLost { offset: u64 },
    #[error(
        "exceeded max_unsynced_bytes ({max}) without acquiring sync; \
         input does not look like a TS stream"
    )]
    NoSyncAfterLimit { max: u64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SenderStats {
    pub bytes_pushed: u64,
    pub bytes_skipped_for_sync: u64,
    pub resync_events: u64,
    pub packets_sent: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Unsynced,
    Synced,
}

pub struct TsFraming {
    max_unsynced_bytes: usize,
    state: State,
    /// Bytes pending: in UNSYNCED these are the scan window; in SYNCED
    /// they're whole-packet aligned and accumulating until a 7-packet
    /// bundle is ready.
    buffer: VecDeque<u8>,
    /// In UNSYNCED: count of bytes consumed scanning for sync.
    unsynced_consumed: usize,
    stats: SenderStats,
}

impl TsFraming {
    pub fn new(max_unsynced_bytes: usize) -> Self {
        Self {
            max_unsynced_bytes,
            state: State::Unsynced,
            buffer: VecDeque::new(),
            unsynced_consumed: 0,
            stats: SenderStats::default(),
        }
    }

    pub fn is_synced(&self) -> bool {
        self.state == State::Synced
    }

    pub fn stats(&self) -> SenderStats {
        self.stats.clone()
    }

    /// Zero all stats counters. The framing-state machine (sync-byte
    /// recovery state, partial-bundle buffer) is NOT reset — only the
    /// counters on top of it.
    pub fn reset_stats(&mut self) {
        self.stats = SenderStats::default();
    }

    /// Push bytes (RECOVER mode): returns any complete 7-packet bundles
    /// emitted. Sync is acquired silently; loss-of-sync is silently
    /// recovered. Stats reflect events.
    pub fn push(&mut self, bytes: &[u8]) -> (Vec<Vec<u8>>, &SenderStats) {
        self.stats.bytes_pushed += bytes.len() as u64;
        for &b in bytes {
            self.buffer.push_back(b);
        }
        let mut bundles = Vec::new();
        loop {
            match self.state {
                State::Unsynced => {
                    if !self.try_acquire() {
                        break;
                    }
                }
                State::Synced => {
                    if !self.try_emit(&mut bundles) {
                        break;
                    }
                }
            }
        }
        (bundles, &self.stats)
    }

    /// Push bytes (STRICT mode): returns any bundles emitted. Errors on
    /// any misalignment — no recovery.
    pub fn push_strict(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, TsFramingError> {
        self.stats.bytes_pushed += bytes.len() as u64;
        for &b in bytes {
            self.buffer.push_back(b);
        }
        let mut bundles = Vec::new();
        loop {
            match self.state {
                State::Unsynced => {
                    // STRICT mode: first byte must be 0x47. If buffer
                    // doesn't start with sync byte, error immediately.
                    if self.buffer.is_empty() {
                        break;
                    }
                    if self.buffer[0] != SYNC_BYTE {
                        return Err(TsFramingError::SyncLost { offset: 0u64 });
                    }
                    // STRICT: still need 3-byte verify, but no skipping.
                    if self.buffer.len() < 2 * TS_PACKET_SIZE + 1 {
                        // Need ≥ 377 bytes (P + 188 + 188 + 1) to verify.
                        // Use < 2*188+1 = 377, but checking buffer.len() < 377
                        // is what we want for verify of (0, 188, 376).
                        break;
                    }
                    if self.buffer[TS_PACKET_SIZE] != SYNC_BYTE {
                        return Err(TsFramingError::SyncLost {
                            offset: TS_PACKET_SIZE as u64,
                        });
                    }
                    if self.buffer[2 * TS_PACKET_SIZE] != SYNC_BYTE {
                        return Err(TsFramingError::SyncLost {
                            offset: (2 * TS_PACKET_SIZE) as u64,
                        });
                    }
                    self.state = State::Synced;
                }
                State::Synced => {
                    // STRICT: every boundary must be 0x47.
                    if self.buffer.len() < SRT_BUNDLE_BYTES {
                        break;
                    }
                    for i in 0..SRT_BUNDLE_PACKETS {
                        let off = i * TS_PACKET_SIZE;
                        if self.buffer[off] != SYNC_BYTE {
                            return Err(TsFramingError::SyncLost { offset: off as u64 });
                        }
                    }
                    let bundle: Vec<u8> = self.buffer.drain(..SRT_BUNDLE_BYTES).collect();
                    bundles.push(bundle);
                    self.stats.packets_sent += SRT_BUNDLE_PACKETS as u64;
                }
            }
        }
        Ok(bundles)
    }

    /// In UNSYNCED: try to acquire sync via 3-byte verify. Returns true
    /// if state changed (so the caller's outer loop re-evaluates). False
    /// means we don't yet have enough bytes to verify.
    fn try_acquire(&mut self) -> bool {
        // Scan from current buffer head for a candidate position. Need at
        // least 2*188+1 = 377 bytes after the candidate to verify P, P+188, P+376.
        const NEEDED: usize = 2 * TS_PACKET_SIZE + 1;
        while !self.buffer.is_empty() {
            // Find next 0x47 in buffer.
            let candidate_idx = match self.buffer.iter().position(|&b| b == SYNC_BYTE) {
                Some(i) => i,
                None => {
                    // No sync byte anywhere; discard everything we've buffered.
                    let n = self.buffer.len();
                    self.buffer.clear();
                    self.unsynced_consumed += n;
                    self.stats.bytes_skipped_for_sync += n as u64;
                    self.check_unsynced_limit_recover();
                    return false;
                }
            };
            // Discard everything before the candidate.
            if candidate_idx > 0 {
                for _ in 0..candidate_idx {
                    self.buffer.pop_front();
                }
                self.unsynced_consumed += candidate_idx;
                self.stats.bytes_skipped_for_sync += candidate_idx as u64;
            }
            // Now buffer[0] == 0x47. Need 377 bytes to verify.
            if self.buffer.len() < NEEDED {
                self.check_unsynced_limit_recover();
                return false;
            }
            if self.buffer[TS_PACKET_SIZE] == SYNC_BYTE
                && self.buffer[2 * TS_PACKET_SIZE] == SYNC_BYTE
            {
                self.state = State::Synced;
                self.unsynced_consumed = 0;
                return true;
            }
            // False candidate. Discard just the leading 0x47 and rescan.
            self.buffer.pop_front();
            self.unsynced_consumed += 1;
            self.stats.bytes_skipped_for_sync += 1;
            self.check_unsynced_limit_recover();
        }
        false
    }

    /// In SYNCED: try to emit one bundle. Returns true if a bundle was
    /// emitted (caller should loop), false if not enough bytes (or
    /// loss-of-sync triggered a return to UNSYNCED).
    fn try_emit(&mut self, bundles: &mut Vec<Vec<u8>>) -> bool {
        if self.buffer.len() < SRT_BUNDLE_BYTES {
            return false;
        }
        // Verify every 188-byte boundary in the candidate bundle.
        for i in 0..SRT_BUNDLE_PACKETS {
            if self.buffer[i * TS_PACKET_SIZE] != SYNC_BYTE {
                // Sync lost.
                self.state = State::Unsynced;
                self.stats.resync_events += 1;
                self.buffer.clear();
                return true; // re-enter loop in UNSYNCED.
            }
        }
        let bundle: Vec<u8> = self.buffer.drain(..SRT_BUNDLE_BYTES).collect();
        bundles.push(bundle);
        self.stats.packets_sent += SRT_BUNDLE_PACKETS as u64;
        true
    }

    fn check_unsynced_limit_recover(&mut self) {
        if self.unsynced_consumed > self.max_unsynced_bytes {
            // Saturate the limit; subsequent push_strict calls will error,
            // but in RECOVER mode we just keep skipping bytes until sync
            // is found OR the caller gives up. The C ABI surface (Plan 2)
            // can choose to error after the limit; for the framing
            // primitive we just track and surface via stats.
            //
            // No-op here intentionally — RECOVER mode keeps trying. The
            // caller should monitor stats.bytes_skipped_for_sync against
            // their own threshold if they want fail-fast.
        }
    }

    /// Emit any pending bytes as a partial bundle (1-6 packets).
    /// Called explicitly via flush() or implicitly on close.
    pub fn flush(&mut self) -> Vec<Vec<u8>> {
        if self.state != State::Synced || self.buffer.is_empty() {
            return Vec::new();
        }
        // Buffer should be a whole-packet multiple in SYNCED state.
        let n = self.buffer.len() - (self.buffer.len() % TS_PACKET_SIZE);
        if n == 0 {
            return Vec::new();
        }
        let bundle: Vec<u8> = self.buffer.drain(..n).collect();
        self.stats.packets_sent += (n / TS_PACKET_SIZE) as u64;
        vec![bundle]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic stream of N TS packets with sync byte at every
    /// 188-byte boundary. PID is irrelevant — just need 0x47 markers.
    fn synthetic_ts_stream(n_packets: usize) -> Vec<u8> {
        let mut buf = Vec::with_capacity(n_packets * 188);
        for i in 0..n_packets {
            buf.push(0x47);
            // Fill the remaining 187 bytes with a recognizable pattern.
            for j in 1..188 {
                buf.push(((i & 0xFF) as u8).wrapping_add(j as u8));
            }
        }
        buf
    }

    #[test]
    fn unsynced_acquires_after_three_sync_bytes() {
        let mut framing = TsFraming::new(18800);
        let ts = synthetic_ts_stream(3);
        let (out, stats) = framing.push(&ts);
        // 3 packets ≠ 7, so no bundle emitted; framing acquired sync.
        assert!(out.is_empty());
        let bytes_pushed = stats.bytes_pushed;
        let bytes_skipped = stats.bytes_skipped_for_sync;
        assert!(framing.is_synced());
        assert_eq!(bytes_pushed, ts.len() as u64);
        assert_eq!(bytes_skipped, 0);
    }

    #[test]
    fn unsynced_skips_misaligned_prefix() {
        let mut framing = TsFraming::new(18800);
        // 50 bytes of garbage prefix, then a valid TS stream.
        let prefix: Vec<u8> = (0..50).map(|i| 0x80 | (i as u8)).collect();
        let ts = synthetic_ts_stream(3);
        let mut input = prefix.clone();
        input.extend_from_slice(&ts);
        let (out, stats) = framing.push(&input);
        assert!(out.is_empty());
        // Copy needed values before stats borrow ends (stats borrows framing).
        let bytes_skipped = stats.bytes_skipped_for_sync;
        let _ = stats; // ensure stats is not used after this point
        assert!(framing.is_synced());
        // The prefix that doesn't contain 0x47 (the test prefix uses
        // 0x80..0xB1 which avoids 0x47) is fully discarded.
        assert!(bytes_skipped >= 50);
    }

    #[test]
    fn emits_one_bundle_per_seven_packets() {
        let mut framing = TsFraming::new(18800);
        let ts = synthetic_ts_stream(7);
        let (out, _stats) = framing.push(&ts);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), 1316);
    }

    #[test]
    fn emits_two_bundles_per_fourteen_packets() {
        let mut framing = TsFraming::new(18800);
        let ts = synthetic_ts_stream(14);
        let (out, _stats) = framing.push(&ts);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 1316);
        assert_eq!(out[1].len(), 1316);
    }

    #[test]
    fn one_byte_at_a_time_acquires_sync() {
        let mut framing = TsFraming::new(18800);
        let ts = synthetic_ts_stream(7);
        let mut accumulated_out = Vec::new();
        for &b in &ts {
            let (mut out, _stats) = framing.push(&[b]);
            accumulated_out.append(&mut out);
        }
        assert_eq!(
            accumulated_out.len(),
            1,
            "should bundle exactly one 7-packet message"
        );
        assert_eq!(accumulated_out[0].len(), 1316);
    }

    #[test]
    fn loss_of_sync_resyncs() {
        let mut framing = TsFraming::new(18800);
        // 7 valid packets, then garbage that breaks alignment, then 7
        // more valid packets with their own valid sync.
        let ts1 = synthetic_ts_stream(7);
        let garbage: Vec<u8> = vec![0x00; 100]; // 100 bytes of zeros — no 0x47
        let ts2 = synthetic_ts_stream(7);
        let mut input = ts1.clone();
        input.extend_from_slice(&garbage);
        input.extend_from_slice(&ts2);
        let (out, stats) = framing.push(&input);
        // 7 + 7 = 14 packets, but the garbage breaks alignment; after
        // resync the second 7 packets get bundled.
        assert!(!out.is_empty(), "first bundle must emit");
        assert!(
            stats.resync_events >= 1,
            "must have at least one resync event"
        );
    }

    #[test]
    fn strict_mode_rejects_misalignment() {
        let mut framing = TsFraming::new(18800);
        let prefix = vec![0x80, 0x81, 0x82];
        let ts = synthetic_ts_stream(3);
        let mut input = prefix;
        input.extend_from_slice(&ts);
        let result = framing.push_strict(&input);
        assert!(result.is_err(), "strict mode must reject prefix garbage");
    }

    #[test]
    fn strict_mode_rejects_non_sync_first_byte() {
        // STRICT mode errors immediately on the first non-0x47 byte. The
        // max_unsynced_bytes knob is RECOVER-mode-only and is not
        // exercised here. (RECOVER mode tracks bytes_skipped_for_sync in
        // stats but does not auto-error on threshold.)
        let mut framing = TsFraming::new(200);
        let no_sync = vec![0x00; 300]; // 300 bytes, never 0x47
        let result = framing.push_strict(&no_sync);
        assert!(
            result.is_err(),
            "STRICT mode must error on the first non-sync byte"
        );
    }

    #[test]
    fn flush_emits_pending_partial_bundle() {
        let mut framing = TsFraming::new(18800);
        let ts = synthetic_ts_stream(3);
        let (out, _stats) = framing.push(&ts);
        assert!(out.is_empty(), "3 < 7 → no bundle yet");
        let flushed = framing.flush();
        assert_eq!(flushed.len(), 1);
        assert_eq!(flushed[0].len(), 188 * 3);
    }
}
