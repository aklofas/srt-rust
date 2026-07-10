//! RFC 6184 H.264 RTP depacketizer — AU reassembly state machine.
//!
//! # Overview
//!
//! [`H264Depacketizer`] reassembles Access Units (AUs) from RTP packets
//! carrying H.264 NAL units per RFC 6184. It follows the `feed()` / `next_au()`
//! idiom used by [`tst_core::mpegts::demux::Demuxer`]: call [`feed`] for each
//! received RTP packet, then drain [`next_au`] until it returns `None`.
//!
//! # AU boundary rules (state-machine contract)
//!
//! 1. **SSRC**: the first packet latches the SSRC. A different SSRC signals a
//!    source restart — the open AU is discarded (no `aus_dropped` tick), seq/
//!    timestamp state is reset, and `ssrc_changes` is incremented. PTS stays
//!    monotonic across resets.
//!
//! 2. **Sequence gaps**: a gap (`delta ≠ 1`) poisons the currently-accumulating
//!    AU (if any) *before* boundary handling, then also poisons the AU this
//!    packet joins *after* boundary handling (sticky `gap_pending` flag).
//!    Duplicate packets (`delta = 0`) are silently ignored.
//!
//! 3. **Timestamp boundary**: when `header.timestamp` differs from the ongoing
//!    AU's timestamp, `complete_au()` is called first, then the new timestamp is
//!    adopted. One AU per RTP timestamp per RFC 6184 §5.1.
//!
//! 4. **Marker fast-path**: `M=1` calls `complete_au()` immediately after
//!    dispatching the packet's payload. This is an early-emission hint, not the
//!    sole boundary mechanism — rule 3 is the correctness path.
//!
//! 5. **`complete_au()`**: an open FU at completion counts as a discarded NALU
//!    and poisons the AU. Empty unpoisoned AUs reset silently. Poisoned AUs
//!    increment `aus_dropped`. Clean AUs push an [`H264Au`] onto the ready queue.
//!
//! 6. **Payload dispatch** on `payload[0] & 0x1F` (NALU type): types 1–23 are
//!    single NAL units (Task 3); type 24 = STAP-A (Task 4); type 28 = FU-A
//!    (Task 5); types 25/26/27/29 are interleaved-mode-only and poison the AU;
//!    types 0/30/31 are reserved and are discarded without poisoning.
//!
//! 7. **`push_nalu(bytes)`**: if the F bit (`bytes[0] & 0x80`) is set the NALU
//!    is discarded and the AU is poisoned. Otherwise `[0,0,0,1]` + bytes is
//!    appended. NALU types 5/7/8 set `au_has_idr`/`au_has_sps`/`au_has_pps`.
//!
//! 8. **[`flush`](H264Depacketizer::flush)**: equivalent to `complete_au()` then
//!    [`next_au`](H264Depacketizer::next_au). The caller should drain `next_au()`
//!    before calling `flush`.
//!
//! [`feed`]: H264Depacketizer::feed
//! [`next_au`]: H264Depacketizer::next_au

use std::collections::VecDeque;

use tst_core::mpegts::common::Pts90khz;

use crate::packet::RtpHeader;

/// Step between the last emitted PTS and the re-anchor point after an SSRC
/// reset. Nominal one 30 fps frame at 90 kHz (3003 ticks). Chosen to be
/// non-zero and monotonically-increasing; the exact value is not meaningful.
const SSRC_RESET_PTS_STEP: i64 = 3003;

/// Annex B framing start code prepended to each NALU.
const ANNEXB_START_CODE: [u8; 4] = [0, 0, 0, 1];

// ──────────────────────────────────────────────────────────────────────────────
// Public types (frozen — later tasks and two binding mirrors depend on
// these exact names / signatures).
// ──────────────────────────────────────────────────────────────────────────────

/// A fully reassembled H.264 Access Unit, ready for decoding or muxing.
///
/// The `annexb` buffer contains all NALUs concatenated in Annex B framing
/// (`[0,0,0,1]` start code before each NALU), in RTP packet order.
#[non_exhaustive]
pub struct H264Au {
    /// Annex B–framed NALU bytes (one or more NALUs concatenated with
    /// `[0,0,0,1]` start codes).
    pub annexb: Vec<u8>,
    /// 90 kHz presentation timestamp, monotonically non-decreasing.
    pub pts: Pts90khz,
    /// `true` if the AU contains at least one IDR slice (NALU type 5).
    pub key_frame: bool,
    /// The RTP timestamp for this AU as carried in the RTP header.
    pub rtp_timestamp: u32,
}

