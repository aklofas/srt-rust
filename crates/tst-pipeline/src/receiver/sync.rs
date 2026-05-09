//! TS sync state machine. Locks on the 0x47 sync byte spaced 188 bytes apart.
//!
//! ## States
//!
//! - **HUNT** — scanning forward byte-by-byte for the first 0x47.
//! - **VERIFY** — found a candidate 0x47; peeking ahead 188*N bytes to
//!   confirm N more sync bytes at the expected positions before committing.
//!   We require 4 confirming sync bytes total (count 1..=4) before locking.
//!   VERIFY is peek-only: no bytes are consumed from the buffer. That means
//!   every byte fed during verification is still available for LOCKED to drain
//!   as real packets.
//! - **LOCKED** — confirmed alignment. Emit 188-byte packets one at a time,
//!   consuming from the front of the buffer. If the front byte ever stops
//!   being 0x47, fall back to HUNT.
//!
//! This mirrors the posture of libavformat's `mpegts_resync` and ffmpeg's
//! two-pass sync strategy: don't declare lock until you've seen several
//! back-to-back aligned sync bytes.

/// Current state of the sync state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// Looking for the first 0x47 sync byte.
    Hunt,
    /// Found a candidate 0x47 at `buf[0]`. `count` is the number of confirmations
    /// seen so far — starts at 1 (the initial 0x47 in HUNT transitions here).
    /// After 4 total confirmations (`new_count >= 4`) we transition to LOCKED.
    Verify { count: u8 },
    /// Confirmed 0x47 alignment. Emitting 188-byte packets from buf[0..188].
    Locked,
}

/// Stateful TS sync recoverer.
///
/// Feed bytes via [`push`][Self::push] and drain aligned 188-byte packets via
/// [`next_packet`][Self::next_packet]. Bytes not yet consumed by a packet
/// stay in the internal buffer; push more data when `next_packet` returns
/// `None`.
///
/// The internal buffer is a `Vec<u8>` with `drain`-based consumption.
/// For receiver pipelines the packet rate (~7 Mbit/s of 1316-byte SRT
/// messages) means the drain overhead is not a bottleneck in practice;
/// a ring-buffer optimisation is deferred.
#[derive(Debug)]
pub struct Syncer {
    state: SyncState,
    buf: Vec<u8>,
    /// Bytes drained while scanning for alignment (HUNT skips + VERIFY
    /// single-byte drops on failed confirmation).
    pub(crate) bytes_skipped_for_sync: u64,
    /// Number of times the syncer has transitioned from HUNT/VERIFY to
    /// LOCKED — counts successful lock acquisitions (initial lock-on and
    /// re-locks after sync loss).
    pub(crate) resync_events: u64,
}

impl Syncer {
    /// Create a new [`Syncer`] starting in the HUNT state with an empty buffer.
    pub fn new() -> Self {
        Self {
            state: SyncState::Hunt,
            buf: Vec::new(),
            bytes_skipped_for_sync: 0,
            resync_events: 0,
        }
    }

    /// Append incoming bytes to the internal buffer.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Reset to HUNT and discard any buffered bytes.
    ///
    /// Intended for reconnect scenarios where the underlying transport has
    /// been re-established: bytes left over from a dead connection cannot
    /// straddle the reconnect boundary and must not contribute to the new
    /// alignment search. Higher-level shells (a future `ManagedReceiver`)
    /// call this after a transport rebuild before feeding fresh bytes.
    ///
    /// Does NOT reset stat counters — those are owned by
    /// [`Receiver`][crate::Receiver] and reset via
    /// [`Receiver::reset_stats`][crate::Receiver::reset_stats].
    pub fn reset(&mut self) {
        self.buf.clear();
        self.state = SyncState::Hunt;
    }

    /// Zero the sync-recovery counters. Called by `Receiver::reset_stats`.
    pub(crate) fn reset_stats(&mut self) {
        self.bytes_skipped_for_sync = 0;
        self.resync_events = 0;
    }

