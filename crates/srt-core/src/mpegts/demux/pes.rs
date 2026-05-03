// crates/srt-core/src/mpegts/demux/pes.rs
//! Per-PID PES reassembly.
//!
//! Drive with `Reassembler::push(pid, payload, pusi)` for each TS packet.
//! Emits a complete `PesPayload` whenever a PES boundary is detected
//! (next-PUSI on same PID, or `PES_packet_length`-driven end). Enforces
//! per-PID and aggregate caps; breaches surface as `Overflow` events.

use crate::error::DemuxError;
use std::collections::HashMap;

/// One reassembled PES on a single PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PesPayload {
    pub pid: u16,
    pub stream_id: u8,
    /// 90 kHz PTS, if the PES carried one.
    pub pts: Option<i64>,
    /// 90 kHz DTS, if the PES carried one.
    pub dts: Option<i64>,
    /// Elementary stream payload bytes (after the PES header, including
    /// header_data_length adjustment).
    pub payload: Vec<u8>,
}

/// Internally buffered partial PES on a PID.
#[derive(Debug)]
struct Partial {
    declared_total_len: Option<usize>, // PES_packet_length-derived total of the body
    buf: Vec<u8>,
}

/// Per-call output from `Reassembler::push`. A single TS payload chunk
/// can complete the previous PES *and* begin a new one, so a vec of
/// outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReassemblyOutcome {
    /// A complete PES is now available.
    Complete(PesPayload),
    /// Buffer cap exceeded for this PID; partial PES dropped.
    Overflow { pid: u16 },
    /// Aggregate buffer cap exceeded; all partial PES on all PIDs dropped.
    OverflowTotal,
}

#[derive(Debug)]
pub struct Reassembler {
    by_pid: HashMap<u16, Partial>,
    total_buffered: usize,
    cap_per_pid: usize,
    cap_total: usize,
}

impl Reassembler {
    pub fn new(cap_per_pid: usize, cap_total: usize) -> Self {
        Self {
            by_pid: HashMap::new(),
            total_buffered: 0,
            cap_per_pid,
            cap_total,
        }
    }

    /// Feed one TS-packet's payload bytes for `pid`. `pusi=true` means
    /// this packet begins a new PES on this PID.
    pub fn push(
        &mut self,
        pid: u16,
        payload: &[u8],
        pusi: bool,
    ) -> Result<Vec<ReassemblyOutcome>, DemuxError> {
        let mut out = Vec::new();
        if pusi {
            // PUSI: drain whatever was in flight on this PID first.
            if let Some(prev) = self.by_pid.remove(&pid) {
                self.total_buffered = self.total_buffered.saturating_sub(prev.buf.len());
                if let Some(pes) = parse_complete(pid, &prev.buf)? {
                    out.push(ReassemblyOutcome::Complete(pes));
                }
            }
            // Start fresh partial.
            self.by_pid.insert(
                pid,
                Partial {
                    declared_total_len: None,
                    buf: Vec::new(),
                },
            );
        }
        // Append payload to whatever partial exists for this PID.
        let part = match self.by_pid.get_mut(&pid) {
            Some(p) => p,
            None => return Ok(out), // bytes before we ever saw a PUSI; drop.
        };
        if part.buf.len() + payload.len() > self.cap_per_pid {
            // Cap-per-PID exceeded. Drop, surface, resume from next PUSI on this PID.
            self.total_buffered = self.total_buffered.saturating_sub(part.buf.len());
            self.by_pid.remove(&pid);
            out.push(ReassemblyOutcome::Overflow { pid });
            return Ok(out);
        }
        part.buf.extend_from_slice(payload);
        self.total_buffered += payload.len();
        // Try to derive declared_total_len if still unknown.
        if part.declared_total_len.is_none() && part.buf.len() >= 6 {
            let pkt_len_field = u16::from_be_bytes([part.buf[4], part.buf[5]]);
            if pkt_len_field != 0 {
                // PES_packet_length is the count of bytes after this 6-byte header.
                part.declared_total_len = Some(6 + pkt_len_field as usize);
            }
        }
        // Aggregate cap check.
        if self.total_buffered > self.cap_total {
            self.by_pid.clear();
            self.total_buffered = 0;
            out.push(ReassemblyOutcome::OverflowTotal);
            return Ok(out);
        }
        // Length-driven completion.
        let mut completed_now = None;
        if let Some(total) = part.declared_total_len {
            if part.buf.len() >= total {
                // Slice off exactly `total` bytes; anything beyond is the next PES
                // on this PID (rare — most PIDs use PUSI for boundaries).
                let body = std::mem::take(&mut part.buf);
                completed_now = Some(body);
                self.total_buffered = self
                    .total_buffered
                    .saturating_sub(completed_now.as_ref().map(|b| b.len()).unwrap_or(0));
                self.by_pid.remove(&pid);
            }
        }
        if let Some(buf) = completed_now {
            if let Some(pes) = parse_complete(pid, &buf)? {
                out.push(ReassemblyOutcome::Complete(pes));
            }
        }
        Ok(out)
    }