/// Controls whether out-of-band SPS/PPS are injected before IDR frames.
///
/// Task 6 implements injection; until then only [`ParameterSetInjection::None`]
/// has an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParameterSetInjection {
    /// No injection — pass NALUs through exactly as received.
    None,
    /// Inject cached SPS and PPS NALUs before every IDR frame. Useful for
    /// enabling random-access decoding.
    BeforeIdr,
}

impl Default for ParameterSetInjection {
    fn default() -> Self {
        Self::BeforeIdr
    }
}

/// Configuration for [`H264Depacketizer`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct H264DepayConfig {
    /// Expected RTP payload type (7 bits). Packets with a different PT are
    /// still accepted — PT filtering is the transport layer's responsibility.
    pub payload_type: u8,
    /// Whether to inject cached SPS/PPS before IDR frames.
    pub parameter_set_injection: ParameterSetInjection,
    /// Out-of-band parameter sets from SDP `sprop-parameter-sets`. Each
    /// element is one raw NALU (type 7 or 8). Used by injection (Task 6).
    pub initial_parameter_sets: Vec<Vec<u8>>,
}

impl Default for H264DepayConfig {
    fn default() -> Self {
        Self {
            payload_type: 96,
            parameter_set_injection: ParameterSetInjection::BeforeIdr,
            initial_parameter_sets: Vec::new(),
        }
    }
}

/// Counters for monitoring the depacketizer.
///
/// Returned by value from [`H264Depacketizer::stats`] (the struct is `Copy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct H264DepayStats {
    /// Number of complete, unpoisoned AUs emitted.
    pub aus_emitted: u64,
    /// Number of AUs discarded due to poisoning (seq gaps, F-bit, etc.).
    pub aus_dropped: u64,
    /// Number of RTP packets discarded (empty, reserved, interleaved types).
    pub packets_discarded: u64,
    /// Number of NALUs discarded (F-bit set, open FU at AU completion, etc.).
    pub nalus_discarded: u64,
    /// Number of sequence-number gaps detected.
    pub seq_gaps: u64,
    /// Number of duplicate sequence numbers detected.
    pub duplicate_packets: u64,
    /// Number of times cached parameter sets were updated (Task 6).
    pub parameter_set_updates: u64,
    /// Number of SSRC changes (source restarts) detected.
    pub ssrc_changes: u64,
}

// ──────────────────────────────────────────────────────────────────────────────
// Depacketizer
// ──────────────────────────────────────────────────────────────────────────────

/// RFC 6184 H.264 RTP depacketizer.
///
/// Call [`feed`](Self::feed) for each received RTP packet (in order of
/// arrival), then drain [`next_au`](Self::next_au) until it returns `None`.
///
/// # Panic-freedom
///
/// `feed` never panics on any input byte pattern — adversarial payloads are
/// handled by silently discarding and ticking the appropriate stat counter.
///
/// # AU boundary / marker / poisoning contract summary
///
/// See the [module-level doc](self) for the full 8-rule state-machine contract.
pub struct H264Depacketizer {
    // Used in Task 6 for parameter-set injection.
    #[allow(dead_code)]
    config: H264DepayConfig,
    stats: H264DepayStats,

    // SSRC state (rule 1)
    ssrc: Option<u32>,

    // Seq tracking (rule 2)
    last_seq: Option<u16>,

    // RTP timestamp unwrapping (rule 3)
    /// Last RTP timestamp seen (wrapping u32).
    last_ts: Option<u32>,
    /// Extended (unwrapped) timestamp corresponding to `last_ts`.
    ts_ext: i64,

    // PTS anchoring
    /// Extended timestamp at which the current AU started.
    au_ts_ext: i64,
    /// PTS offset: `pts_raw = au_ts_ext - pts_base`. Set on first AU after
    /// construction or SSRC reset.
    pts_base: Option<i64>,
    /// Last PTS emitted. Initialised to `-SSRC_RESET_PTS_STEP` so the first
    /// AU lands at PTS 0 through the same re-anchor path used after resets.
    last_emitted_pts: i64,