    /// Pull the next 188-byte aligned packet from the buffer, if one is ready.
    ///
    /// Returns `None` when more bytes must be [`push`][Self::push]ed before
    /// the next packet can be emitted. The caller should feed more data and
    /// call `next_packet` again.
    ///
    /// VERIFY state is peek-only: bytes are not consumed while confirming
    /// alignment, so all bytes pushed during the verification window remain
    /// available for LOCKED to emit as real packets.
    ///
    /// Allocates one `Vec<u8>` per emitted packet (heap alloc on the hot
    /// path). The caller (`Receiver`) immediately copies the result into a
    /// `[u8; 188]`. Both costs disappear if/when this struct is reshaped to a
    /// ring buffer; deferred to a follow-up so Task 12 stays scoped to the
    /// state machine.
    pub fn next_packet(&mut self) -> Option<Vec<u8>> {
        loop {
            match self.state {
                SyncState::Hunt => {
                    // Scan forward for the first 0x47 sync byte. Drain
                    // everything before it — those bytes can never be the
                    // start of a valid packet.
                    let pos = self.buf.iter().position(|&b| b == 0x47)?;
                    self.bytes_skipped_for_sync += pos as u64;
                    self.buf.drain(..pos);
                    // buf[0] is now 0x47. One confirmation seen.
                    self.state = SyncState::Verify { count: 1 };
                    // Fall through to VERIFY on the next loop iteration.
                }
                SyncState::Verify { count } => {
                    // We have a candidate 0x47 at buf[0]. Check whether
                    // buf[188 * count] is also 0x47.
                    //
                    // `need` ensures the index we're about to peek at actually
                    // exists in the buffer before we read it. The +1 means:
                    // after the last confirmation (count→4, lock) we also know
                    // there is at least one byte of the first real packet in
                    // the buffer, so the LOCKED arm won't immediately return
                    // None on the following iteration.
                    let need = 188 * (count as usize + 1) + 1;
                    if self.buf.len() < need {
                        return None;
                    }
                    if self.buf[188 * count as usize] == 0x47 {
                        let new_count = count + 1;
                        if new_count >= 4 {
                            // Four confirmations — alignment is solid.
                            self.state = SyncState::Locked;
                            // Count each lock acquisition (initial + re-locks
                            // after sync loss) as one resync event, mirroring
                            // the sender-side framing convention.
                            self.resync_events += 1;
                        } else {
                            self.state = SyncState::Verify { count: new_count };
                        }
                        // Loop: either emit from LOCKED or check next confirmation.
                    } else {
                        // Candidate failed. Drop one byte and re-hunt.
                        // We can't just advance 188 bytes because buf[1..188]
                        // may contain a real sync byte for a different alignment.
                        self.bytes_skipped_for_sync += 1;
                        self.buf.drain(..1);
                        self.state = SyncState::Hunt;
                    }
                }
                SyncState::Locked => {
                    if self.buf.len() < 188 {
                        return None;
                    }
                    if self.buf[0] != 0x47 {
                        // Lost sync — corrupted stream or a gap in the source.
                        // Fall back to HUNT; the loop will scan for the next
                        // 0x47 without consuming the non-sync byte here
                        // (HUNT will drain up to it).
                        self.state = SyncState::Hunt;
                        continue;
                    }
                    // Emit one packet. The syncer always emits exactly 188 bytes
                    // when in LOCKED state; the `.unwrap()` in the caller is
                    // sound because `pkt` is always a 188-element Vec here.
                    let pkt = self.buf[..188].to_vec();
                    self.buf.drain(..188);
                    return Some(pkt);
                }
            }
        }
    }

    /// Current sync state. Exposed for testing and diagnostics.
    #[cfg(test)]
    pub fn state(&self) -> SyncState {
        self.state
    }
}