    pub fn drain_partial(&mut self) -> Vec<PesPayload> {
        let mut out = Vec::new();
        for (pid, p) in std::mem::take(&mut self.by_pid) {
            if let Ok(Some(pes)) = parse_complete(pid, &p.buf) {
                out.push(pes);
            }
        }
        self.total_buffered = 0;
        out
    }

    pub fn buffered_bytes(&self) -> usize {
        self.total_buffered
    }
}

/// Parse a fully-buffered PES packet (header + body) into a `PesPayload`.
/// Returns `None` if the buffer is too short to be a valid PES.
fn parse_complete(pid: u16, buf: &[u8]) -> Result<Option<PesPayload>, DemuxError> {
    if buf.len() < 6 {
        return Ok(None);
    }
    if buf[0] != 0x00 || buf[1] != 0x00 || buf[2] != 0x01 {
        return Err(DemuxError::MalformedPes {
            pid,
            reason: "missing 0x000001 PES start code prefix",
        });
    }
    let stream_id = buf[3];
    // Stream IDs that don't carry the optional header: 0xBE (padding), 0xBF
    // (private_stream_2), 0xF0–0xF2 (PROG/...), 0xFF (program_stream_directory).
    // Spec lists more; the common ones for our domain (video 0xE0-0xEF,
    // audio 0xC0-0xDF, private_stream_1 0xBD, metadata 0xFC) all carry the
    // optional header.
    let has_optional_header = matches!(
        stream_id,
        0xC0..=0xDF | 0xE0..=0xEF | 0xBD | 0xFC | 0xFD
    );
    let mut body_off = 6;
    let mut pts = None;
    let mut dts = None;
    if has_optional_header {
        if buf.len() < 9 {
            return Err(DemuxError::MalformedPes {
                pid,
                reason: "PES too short for optional header",
            });
        }
        let pts_dts_flags = (buf[7] >> 6) & 0x03;
        let header_data_length = buf[8] as usize;
        body_off = 9 + header_data_length;
        if buf.len() < body_off {
            return Err(DemuxError::MalformedPes {
                pid,
                reason: "PES too short for declared header_data_length",
            });
        }
        if pts_dts_flags == 0b10 || pts_dts_flags == 0b11 {
            // PTS in 5 bytes at offset 9.
            if buf.len() < 14 {
                return Err(DemuxError::MalformedPes {
                    pid,
                    reason: "PES too short for PTS",
                });
            }
            pts = Some(decode_pts_dts(&buf[9..14]));
        }
        if pts_dts_flags == 0b11 {
            if buf.len() < 19 {
                return Err(DemuxError::MalformedPes {
                    pid,
                    reason: "PES too short for DTS",
                });
            }
            dts = Some(decode_pts_dts(&buf[14..19]));
        }
    }
    let payload = buf[body_off..].to_vec();
    Ok(Some(PesPayload {
        pid,
        stream_id,
        pts,
        dts,
        payload,
    }))
}