    // Per-AU accumulator (rule 5)
    /// The RTP timestamp for the open AU (u32 wrapping form).
    au_ts: Option<u32>,
    /// Annex B buffer for the open AU.
    au_buf: Vec<u8>,
    /// Open FU-A accumulation buffer (Task 5 fills this; exists for rule 2
    /// gap-clearing and rule 5 completion checks).
    fu: Option<Vec<u8>>,
    /// True if the AU should be dropped when completed.
    au_poisoned: bool,
    /// True if the next AU start should be poisoned (sticky from a seq gap).
    gap_pending: bool,
    /// AU metadata flags (rules 7, 5)
    au_has_idr: bool,
    au_has_sps: bool,
    au_has_pps: bool,

    /// Fully reassembled AUs waiting to be consumed.
    ready: VecDeque<H264Au>,
}

impl H264Depacketizer {
    /// Construct a new depacketizer with the given configuration.
    pub fn new(config: H264DepayConfig) -> Self {
        Self {
            config,
            stats: H264DepayStats::default(),
            ssrc: None,
            last_seq: None,
            last_ts: None,
            ts_ext: 0,
            au_ts_ext: 0,
            pts_base: None,
            last_emitted_pts: -SSRC_RESET_PTS_STEP,
            au_ts: None,
            au_buf: Vec::new(),
            fu: None,
            au_poisoned: false,
            gap_pending: false,
            au_has_idr: false,
            au_has_sps: false,
            au_has_pps: false,
            ready: VecDeque::new(),
        }
    }

    /// Feed one RTP packet into the depacketizer.
    ///
    /// This never fails and never panics — any malformed or unexpected input
    /// is handled by discarding and ticking the appropriate stat counter.
    pub fn feed(&mut self, header: &RtpHeader, payload: &[u8]) {
        // ── Rule 1: SSRC ──────────────────────────────────────────────────
        if let Some(known) = self.ssrc {
            if header.ssrc != known {
                // Source restart: discard open AU without counting it.
                self.au_ts = None;
                self.au_buf.clear();
                self.fu = None;
                self.au_poisoned = false;
                self.gap_pending = false;
                self.au_has_idr = false;
                self.au_has_sps = false;
                self.au_has_pps = false;
                // Reset seq/ts tracking.
                self.last_seq = None;
                self.last_ts = None;
                // PTS stays monotonic; next AU re-anchors at
                // last_emitted_pts + SSRC_RESET_PTS_STEP.
                self.pts_base = None;
                self.ssrc = Some(header.ssrc);
                self.stats.ssrc_changes += 1;
            }
        } else {
            self.ssrc = Some(header.ssrc);
        }

        // ── Rule 2: seq-gap / duplicate detection ─────────────────────────
        // Do this BEFORE boundary handling (rule 2 says "poison the
        // currently-accumulating AU before boundary handling").
        let is_gap;
        if let Some(last) = self.last_seq {
            let delta = header.seq.wrapping_sub(last);
            if delta == 0 {
                self.stats.duplicate_packets += 1;
                return; // whole packet ignored
            } else if delta == 1 {
                is_gap = false;
            } else {
                // Gap: poison the open AU right now (before boundary handling).
                if self.au_ts.is_some() {
                    self.au_poisoned = true;
                }
                self.stats.seq_gaps += 1;
                is_gap = true;
            }
        } else {
            is_gap = false;
        }
        self.last_seq = Some(header.seq);

        // ── Rule 3: timestamp boundary ────────────────────────────────────
        // Unwrap the RTP timestamp.
        let ts_ext = if let Some(last_ts) = self.last_ts {
            // Nearest-distance unwrap: cast the wrapping_sub to i32 then i64.
            self.ts_ext + (header.timestamp.wrapping_sub(last_ts) as i32 as i64)
        } else {
            // First packet seeds ts_ext = 0.
            0
        };
        self.last_ts = Some(header.timestamp);
        self.ts_ext = ts_ext;

        if let Some(open_ts) = self.au_ts {
            if header.timestamp != open_ts {
                // Different timestamp → complete the open AU first.
                self.complete_au();
            }
        }

        // Adopt this timestamp for the current AU.
        if self.au_ts.is_none() {
            self.au_ts = Some(header.timestamp);
            self.au_ts_ext = ts_ext;

            // Anchor PTS on first AU after construction / SSRC reset.
            if self.pts_base.is_none() {
                self.pts_base = Some(ts_ext - (self.last_emitted_pts + SSRC_RESET_PTS_STEP));
            }

            // Apply sticky gap poison to this newly-started AU.
            if is_gap || self.gap_pending {
                self.au_poisoned = true;
            }
        }
        // For subsequent packets of the same AU, apply gap poison here.
        if is_gap {
            self.au_poisoned = true;
            self.gap_pending = true;
        } else {
            self.gap_pending = false;
        }

        // ── Rule 6: payload dispatch ──────────────────────────────────────
        if payload.is_empty() {
            self.stats.packets_discarded += 1;
            return;
        }
        let nalu_type = payload[0] & 0x1F;
        match nalu_type {
            1..=23 => {
                // Single NALU packet (RFC 6184 §5.6).
                self.push_nalu(payload);
            }
            24 => {
                // Task 4 replaces this arm (STAP-A).
                self.stats.packets_discarded += 1;
                self.au_poisoned = true;
            }
            28 => {
                // Task 5 replaces this arm (FU-A).
                self.stats.packets_discarded += 1;
                self.au_poisoned = true;
            }
            25 | 26 | 27 | 29 => {
                // Interleaved-mode-only types — unrecoverable, poison the AU.
                self.stats.packets_discarded += 1;
                self.au_poisoned = true;
            }
            _ => {
                // Types 0 and 30–31: reserved per §5.4 Table 3 ("ig").
                // Discard WITHOUT poisoning.
                self.stats.packets_discarded += 1;
            }
        }

        // ── Rule 4: marker fast-path ──────────────────────────────────────
        if header.marker {
            self.complete_au();
        }
    }

