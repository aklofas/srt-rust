//! Extract `.klv` payload blobs from an MPEG-TS file.
//!
//! Usage: `cargo run --example extract_klv -- <input.ts> [output_prefix]`
//!
//! Walks the TS, parses PAT and PMT to find the data PID with a KLV
//! registration descriptor (`0x05` registration descriptor with format
//! identifier `KLVA`), demuxes PES packets on that PID, and writes each PES
//! payload as a separate `<prefix>_NNNN.klv` file.
//!
//! This is dev tooling for assembling test fixtures from real captures.
//! Mature TS demux is deferred to `mpegts::demux` (see docs/deferred-features.md).

use std::env;
use std::fs;
use std::path::Path;

const TS_PACKET_SIZE: usize = 188;
const SYNC_BYTE: u8 = 0x47;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: extract_klv <input.ts> [output_prefix]");
        std::process::exit(2);
    }
    let input = Path::new(&args[1]);
    let default_prefix = input.file_stem().unwrap().to_string_lossy().into_owned();
    let prefix = args.get(2).map(String::as_str).unwrap_or(&default_prefix);

    let bytes = fs::read(input).expect("read input");
    let klv_pid = find_klv_pid(&bytes).expect("no KLV PID found in PMT");
    eprintln!("KLV PID = 0x{klv_pid:04X}");

    let pes_payloads = demux_pes_on_pid(&bytes, klv_pid);
    let out_dir = input.parent().unwrap_or(Path::new("."));
    for (i, payload) in pes_payloads.iter().enumerate() {
        let path = out_dir.join(format!("{prefix}_{i:04}.klv"));
        fs::write(&path, payload).unwrap();
        eprintln!("wrote {} ({} bytes)", path.display(), payload.len());
    }
    println!("extracted {} KLV blobs", pes_payloads.len());
}

/// Walk PAT then PMT to find the elementary PID whose registration descriptor
/// is `KLVA`. Returns `Some(pid)` on first match, `None` otherwise.
fn find_klv_pid(ts: &[u8]) -> Option<u16> {
    let mut pmt_pids: Vec<u16> = Vec::new();
    // Pass 1: find PMT PIDs from PAT (PID 0x0000).
    for pkt in ts.chunks_exact(TS_PACKET_SIZE) {
        if pkt[0] != SYNC_BYTE {
            continue;
        }
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        let payload_unit_start = (pkt[1] & 0x40) != 0;
        if pid != 0 || !payload_unit_start {
            continue;
        }
        // Skip pointer + section header. Naive parse: the PMT PID is bytes 10-11
        // of the section after the pointer in the simplest single-program PAT.
        let payload_start = 4 + 1; // header + pointer (assumed 0)
        if payload_start + 12 > TS_PACKET_SIZE {
            continue;
        }
        let section_len =
            (((pkt[payload_start + 1] & 0x0F) as usize) << 8) | pkt[payload_start + 2] as usize;
        // table_id at payload_start, len at +1/+2, sections start at payload_start+3
        let mut i = payload_start + 8; // skip transport_stream_id, version, etc.
        while i + 4 <= payload_start + 3 + section_len.saturating_sub(4) {
            let _program_number = u16::from_be_bytes([pkt[i], pkt[i + 1]]);
            let pmt_pid = (((pkt[i + 2] & 0x1F) as u16) << 8) | pkt[i + 3] as u16;
            pmt_pids.push(pmt_pid);
            i += 4;
        }
        break;
    }

    // Pass 2: walk PMT; find ES with KLVA registration descriptor.
    for pkt in ts.chunks_exact(TS_PACKET_SIZE) {
        if pkt[0] != SYNC_BYTE {
            continue;
        }
        let pid = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if !pmt_pids.contains(&pid) {
            continue;
        }
        let payload_unit_start = (pkt[1] & 0x40) != 0;
        if !payload_unit_start {
            continue;
        }
        let payload_start = 4 + 1; // skip header + pointer (assumed 0)
        if payload_start + 12 > TS_PACKET_SIZE {
            continue;
        }
        let section_len =
            (((pkt[payload_start + 1] & 0x0F) as usize) << 8) | pkt[payload_start + 2] as usize;
        let pi_len =
            (((pkt[payload_start + 10] & 0x0F) as usize) << 8) | pkt[payload_start + 11] as usize;
        let mut i = payload_start + 12 + pi_len;
        let end = (payload_start + 3 + section_len).min(TS_PACKET_SIZE) - 4; // exclude CRC32
        while i + 5 <= end {
            let stream_type = pkt[i];
            let es_pid = (((pkt[i + 1] & 0x1F) as u16) << 8) | pkt[i + 2] as u16;
            let es_info_len = (((pkt[i + 3] & 0x0F) as usize) << 8) | pkt[i + 4] as usize;
            let desc_start = i + 5;
            let desc_end = (desc_start + es_info_len).min(end);
            // Walk descriptors looking for tag 0x05 (registration) with
            // format_identifier "KLVA".
            let mut d = desc_start;
            while d + 2 <= desc_end {
                let tag = pkt[d];
                let len = pkt[d + 1] as usize;
                if tag == 0x05 && len >= 4 && d + 6 <= desc_end && &pkt[d + 2..d + 6] == b"KLVA" {
                    let _ = stream_type;
                    return Some(es_pid);
                }
                d += 2 + len;
            }
            i = desc_end;
        }
    }
    None
}

/// Demux PES payloads on `pid`, returning each PES's payload bytes (after the
/// PES header).
fn demux_pes_on_pid(ts: &[u8], pid: u16) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut current: Option<Vec<u8>> = None;
    for pkt in ts.chunks_exact(TS_PACKET_SIZE) {
        if pkt[0] != SYNC_BYTE {
            continue;
        }
        let p = (((pkt[1] & 0x1F) as u16) << 8) | pkt[2] as u16;
        if p != pid {
            continue;
        }
        let payload_unit_start = (pkt[1] & 0x40) != 0;
        let adaptation_field_control = (pkt[3] >> 4) & 0x3;
        let mut payload_offset = 4;
        if adaptation_field_control & 0x2 != 0 {
            // Adaptation field present
            payload_offset += 1 + pkt[4] as usize;
        }
        if adaptation_field_control & 0x1 == 0 {
            continue; // no payload
        }
        if payload_offset >= TS_PACKET_SIZE {
            continue;
        }
        let payload = &pkt[payload_offset..];

        if payload_unit_start {
            // Flush any in-progress PES
            if let Some(buf) = current.take() {
                if let Some(klv) = strip_pes_header(&buf) {
                    out.push(klv);
                }
            }
            current = Some(payload.to_vec());
        } else if let Some(buf) = current.as_mut() {
            buf.extend_from_slice(payload);
        }
    }
    if let Some(buf) = current.take() {
        if let Some(klv) = strip_pes_header(&buf) {
            out.push(klv);
        }
    }
    out
}

fn strip_pes_header(pes: &[u8]) -> Option<Vec<u8>> {
    if pes.len() < 9 || pes[..3] != [0x00, 0x00, 0x01] {
        return None;
    }
    let pes_header_data_len = pes[8] as usize;
    let payload_start = 9 + pes_header_data_len;
    if payload_start >= pes.len() {
        return None;
    }
    Some(pes[payload_start..].to_vec())
}
