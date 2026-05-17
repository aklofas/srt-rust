//! Minimal MPEG-TS parser for integration tests.
//!
//! Reads muxed bytes from `Muxer::pull`, identifies PAT and PMT, reassembles
//! PES on a chosen PID, extracts the payload and (if present) the PTS. Not
//! intended for production use — error recovery is minimal, only this
//! crate's mux output shape is supported.

use std::collections::HashMap;
use tst_core::mpegts::common::{TS_PACKET_SIZE, TS_SYNC_BYTE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmtStream {
    pub pid: u16,
    pub stream_type: u8,
    pub klva: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedStream {
    pub pmt_pid: Option<u16>,
    pub pcr_pid: Option<u16>,
    pub streams: Vec<PmtStream>,
    /// Per-PID list of (PTS_or_None, payload_bytes) PES units, in order.
    #[allow(clippy::type_complexity)]
    pub pes_by_pid: HashMap<u16, Vec<(Option<u64>, Vec<u8>)>>,
    /// Sequence of PCR samples observed in adaptation fields, in TS-packet order.
    /// Each entry is `(pid, pcr_27mhz)`.
    pub pcr_samples: Vec<(u16, u64)>,
}

/// Parse a buffer of TS packets emitted by our muxer.
pub fn parse(buf: &[u8]) -> ParsedStream {
    let mut state = ParsedStream::default();
    // Per-PID PES reassembly state: (pts, accumulated_payload).
    let mut pes_state: HashMap<u16, (Option<u64>, Vec<u8>)> = HashMap::new();
    // After PMT is parsed, we know which PIDs are PES streams.
    let mut pes_pids: Vec<u16> = Vec::new();

    for pkt in buf.chunks_exact(TS_PACKET_SIZE) {
        if pkt[0] != TS_SYNC_BYTE {
            panic!("invalid sync byte at packet boundary");
        }
        let pusi = (pkt[1] & 0x40) != 0;
        let pid = (((pkt[1] as u16) & 0x1F) << 8) | (pkt[2] as u16);
        let afc = (pkt[3] >> 4) & 0x3;

        let mut payload_offset = 4;
        if afc & 0x2 != 0 {
            let af_len = pkt[4] as usize;
            if af_len > 0 {
                let flags = pkt[5];
                let pcr_present = (flags & 0x10) != 0;
                if pcr_present && af_len >= 7 {
                    state.pcr_samples.push((pid, decode_pcr(&pkt[6..12])));
                }
            }
            payload_offset = 5 + af_len;
        }
        if afc & 0x1 == 0 || payload_offset >= TS_PACKET_SIZE {
            continue;
        }
        let payload = &pkt[payload_offset..];

        match pid {
            0x0000 if pusi => {
                // PAT — extract first PMT PID.
                let ptr = payload[0] as usize;
                let body = &payload[1 + ptr..];
                state.pmt_pid = parse_pat_first_pmt_pid(body);
            }
            0x0000 => {}
            p if Some(p) == state.pmt_pid && pusi => {
                // PMT — populate streams + pcr_pid.
                let ptr = payload[0] as usize;
                let body = &payload[1 + ptr..];
                let parsed = parse_pmt(body);
                state.pcr_pid = Some(parsed.0);
                state.streams = parsed.1;
                pes_pids = state.streams.iter().map(|s| s.pid).collect();
            }
            p if Some(p) == state.pmt_pid => {}
            p if pes_pids.contains(&p) => {
                if pusi {
                    // Flush any in-progress PES on this PID.
                    if let Some((pts, buf)) = pes_state.remove(&p) {
                        state.pes_by_pid.entry(p).or_default().push((pts, buf));
                    }
                    // Start a new PES.
                    let (pts, body) = parse_pes_start(payload);
                    pes_state.insert(p, (pts, body.to_vec()));
                } else if let Some(entry) = pes_state.get_mut(&p) {
                    entry.1.extend_from_slice(payload);
                }
            }
            _ => {}
        }
    }

    // Flush remaining in-progress PES.
    for (p, (pts, buf)) in pes_state {
        state.pes_by_pid.entry(p).or_default().push((pts, buf));
    }

    state
}

fn parse_pat_first_pmt_pid(body: &[u8]) -> Option<u16> {
    // table_id(1) + section_syntax+length(2) + tsid(2) + ver+curr(1) + sect(1) + last(1)
    // Then (program_number(2) + reserved+pmt_pid(2)) entries until CRC.
    if body.len() < 12 {
        return None;
    }
    let section_length = (((body[1] as u16) & 0x0F) << 8) | (body[2] as u16);
    let loop_start = 8;
    let loop_end = 3 + section_length as usize - 4; // CRC is last 4 bytes
    let mut i = loop_start;
    while i + 4 <= loop_end {
        let program_number = ((body[i] as u16) << 8) | (body[i + 1] as u16);
        let pid = (((body[i + 2] as u16) & 0x1F) << 8) | (body[i + 3] as u16);
        if program_number != 0 {
            return Some(pid);
        }
        i += 4;
    }
    None
}

/// Returns (pcr_pid, list_of_streams).
fn parse_pmt(body: &[u8]) -> (u16, Vec<PmtStream>) {
    // Layout: table_id(1) + section_syntax+length(2) + program_number(2) +
    //         ver+curr(1) + section(1) + last(1) + reserved+PCR_PID(2) +
    //         reserved+program_info_length(2) + program_info_descriptors +
    //         ES loop entries + CRC(4).
    let section_length = (((body[1] as u16) & 0x0F) << 8) | (body[2] as u16);
    let pcr_pid = (((body[8] as u16) & 0x1F) << 8) | (body[9] as u16);
    let program_info_length = (((body[10] as u16) & 0x0F) << 8) | (body[11] as u16);
    let mut i = 12 + program_info_length as usize;
    let loop_end = (3 + section_length as usize - 4).min(body.len()); // CRC, clamped

    let mut streams = Vec::new();
    while i + 5 <= loop_end {
        let stream_type = body[i];
        let pid = (((body[i + 1] as u16) & 0x1F) << 8) | (body[i + 2] as u16);
        let es_info_length = (((body[i + 3] as u16) & 0x0F) << 8) | (body[i + 4] as u16);
        let descriptors_start = i + 5;
        // Real-world PMTs occasionally declare ES-descriptor lengths that
        // overrun the section bound. Clamp so the test helper survives —
        // production demux handles this in its own parser.
        let descriptors_end = (descriptors_start + es_info_length as usize).min(loop_end);
        let descriptors = &body[descriptors_start..descriptors_end];
        let klva = scan_for_klva(descriptors);
        streams.push(PmtStream {
            pid,
            stream_type,
            klva,
        });
        i = descriptors_end;
    }
    (pcr_pid, streams)
}

fn scan_for_klva(descriptors: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 <= descriptors.len() {
        let tag = descriptors[i];
        let len = descriptors[i + 1] as usize;
        let body_start = i + 2;
        let body_end = body_start + len;
        if tag == 0x05 && len == 4 && &descriptors[body_start..body_end] == b"KLVA" {
            return true;
        }
        i = body_end;
    }
    false
}

/// Parse a PES packet's start (after PUSI=1). Returns (pts, body_after_header).
fn parse_pes_start(buf: &[u8]) -> (Option<u64>, &[u8]) {
    debug_assert!(buf.len() >= 9);
    debug_assert!(buf[..3] == [0x00, 0x00, 0x01]);
    // stream_id at buf[3]; PES_packet_length at buf[4..6]; flags1 buf[6];
    // flags2 buf[7]; PES_header_data_length buf[8].
    let pts_dts_flags = (buf[7] >> 6) & 0x03;
    let header_data_length = buf[8] as usize;
    let body_start = 9 + header_data_length;
    let pts = if pts_dts_flags & 0x02 != 0 {
        let p = &buf[9..14];
        Some(decode_pts(p))
    } else {
        None
    };
    (pts, &buf[body_start..])
}

/// Decode a 6-byte PCR field (program_clock_reference) into 27 MHz units.
/// Layout: 33-bit base (90 kHz) + 6 reserved bits + 9-bit extension (27 MHz mod 300).
/// Final value = base * 300 + extension.
fn decode_pcr(buf: &[u8]) -> u64 {
    let base = ((buf[0] as u64) << 25)
        | ((buf[1] as u64) << 17)
        | ((buf[2] as u64) << 9)
        | ((buf[3] as u64) << 1)
        | ((buf[4] as u64) >> 7);
    let ext = (((buf[4] as u64) & 0x01) << 8) | (buf[5] as u64);
    base * 300 + ext
}

fn decode_pts(buf: &[u8]) -> u64 {
    let b0 = buf[0] as u64;
    let b1 = buf[1] as u64;
    let b2 = buf[2] as u64;
    let b3 = buf[3] as u64;
    let b4 = buf[4] as u64;
    (((b0 >> 1) & 0x07) << 30)
        | (b1 << 22)
        | (((b2 >> 1) & 0x7F) << 15)
        | (b3 << 7)
        | ((b4 >> 1) & 0x7F)
}