impl Default for Syncer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a syntactically valid 188-byte TS packet with the given PID.
    /// Payload bytes are 0xFF (don't-care filler).
    fn ts_packet(pid: u16) -> [u8; 188] {
        let mut buf = [0xFFu8; 188];
        buf[0] = 0x47; // sync byte
        buf[1] = 0x40 | ((pid >> 8) as u8 & 0x1F); // PUSI + PID high
        buf[2] = (pid & 0xFF) as u8; // PID low
        buf[3] = 0x10; // no adaptation field, has payload
        buf
    }

    /// Feed N packets, push all at once, drain everything. The Syncer is in
    /// peek-only VERIFY mode, so all 6 bytes fed during alignment confirmation
    /// remain in the buffer and are emitted once LOCKED.
    #[test]
    fn locks_after_four_packets() {
        let mut s = Syncer::new();
        let mut buf = Vec::new();
        for i in 0..6 {
            buf.extend_from_slice(&ts_packet(i));
        }
        s.push(&buf);
        let mut got = 0;
        while s.next_packet().is_some() {
            got += 1;
        }
        // VERIFY is peek-only: the 4 confirmation bytes aren't consumed, so all
        // 6 packets are available once the syncer transitions to LOCKED.
        assert_eq!(got, 6);
        assert_eq!(s.state(), SyncState::Locked);
    }

    /// Garbage prefix before the first valid TS stream — HUNT scans past it.
    #[test]
    fn skips_garbage_prefix() {
        let mut s = Syncer::new();
        let mut buf = vec![0xAAu8; 100]; // 100 bytes of garbage before any 0x47
        for i in 0..5 {
            buf.extend_from_slice(&ts_packet(i));
        }
        s.push(&buf);
        let mut got = 0;
        while s.next_packet().is_some() {
            got += 1;
        }
        assert_eq!(got, 5);
    }

    /// If alignment is lost mid-VERIFY (a sync byte at the expected position
    /// is missing), the syncer drops one byte and re-hunts rather than
    /// discarding a potentially valid alignment just 1 byte forward.
    #[test]
    fn recovers_from_interrupted_verify() {
        let mut s = Syncer::new();
        // Start with a fake 0x47 (only one, no subsequent sync bytes 188 apart).
        // Then 400 bytes of filler followed by 5 real packets.
        let mut buf = vec![0x47u8]; // triggers Verify { count: 1 }
        buf.extend_from_slice(&[0xBBu8; 187]); // not a real packet
        buf.extend_from_slice(&[0xCCu8; 213]); // more garbage (no 0x47 at 188)
        for i in 0..5 {
            buf.extend_from_slice(&ts_packet(i));
        }
        s.push(&buf);
        let mut got = 0;
        while s.next_packet().is_some() {
            got += 1;
        }
        assert_eq!(got, 5);
    }

    /// After locking, a corrupted packet (missing sync byte) causes a
    /// re-hunt. The syncer recovers and emits the remaining valid packets.
    #[test]
    fn recovers_after_sync_loss_in_locked() {
        let mut s = Syncer::new();
        let mut buf = Vec::new();
        // 5 good packets to establish lock.
        for i in 0..5u16 {
            buf.extend_from_slice(&ts_packet(i));
        }
        // One corrupted packet: first byte is 0x00 instead of 0x47. None of
        // the remaining bytes in this packet are 0x47 (pid=200 → high byte
        // 0x40|0=0x40, low byte 0xC8; filler is 0xFF).
        let mut bad = ts_packet(200);
        bad[0] = 0x00;
        buf.extend_from_slice(&bad);
        // 6 more good packets after the bad one — enough for VERIFY to re-lock
        // (needs 4 confirmations, so ≥5 packets, with the +1 byte lookahead
        // pushing the minimum to 5 * 188 + 1 = 941 bytes = 5 full packets).
        for i in 0..6u16 {
            buf.extend_from_slice(&ts_packet(i + 100));
        }
        s.push(&buf);

        let mut got = 0;
        while s.next_packet().is_some() {
            got += 1;
        }
        // 5 leading packets + 6 trailing packets (bad packet's 188 bytes are
        // consumed by HUNT as it scans for the next 0x47).
        assert_eq!(got, 11);
    }
}
