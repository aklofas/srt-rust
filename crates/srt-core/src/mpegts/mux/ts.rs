//! 188-byte MPEG-TS packet writer.
//!
//! Handles:
//! - Sync byte (0x47), TEI, transport priority
//! - `payload_unit_start_indicator` (PUSI)
//! - 13-bit PID
//! - Per-PID 4-bit continuity counter (increments on payload-bearing packets
//!   per ISO/IEC 13818-1 §2.4.3.3)
//! - Adaptation field: optional PCR (6 bytes), random_access_indicator,
//!   stuffing
//! - Payload fill from caller-supplied bytes
//!
//! Cadence decisions (when to emit PCR, when to emit PSI, when to start a
//! new PES) live in the `Muxer` orchestrator (Task 8). This module is purely
//! mechanical packet assembly.

use crate::mpegts::common::Pcr27mhz;
use std::collections::BTreeMap;

/// Per-PID 4-bit continuity counters.
///
/// `BTreeMap` rather than `HashMap` to avoid pulling in a hasher dep and
/// keep the data path zero-allocation after warm-up (we have ≤ 4 PIDs in v0).
#[derive(Debug, Default)]
pub(crate) struct ContinuityCounters {
    counters: BTreeMap<u16, u8>,
}

impl ContinuityCounters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the next continuity counter value for `pid`, incrementing it.
    /// Per spec, this is called only when the packet has a payload.
    fn next(&mut self, pid: u16) -> u8 {
        let entry = self.counters.entry(pid).or_insert(0);
        let cc = *entry & 0x0F;
        *entry = (*entry + 1) & 0x0F;
        cc
    }
}

/// Optional adaptation field contents to attach to a packet.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AdaptationField {
    /// Set the `random_access_indicator` flag — used on the first packet of
    /// an IDR access unit so receivers can find a tune-in point.
    pub random_access: bool,
    /// PCR to embed. None = no PCR field.
    pub pcr: Option<Pcr27mhz>,
}

impl AdaptationField {
    pub fn is_empty(&self) -> bool {
        !self.random_access && self.pcr.is_none()
    }

    /// Number of bytes the adaptation field occupies excluding stuffing,
    /// including the length byte.
    /// = 1 byte length + 1 byte flags + (6 bytes PCR if present)
    fn min_size(&self) -> usize {
        if self.is_empty() {
            0
        } else {
            2 + if self.pcr.is_some() { 6 } else { 0 }
        }
    }
}

/// Result of writing a TS packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WriteResult {
    /// Bytes of payload consumed from the input.
    pub payload_consumed: usize,
}