    /// Pull the next completed Access Unit, if one is available.
    ///
    /// Call repeatedly until `None` after each [`feed`](Self::feed) call.
    pub fn next_au(&mut self) -> Option<H264Au> {
        self.ready.pop_front()
    }

    /// Force completion of any open AU and return it.
    ///
    /// The caller should drain [`Self::next_au`] before calling this. After
    /// `flush` the depacketizer state is clean.
    pub fn flush(&mut self) -> Option<H264Au> {
        self.complete_au();
        self.ready.pop_front()
    }

    /// Return a snapshot of the current statistics.
    pub fn stats(&self) -> H264DepayStats {
        self.stats
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    /// Complete the open AU (rule 5).
    fn complete_au(&mut self) {
        if self.au_ts.is_none() {
            // Nothing open.
            return;
        }

        // An open FU at completion → discard + poison.
        if self.fu.take().is_some() {
            self.stats.nalus_discarded += 1;
            self.au_poisoned = true;
        }

        if self.au_buf.is_empty() && !self.au_poisoned {
            // Empty unpoisoned AU — reset silently.
            self.reset_au();
            return;
        }

        if self.au_poisoned {
            self.stats.aus_dropped += 1;
            self.reset_au();
            return;
        }

        // Compute PTS.
        let pts_base = self.pts_base.unwrap_or(0);
        let pts_raw = self.au_ts_ext - pts_base;
        let pts = Pts90khz::new(pts_raw);
        self.last_emitted_pts = pts_raw;

        // Collect the completed AU.
        let annexb = core::mem::take(&mut self.au_buf);
        let au = H264Au {
            annexb,
            pts,
            key_frame: self.au_has_idr,
            rtp_timestamp: self.au_ts.unwrap_or(0),
        };
        self.stats.aus_emitted += 1;
        self.ready.push_back(au);
        self.reset_au();
    }

    /// Push one NALU into the open AU buffer (rule 7).
    ///
    /// If the F bit is set the NALU is corrupt — discard and poison.
    fn push_nalu(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            // Shouldn't happen given the dispatch check, but be safe.
            self.stats.packets_discarded += 1;
            return;
        }
        if bytes[0] & 0x80 != 0 {
            // F bit set → advertised corrupt, §5.3.
            self.stats.nalus_discarded += 1;
            self.au_poisoned = true;
            return;
        }
        let nalu_type = bytes[0] & 0x1F;
        match nalu_type {
            5 => self.au_has_idr = true,
            7 => {
                self.au_has_sps = true;
                // Task 6 updates the parameter-set cache here.
            }
            8 => {
                self.au_has_pps = true;
                // Task 6 updates the parameter-set cache here.
            }
            _ => {}
        }
        // Append Annex B start code + NALU bytes.
        self.au_buf.extend_from_slice(&ANNEXB_START_CODE);
        self.au_buf.extend_from_slice(bytes);
    }

