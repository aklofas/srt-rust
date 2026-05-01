//! Extract per-AU video binaries from a real `.ts` file.
//!
//! Usage:
//!   cargo run --example extract_video_au -- <input.ts> [out_dir]
//!
//! Output:
//!   <out_dir>/au_<idx>_<pts>.bin — one Annex-B access unit per file.
//!   <out_dir>/manifest.txt — index, PTS, codec, size, key_frame flag.
//!
//! Companion to `examples/extract_klv.rs`. Both pull per-frame data from
//! real STANAG 4609 captures so integration tests can replay against
//! production-shaped inputs.

use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: extract_video_au <input.ts> [out_dir]");
        std::process::exit(2);
    }
    let input = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(args.get(2).cloned().unwrap_or_else(|| "video_aus".into()));
    fs::create_dir_all(&out_dir)?;

    let mut data = Vec::new();
    File::open(&input)?.read_to_end(&mut data)?;

    let aus = extract_aus(&data);
    let mut manifest = File::create(out_dir.join("manifest.txt"))?;
    writeln!(manifest, "# idx pts size key_frame stream_type")?;
    for (i, au) in aus.iter().enumerate() {
        let path = out_dir.join(format!("au_{:04}_{}.bin", i, au.pts.unwrap_or(0)));
        File::create(&path)?.write_all(&au.bytes)?;
        writeln!(
            manifest,
            "{} {} {} {} {:#04x}",
            i,
            au.pts.unwrap_or(0),
            au.bytes.len(),
            au.key_frame,
            au.stream_type
        )?;
    }
    println!(
        "extracted {} access units to {}",
        aus.len(),
        out_dir.display()
    );
    Ok(())
}

#[derive(Debug)]
struct Au {
    pts: Option<u64>,
    bytes: Vec<u8>,
    key_frame: bool,
    stream_type: u8,
}

fn extract_aus(data: &[u8]) -> Vec<Au> {
    let mut pmt_pid: Option<u16> = None;
    let mut video_pids: HashMap<u16, u8> = HashMap::new(); // PID -> stream_type
    let mut pes_state: HashMap<u16, (Option<u64>, Vec<u8>)> = HashMap::new();
    let mut aus: Vec<Au> = Vec::new();

    for pkt in data.chunks_exact(188) {
        if pkt[0] != 0x47 {
            continue;
        }
        let pusi = (pkt[1] & 0x40) != 0;
        let pid = (((pkt[1] as u16) & 0x1F) << 8) | (pkt[2] as u16);
        let afc = (pkt[3] >> 4) & 0x3;
        let mut payload_offset = 4;
        if afc & 0x2 != 0 {
            payload_offset = 5 + pkt[4] as usize;
        }
        if afc & 0x1 == 0 || payload_offset >= 188 {
            continue;
        }
        let payload = &pkt[payload_offset..];

        if pid == 0 && pusi {
            let ptr = payload[0] as usize;
            pmt_pid = parse_pat_pmt_pid(&payload[1 + ptr..]);
        } else if Some(pid) == pmt_pid && pusi {
            let ptr = payload[0] as usize;
            video_pids = parse_pmt_video_pids(&payload[1 + ptr..]);
        } else if let Some(&st) = video_pids.get(&pid) {
            if pusi {
                if let Some((pts, buf)) = pes_state.remove(&pid) {
                    let key_frame = scan_idr(&buf, st);
                    aus.push(Au {
                        pts,
                        bytes: buf,
                        key_frame,
                        stream_type: st,
                    });
                }
                let (pts, body) = parse_pes_start(payload);
                pes_state.insert(pid, (pts, body.to_vec()));
            } else if let Some(entry) = pes_state.get_mut(&pid) {
                entry.1.extend_from_slice(payload);
            }
        }
    }

    // Flush any remaining in-progress AUs.
    for (pid, (pts, buf)) in pes_state {
        if let Some(&st) = video_pids.get(&pid) {
            let key_frame = scan_idr(&buf, st);
            aus.push(Au {
                pts,
                bytes: buf,
                key_frame,
                stream_type: st,
            });
        }
    }
    aus
}

fn parse_pat_pmt_pid(body: &[u8]) -> Option<u16> {
    if body.len() < 12 {
        return None;
    }
    let section_length = (((body[1] as u16) & 0x0F) << 8) | (body[2] as u16);
    let mut i = 8;
    let end = 3 + section_length as usize - 4;
    while i + 4 <= end {
        let prog = ((body[i] as u16) << 8) | (body[i + 1] as u16);
        let pid = (((body[i + 2] as u16) & 0x1F) << 8) | (body[i + 3] as u16);
        if prog != 0 {
            return Some(pid);
        }
        i += 4;
    }
    None
}

fn parse_pmt_video_pids(body: &[u8]) -> HashMap<u16, u8> {
    let mut out = HashMap::new();
    let section_length = (((body[1] as u16) & 0x0F) << 8) | (body[2] as u16);
    let program_info_len = (((body[10] as u16) & 0x0F) << 8) | (body[11] as u16);
    let mut i = 12 + program_info_len as usize;
    let end = 3 + section_length as usize - 4;
    while i + 5 <= end {
        let st = body[i];
        let pid = (((body[i + 1] as u16) & 0x1F) << 8) | (body[i + 2] as u16);
        let es_info_len = (((body[i + 3] as u16) & 0x0F) << 8) | (body[i + 4] as u16);
        if st == 0x1B || st == 0x24 {
            out.insert(pid, st);
        }
        i = i + 5 + es_info_len as usize;
    }
    out
}

fn parse_pes_start(buf: &[u8]) -> (Option<u64>, &[u8]) {
    if buf.len() < 9 || buf[..3] != [0x00, 0x00, 0x01] {
        return (None, buf);
    }
    let pts_dts_flags = (buf[7] >> 6) & 0x03;
    let header_data_length = buf[8] as usize;
    let body_start = 9 + header_data_length;
    let pts = if pts_dts_flags & 0x02 != 0 && buf.len() >= 14 {
        let p = &buf[9..14];
        let b0 = p[0] as u64;
        let b1 = p[1] as u64;
        let b2 = p[2] as u64;
        let b3 = p[3] as u64;
        let b4 = p[4] as u64;
        Some(
            (((b0 >> 1) & 0x07) << 30)
                | (b1 << 22)
                | (((b2 >> 1) & 0x7F) << 15)
                | (b3 << 7)
                | ((b4 >> 1) & 0x7F),
        )
    } else {
        None
    };
    if buf.len() < body_start {
        return (pts, &[]);
    }
    (pts, &buf[body_start..])
}

fn scan_idr(buf: &[u8], stream_type: u8) -> bool {
    // Annex-B start codes; for H.264 IDR slice NAL type = 5; for H.265 IDR_W_RADL = 19.
    let target = if stream_type == 0x1B { 5u8 } else { 19u8 };
    let mut i = 0;
    while i + 5 < buf.len() {
        if buf[i] == 0
            && buf[i + 1] == 0
            && (buf[i + 2] == 1 || (buf[i + 2] == 0 && buf[i + 3] == 1))
        {
            let nh = if buf[i + 2] == 1 {
                buf[i + 3]
            } else {
                buf[i + 4]
            };
            let nal_type = if stream_type == 0x1B {
                nh & 0x1F
            } else {
                (nh >> 1) & 0x3F
            };
            if nal_type == target {
                return true;
            }
        }
        i += 1;
    }
    false
}