/// Write one 188-byte TS packet into `out`, packing as much of `payload` as
/// fits given the requested adaptation field.
///
/// `out.len()` must be exactly 188; panics otherwise (private API, caller
/// responsibility).
///
/// Returns the number of payload bytes consumed. The caller advances its
/// payload cursor and calls again with the remainder.
///
/// `payload_unit_start` should be true on the first packet of a new PES or
/// PSI section.
pub(crate) fn write_packet(
    out: &mut [u8; 188],
    pid: u16,
    payload_unit_start: bool,
    adaptation: AdaptationField,
    payload: &[u8],
    counters: &mut ContinuityCounters,
) -> WriteResult {
    debug_assert!(pid <= 0x1FFF, "PID out of range");

    // Header byte 0: sync byte
    out[0] = 0x47;
    // Header byte 1: TEI (0) | PUSI | TP (0) | PID high 5 bits
    out[1] = ((payload_unit_start as u8) << 6) | ((pid >> 8) as u8 & 0x1F);
    // Header byte 2: PID low 8 bits
    out[2] = (pid & 0xFF) as u8;

    // Capacity bookkeeping. `af_min` is the AF byte count when content is
    // present (length + flags + optional PCR). When af_min == 0 but payload
    // can't fill 184 bytes, we still need an AF (just a length byte and
    // stuffing) — that costs 1 byte for the length. `af_overhead` captures
    // both cases.
    let af_min = adaptation.min_size();
    let no_af_payload_capacity: usize = 188 - 4; // 184
    let needs_padding_only_af = af_min == 0 && payload.len() < no_af_payload_capacity;
    let af_overhead = if af_min > 0 {
        af_min
    } else if needs_padding_only_af {
        1
    } else {
        0
    };
    let payload_capacity = 188 - 4 - af_overhead;
    let to_copy = payload_capacity.min(payload.len());
    let stuffing = payload_capacity - to_copy;

    // adaptation_field_control: 11 if AF present, 01 if payload-only.
    // continuity_counter is incremented because we always carry payload here.
    let cc = counters.next(pid);
    let afc = if af_overhead > 0 { 0b11 } else { 0b01 };
    out[3] = (afc << 4) | (cc & 0x0F);

    let mut cursor = 4;

    if afc == 0b11 {
        // adaptation_field_length is the count of bytes after the length byte
        // itself: flags+PCR (= af_min - 1 if af_min > 0 else 0) plus stuffing.
        let af_body_after_length = if af_min > 0 { af_min - 1 } else { 0 } + stuffing;
        out[cursor] = af_body_after_length as u8;
        cursor += 1;

        if af_min > 0 {
            // Flags byte: discontinuity=0, random_access, ES_priority=0,
            // PCR_flag, OPCR=0, splicing=0, transport_private=0, AF_extension=0
            let flags: u8 =
                ((adaptation.random_access as u8) << 6) | ((adaptation.pcr.is_some() as u8) << 4);
            out[cursor] = flags;
            cursor += 1;

            if let Some(pcr) = adaptation.pcr {
                write_pcr(&mut out[cursor..cursor + 6], pcr);
                cursor += 6;
            }
        }

        // Stuffing bytes (0xFF per spec).
        for byte in &mut out[cursor..cursor + stuffing] {
            *byte = 0xFF;
        }
        cursor += stuffing;
    }

    // Payload.
    out[cursor..cursor + to_copy].copy_from_slice(&payload[..to_copy]);
    cursor += to_copy;

    debug_assert_eq!(cursor, 188);

    WriteResult {
        payload_consumed: to_copy,
    }
}