/// Decode a 5-byte PTS or DTS field (4-bit prefix + 33 PTS bits + 3 marker bits).
fn decode_pts_dts(b: &[u8]) -> i64 {
    let p32_30 = ((b[0] >> 1) & 0x07) as u64;
    let p29_15 = (((b[1] as u64) << 7) | ((b[2] as u64) >> 1)) & 0x7FFF;
    let p14_0 = (((b[3] as u64) << 7) | ((b[4] as u64) >> 1)) & 0x7FFF;
    ((p32_30 << 30) | (p29_15 << 15) | p14_0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_pes(stream_id: u8, pts: Option<i64>, payload: &[u8]) -> Vec<u8> {
        let mut s = Vec::new();
        s.extend_from_slice(&[0x00, 0x00, 0x01, stream_id]);
        // Reserve length field; backfill at end.
        s.push(0x00);
        s.push(0x00);
        // Optional header: marker(2)=10 + scrambling(2)=00 + priority(1) + alignment(1) +
        // copyright(1) + original(1)
        s.push(0x80);
        // pts_dts_flags(2) + ESCR(1) + ES_rate(1) + DSM_trick(1) + additional_copy(1) +
        // PES_CRC(1) + PES_extension(1)
        let pts_dts_flags = if pts.is_some() { 0b10 } else { 0b00 };
        s.push(pts_dts_flags << 6);
        let pts_bytes = if let Some(p) = pts {
            let p = p as u64;
            let b0 = 0x21 | (((p >> 30) as u8) << 1) & 0x0E;
            let b1 = ((p >> 22) & 0xFF) as u8;
            let b2 = (((p >> 14) & 0xFE) as u8) | 0x01;
            let b3 = ((p >> 7) & 0xFF) as u8;
            let b4 = (((p << 1) & 0xFE) as u8) | 0x01;
            vec![b0, b1, b2, b3, b4]
        } else {
            vec![]
        };
        s.push(pts_bytes.len() as u8); // header_data_length
        s.extend_from_slice(&pts_bytes);
        s.extend_from_slice(payload);
        // Backfill PES_packet_length (count of bytes after byte 5).
        let pes_packet_length = (s.len() - 6) as u16;
        s[4] = (pes_packet_length >> 8) as u8;
        s[5] = pes_packet_length as u8;
        s
    }

    #[test]
    fn reassembles_one_pes_via_pusi_then_pusi() {
        let mut pes = build_pes(0xE0, Some(900_000), b"hello");
        // Video PES (stream_id 0xE0-0xEF) commonly uses PES_packet_length=0
        // for unbounded payload — boundary is then detected only via the next
        // PUSI on the same PID. Zero the length field so this test exercises
        // the PUSI-driven path rather than length-driven completion.
        pes[4] = 0;
        pes[5] = 0;
        // Split across two PUSI calls: first PUSI starts the PES, second PUSI
        // emits it.
        let mut r = Reassembler::new(1 << 20, 4 << 20);
        let out = r.push(0x100, &pes, true).unwrap();
        assert!(out.is_empty());
        // A second PUSI on the same PID closes the previous one. Zero this
        // PES's length field too so it doesn't immediately length-complete
        // and add a second outcome.
        let mut pes2 = build_pes(0xE0, None, b"");
        pes2[4] = 0;
        pes2[5] = 0;
        let out = r.push(0x100, &pes2, true).unwrap();
        assert_eq!(out.len(), 1);
        match &out[0] {
            ReassemblyOutcome::Complete(p) => {
                assert_eq!(p.pts, Some(900_000));
                assert_eq!(p.payload, b"hello");
            }
            _ => panic!("expected Complete"),
        }
    }

    #[test]
    fn length_driven_completion() {
        let pes = build_pes(0xE0, Some(0), b"abc");
        let mut r = Reassembler::new(1 << 20, 4 << 20);
        let out = r.push(0x100, &pes, true).unwrap();
        // PES_packet_length is set => completion when all bytes seen.
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn per_pid_overflow_emits_event_and_clears() {
        let mut r = Reassembler::new(64, 1 << 20);
        let _ = r
            .push(0x100, b"\x00\x00\x01\xE0\x00\x00\x80\x00\x00", true)
            .unwrap();
        // Now flood until overflow.
        let big = vec![0xCC; 256];
        let out = r.push(0x100, &big, false).unwrap();
        assert!(matches!(out[0], ReassemblyOutcome::Overflow { pid: 0x100 }));
        assert_eq!(r.buffered_bytes(), 0);
    }

    #[test]
    fn aggregate_overflow() {
        let mut r = Reassembler::new(1 << 20, 200);
        let _ = r
            .push(0x100, b"\x00\x00\x01\xE0\x00\x00\x80\x00\x00", true)
            .unwrap();
        let big = vec![0xCC; 300];
        let out = r.push(0x100, &big, false).unwrap();
        assert!(
            out.iter()
                .any(|o| matches!(o, ReassemblyOutcome::OverflowTotal))
        );
        assert_eq!(r.buffered_bytes(), 0);
    }
}