    /// Reset per-AU state (called after completing or discarding an AU).
    fn reset_au(&mut self) {
        self.au_ts = None;
        self.au_buf.clear();
        self.fu = None;
        self.au_poisoned = false;
        self.au_has_idr = false;
        self.au_has_sps = false;
        self.au_has_pps = false;
        // gap_pending is NOT reset here — it must persist to poison the next AU.
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(seq: u16, ts: u32, marker: bool) -> RtpHeader {
        let mut h = RtpHeader::new(seq, ts, 0xAABB);
        h.marker = marker;
        h.payload_type = 96;
        h
    }

    fn depay() -> H264Depacketizer {
        H264Depacketizer::new(H264DepayConfig {
            parameter_set_injection: ParameterSetInjection::None, // injection tested in Task 6
            ..H264DepayConfig::default()
        })
    }

    #[test]
    fn single_nalu_au_completed_by_marker() {
        let mut d = depay();
        d.feed(&hdr(1, 90_000, true), &[0x65, 0xAA]); // type-5 IDR slice
        let au = d.next_au().expect("marker completes AU");
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x65, 0xAA]);
        assert!(au.key_frame);
        assert_eq!(au.pts, Pts90khz::new(0)); // first AU zero-based
        assert_eq!(au.rtp_timestamp, 90_000);
    }

    #[test]
    fn markerless_au_completed_by_timestamp_change() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[0x41, 0x01]); // non-IDR slice
        assert!(d.next_au().is_none()); // no marker → held
        d.feed(&hdr(2, 4003, false), &[0x41, 0x02]); // next AU's first packet
        let au = d.next_au().expect("ts change completes previous AU");
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x41, 0x01]);
        assert!(!au.key_frame);
        assert_eq!(au.pts, Pts90khz::new(0));
        // second AU: PTS = delta from first = 3003
        let au2 = d.flush().expect("flush emits final AU");
        assert_eq!(au2.pts, Pts90khz::new(3003));
    }

    #[test]
    fn two_nalus_one_timestamp_one_au() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[0x67, 0x42]); // SPS
        d.feed(&hdr(2, 1000, true), &[0x65, 0x88]); // IDR, marker
        let au = d.next_au().unwrap();
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 1, 0x65, 0x88]);
        assert!(au.key_frame);
    }

    #[test]
    fn timestamp_wrap_unwraps_monotonically() {
        let mut d = depay();
        // (u32::MAX - 1000) + 3003 wraps to 2002, so +3003 ticks across the u32 boundary.
        // Note: the brief used 2003 here, but (u32::MAX-1000) → 2003 is a delta of 3004,
        // not 3003. Using 2002 matches the expected output of Pts90khz::new(3003).
        d.feed(&hdr(1, u32::MAX - 1000, true), &[0x41, 0x01]);
        d.feed(&hdr(2, 2002, true), &[0x41, 0x02]); // wrapped: +3003 across the u32 boundary
        let a = d.next_au().unwrap();
        let b = d.next_au().unwrap();
        assert_eq!(a.pts, Pts90khz::new(0));
        assert_eq!(b.pts, Pts90khz::new(3003));
    }

    #[test]
    fn reserved_types_ignored_without_poisoning() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[0x41, 0x01]);
        d.feed(&hdr(2, 1000, false), &[30, 0xFF]); // reserved type 30: dropped, AU survives
        d.feed(&hdr(3, 1000, true), &[0x41, 0x02]);
        let au = d.next_au().expect("reserved type must not poison");
        assert_eq!(d.stats().packets_discarded, 1);
        assert_eq!(au.annexb.len(), 4 + 2 + 4 + 2);
    }

    #[test]
    fn duplicate_seq_ignored() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[0x41, 0x01]);
        d.feed(&hdr(1, 1000, false), &[0x41, 0x01]); // exact dup
        d.feed(&hdr(2, 1000, true), &[0x41, 0x02]);
        let au = d.next_au().unwrap();
        assert_eq!(au.annexb.len(), 12);
        assert_eq!(d.stats().duplicate_packets, 1);
    }

    // ── Additional correctness tests ─────────────────────────────────────────

    #[test]
    fn ssrc_change_resets_and_keeps_pts_monotonic() {
        let mut d = depay();
        // First AU from SSRC 0xAABB (hdr() uses ssrc=0xAABB).
        d.feed(&hdr(1, 1000, true), &[0x41, 0x01]);
        let au1 = d.next_au().unwrap();
        assert_eq!(au1.pts, Pts90khz::new(0));

        // SSRC change → source restart. Use a different SSRC (0xCCDD).
        let mut h2 = RtpHeader::new(1, 4003, 0xCCDD);
        h2.marker = true;
        h2.payload_type = 96;
        d.feed(&h2, &[0x41, 0x02]);
        let au2 = d.next_au().unwrap();
        // After SSRC reset the next AU re-anchors at last_emitted_pts + SSRC_RESET_PTS_STEP.
        // last_emitted_pts was 0, so new anchor = 0 + 3003 = 3003.
        assert_eq!(au2.pts, Pts90khz::new(3003));
    }

    #[test]
    fn f_bit_poisons_au() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[0x41, 0x01]); // clean NALU
        d.feed(&hdr(2, 1000, true), &[0xE1, 0xFF]); // F=1, type 1 — corrupt
        assert!(d.next_au().is_none()); // AU dropped
        assert_eq!(d.stats().aus_dropped, 1);
        assert_eq!(d.stats().nalus_discarded, 1);
    }

    #[test]
    fn interleaved_types_poison_au() {
        for &nalu_type in &[25u8, 26, 27, 29] {
            let mut d = depay();
            d.feed(&hdr(1, 1000, false), &[0x41, 0x01]); // clean NALU
            let payload = [nalu_type, 0xFF];
            d.feed(&hdr(2, 1000, true), &payload);
            assert!(d.next_au().is_none(), "type {} should poison", nalu_type);
            assert_eq!(d.stats().aus_dropped, 1, "type {}", nalu_type);
        }
    }

    #[test]
    fn empty_payload_discarded() {
        let mut d = depay();
        d.feed(&hdr(1, 1000, false), &[]); // empty
        d.feed(&hdr(2, 1000, true), &[0x41, 0x01]);
        // AU should succeed — empty packet only ticks packets_discarded.
        let au = d.next_au().expect("non-empty AU should complete");
        assert_eq!(d.stats().packets_discarded, 1);
        assert_eq!(au.annexb, [0, 0, 0, 1, 0x41, 0x01]);
    }

    #[test]
    fn seq_gap_poisons_current_and_next_au() {
        // AU1: seq 1 only. AU2: seq 2 then gap. AU3: should be dropped (gap_pending).
        // AU4: should be clean.
        let mut d = depay();
        // AU1: clean, completed by marker.
        d.feed(&hdr(1, 1000, true), &[0x41, 0x01]);
        let au1 = d.next_au().expect("AU1 clean");
        assert_eq!(au1.pts, Pts90khz::new(0));

        // AU2: seq 2, marker-closed.
        d.feed(&hdr(2, 2000, true), &[0x41, 0x02]);
        let au2 = d.next_au().expect("AU2 clean");
        assert!(!au2.key_frame);

        // Gap: seq 5 (skipped 3,4) on AU3.
        d.feed(&hdr(5, 3000, true), &[0x41, 0x03]);
        assert!(d.next_au().is_none(), "AU3 should be poisoned by gap");
        assert_eq!(d.stats().seq_gaps, 1);
        assert_eq!(d.stats().aus_dropped, 1);

        // AU4: seq 6, right after gap — gap_pending should poison it too.
        d.feed(&hdr(6, 4000, true), &[0x41, 0x04]);
        assert!(d.next_au().is_none(), "AU4 poisoned by gap_pending");
        assert_eq!(d.stats().aus_dropped, 2);

        // AU5: seq 7, clean.
        d.feed(&hdr(7, 5000, true), &[0x41, 0x05]);
        assert!(d.next_au().is_some(), "AU5 should be clean");
    }
}