/// Encode a PCR into 6 bytes per ISO/IEC 13818-1 §2.4.3.5:
///   33-bit base (90 kHz) | 6 reserved bits (set to 1) | 9-bit extension (27 MHz mod 300)
fn write_pcr(out: &mut [u8], pcr: Pcr27mhz) {
    debug_assert!(out.len() == 6);
    let base = pcr.base();
    let ext = pcr.extension() as u32;
    out[0] = (base >> 25) as u8;
    out[1] = (base >> 17) as u8;
    out[2] = (base >> 9) as u8;
    out[3] = (base >> 1) as u8;
    out[4] = (((base & 0x1) << 7) as u8) | 0x7E | ((ext >> 8) & 0x01) as u8;
    out[5] = (ext & 0xFF) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_layout_basic() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let payload = vec![0xAB; 200]; // larger than capacity
        let result = write_packet(
            &mut buf,
            0x1011,
            true,
            AdaptationField::default(),
            &payload,
            &mut cc,
        );
        assert_eq!(buf[0], 0x47);
        // PID 0x1011 high-5 = 0b10000 = 0x10. With PUSI bit = 0x40.
        // byte1 = 0x40 | 0x10 = 0x50.
        assert_eq!(buf[1], 0x50);
        assert_eq!(buf[2], 0x11);
        // afc = 01 (no adaptation), cc = 0 first time
        assert_eq!(buf[3], 0b01 << 4);
        // 184 bytes of payload should be copied (all 0xAB).
        assert_eq!(result.payload_consumed, 184);
        assert!(buf[4..].iter().all(|&b| b == 0xAB));
    }

    #[test]
    fn continuity_counter_increments_per_pid() {
        let mut cc = ContinuityCounters::new();
        let payload = vec![0u8; 200];
        let mut buf = [0u8; 188];
        for expected_cc in 0..16 {
            write_packet(
                &mut buf,
                0x100,
                false,
                AdaptationField::default(),
                &payload,
                &mut cc,
            );
            assert_eq!(buf[3] & 0x0F, expected_cc, "iteration {}", expected_cc);
        }
        // After 16 increments, counter wraps back to 0.
        write_packet(
            &mut buf,
            0x100,
            false,
            AdaptationField::default(),
            &payload,
            &mut cc,
        );
        assert_eq!(buf[3] & 0x0F, 0);
    }

    #[test]
    fn continuity_counter_separate_per_pid() {
        let mut cc = ContinuityCounters::new();
        let payload = vec![0u8; 200];
        let mut buf = [0u8; 188];

        // PID 0x100 advances to 1
        write_packet(
            &mut buf,
            0x100,
            false,
            AdaptationField::default(),
            &payload,
            &mut cc,
        );
        // PID 0x200 should still be at 0
        write_packet(
            &mut buf,
            0x200,
            false,
            AdaptationField::default(),
            &payload,
            &mut cc,
        );
        assert_eq!(buf[3] & 0x0F, 0);
    }

    #[test]
    fn adaptation_random_access_flag() {
        let mut cc = ContinuityCounters::new();
        let payload = vec![0u8; 200];
        let mut buf = [0u8; 188];
        let af = AdaptationField {
            random_access: true,
            pcr: None,
        };
        write_packet(&mut buf, 0x1011, true, af, &payload, &mut cc);
        // afc = 11
        assert_eq!(buf[3] >> 4, 0b11);
        // AF length = 1 (just the flags byte)
        assert_eq!(buf[4], 1);
        // Flags byte: random_access bit (0x40)
        assert_eq!(buf[5], 0x40);
    }

    #[test]
    fn adaptation_with_pcr_layout() {
        let mut cc = ContinuityCounters::new();
        let payload = vec![0u8; 200];
        let mut buf = [0u8; 188];
        let pcr = Pcr27mhz(0); // base=0, ext=0
        let af = AdaptationField {
            random_access: false,
            pcr: Some(pcr),
        };
        write_packet(&mut buf, 0x1011, false, af, &payload, &mut cc);
        // afc = 11
        assert_eq!(buf[3] >> 4, 0b11);
        // AF length = 1 flag byte + 6 PCR bytes = 7
        assert_eq!(buf[4], 7);
        // Flags byte: PCR_flag (0x10)
        assert_eq!(buf[5], 0x10);
        // PCR bytes: all zero base+ext, but byte 4 has the reserved 0x7E
        // bits set per spec.
        assert_eq!(buf[6], 0x00);
        assert_eq!(buf[7], 0x00);
        assert_eq!(buf[8], 0x00);
        assert_eq!(buf[9], 0x00);
        assert_eq!(buf[10], 0x7E); // (base_low=0)<<7 | 0x7E reserved | ext_high=0
        assert_eq!(buf[11], 0x00);
    }

    #[test]
    fn pcr_encoding_round_trip() {
        // Build a PCR with known base and extension, encode it, decode it,
        // verify match.
        let base: u64 = 0x1_2345_6789;
        let ext: u32 = 250;
        let pcr = Pcr27mhz(base * 300 + ext as u64);
        let mut buf = [0u8; 6];
        write_pcr(&mut buf, pcr);
        let dec_base = ((buf[0] as u64) << 25)
            | ((buf[1] as u64) << 17)
            | ((buf[2] as u64) << 9)
            | ((buf[3] as u64) << 1)
            | ((buf[4] as u64) >> 7);
        let dec_ext = (((buf[4] & 0x01) as u32) << 8) | (buf[5] as u32);
        assert_eq!(dec_base, base);
        assert_eq!(dec_ext, ext);
        // Reserved bits in byte 4: 0x7E bits should be set.
        assert_eq!(buf[4] & 0x7E, 0x7E);
    }

    #[test]
    fn stuffing_when_payload_short() {
        let mut cc = ContinuityCounters::new();
        let payload = vec![0xAA; 50];
        let mut buf = [0u8; 188];
        write_packet(
            &mut buf,
            0x1011,
            true,
            AdaptationField::default(),
            &payload,
            &mut cc,
        );
        // Without explicit AF, short payload still needs stuffing — afc = 11
        // (adaptation present) with a length covering the stuffing.
        assert_eq!(buf[3] >> 4, 0b11);
        // 188 - 4 (header) - 1 (af length byte) = 183 bytes available for
        // AF body + payload. payload is 50; stuffing is 183 - 50 = 133.
        // af_length value = 133 (the count of bytes after the length byte).
        assert_eq!(buf[4], 133);
        // Stuffing bytes are 0xFF.
        for &b in &buf[5..5 + 133] {
            assert_eq!(b, 0xFF);
        }
        // Then payload.
        for &b in &buf[138..] {
            assert_eq!(b, 0xAA);
        }
    }
}
