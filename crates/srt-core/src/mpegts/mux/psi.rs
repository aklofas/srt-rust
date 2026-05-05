//! PSI section generation — PAT and PMT.
//!
//! Both are single-section, fit in one TS packet for the current scope
//! (one program, one or two elementary streams). The CRC-32/MPEG-2 over
//! the section is computed via `crate::mpegts::common::crc32::crc32_mpeg2`.
//!
//! Padding convention: PSI packets fill unused payload bytes with `0xFF`
//! *within* the TS packet payload (after the section), not via the
//! adaptation field. The pointer field (always 0 for our single-section
//! packets) tells receivers where the section starts.

use super::ts::{AdaptationField, ContinuityCounters, write_packet};
use crate::mpegts::common::{StreamType, crc32::crc32_mpeg2, descriptor, pid};

/// Build the full 188-byte PAT packet for a single- or multi-program TS.
///
/// Iterates over all programs in `config` and encodes one `(program_number,
/// pmt_pid)` entry per program. Single-program configs produce byte-identical
/// output to the prior hardcoded single-entry implementation.
///
/// Caller passes the shared `ContinuityCounters` so the PAT CC field
/// increments correctly across successive PSI ticks.
pub(crate) fn write_pat_packet(
    out: &mut [u8; 188],
    config: &crate::mpegts::mux::Config,
    counters: &mut ContinuityCounters,
) {
    let n_programs = config.programs.len();
    // section_length = bytes after the section_length field itself through the
    // final CRC byte:
    //   tsid(2) + ver/cni(1) + sect_no(1) + last_sect(1) = 5 fixed bytes
    //   + 4 bytes per program entry (program_number(2) + reserved+pmt_pid(2))
    //   + CRC(4)
    //   = 9 + 4 * n_programs
    let section_length: usize = 9 + 4 * n_programs;

    // Full payload: pointer(1) + table_id(1) + length(2) + fixed header(5)
    //               + program loop + CRC(4) + 0xFF padding.
    let mut payload = [0xFFu8; 184];
    let mut idx = 0usize;

    payload[idx] = 0x00; // pointer field — section starts immediately after
    idx += 1;
    // table_id = 0x00 (PAT)
    payload[idx] = 0x00;
    idx += 1;
    // section_syntax_indicator=1 | '0' | reserved=11 | section_length(12 bits)
    payload[idx] = 0xB0 | (((section_length >> 8) as u8) & 0x0F);
    payload[idx + 1] = (section_length & 0xFF) as u8;
    idx += 2;
    // transport_stream_id = 0x0001
    payload[idx] = 0x00;
    payload[idx + 1] = 0x01;
    idx += 2;
    // version_number(5)=0 | current_next_indicator=1 → 0xC1 (reserved bits 11)
    payload[idx] = 0xC1;
    idx += 1;
    payload[idx] = 0x00; // section_number
    payload[idx + 1] = 0x00; // last_section_number
    idx += 2;

    // Program loop: one 4-byte entry per program.
    let body_start = 1; // pointer byte excluded from CRC body; table_id is at payload[1]
    for prog in &config.programs {
        payload[idx] = (prog.program_number >> 8) as u8;
        payload[idx + 1] = (prog.program_number & 0xFF) as u8;
        idx += 2;
        // reserved(3 bits) = 0b111 | pmt_pid(13 bits)
        payload[idx] = 0xE0 | ((prog.pmt_pid >> 8) as u8 & 0x1F);
        payload[idx + 1] = (prog.pmt_pid & 0xFF) as u8;
        idx += 2;
    }

    // CRC over section body: from table_id through last program entry byte.
    let crc = crc32_mpeg2(&payload[body_start..idx]);
    payload[idx] = (crc >> 24) as u8;
    payload[idx + 1] = (crc >> 16) as u8;
    payload[idx + 2] = (crc >> 8) as u8;
    payload[idx + 3] = crc as u8;
    // Bytes after CRC remain 0xFF from the initialiser.

    write_packet(
        out,
        pid::PAT,
        true,
        AdaptationField::default(),
        &payload,
        counters,
    );
}

/// PMT entry for one elementary stream.
pub(crate) struct PmtStreamEntry<'a> {
    pub stream_type: StreamType,
    pub elementary_pid: u16,
    /// Pre-composed descriptor-loop bytes for this ES (already concatenated
    /// across auto-emitted + caller-supplied descriptors). Empty slice
    /// means no descriptors. Owned by the [`crate::mpegts::mux::Muxer`]'s
    /// per-stream cache, borrowed at PMT emission time.
    pub descriptors: &'a [u8],
}

/// Pre-built KLVA registration descriptor body.
/// descriptor_tag(1) + descriptor_length(1) + format_identifier(4)
pub(crate) const KLVA_REGISTRATION_DESCRIPTOR: &[u8] = &[
    descriptor::REGISTRATION,
    4,
    descriptor::KLVA[0],
    descriptor::KLVA[1],
    descriptor::KLVA[2],
    descriptor::KLVA[3],
];

/// Maximum PMT section body size that fits in one 188-byte TS packet.
/// 188 - 4 (TS header) - 1 (pointer field) = 183 bytes.
pub(crate) const MAX_PMT_SECTION_BYTES: usize = 183;

/// Estimate the PMT section body size (bytes) for a `ProgramConfig`.
///
/// Used by `Config::validate()` to reject configurations that would produce
/// a PMT section too large for one TS packet. Accounts for the pre-composed
/// descriptor cache bytes (KLVA auto-emit + caller-supplied).
///
/// The estimate is exact for the expected common case (no program-level
/// descriptors — `program_info_length = 0`), which matches our current
/// output. If Task 4 adds program-level descriptors, this estimate must
/// also account for `prog.program_descriptors`.
pub(crate) fn estimate_pmt_section_size(prog: &crate::mpegts::mux::ProgramConfig) -> usize {
    let mut es_loop_size: usize = 0;
    for (i, _spec) in prog.streams.iter().enumerate() {
        // stream_type(1) + reserved+ES_PID(2) + reserved+ES_info_length(2) + descriptor bytes.
        // Descriptor bytes = sum of caller-supplied TLV lengths (KLVA auto-emit is included
        // in stream_descriptors for validated configs, but for the size check we must count
        // both auto-emitted and caller-supplied). The pre-composed cache bytes live in the
        // Muxer struct, not in ProgramConfig — so here we only count caller-supplied bytes,
        // and add KLVA auto-emit size (6 bytes) when applicable.
        let caller_descs_len: usize = prog.stream_descriptors[i].iter().map(|d| d.len()).sum();
        let auto_klva_len = match &prog.streams[i] {
            crate::mpegts::mux::StreamSpec::Klv {
                stream_type: crate::mpegts::mux::KlvStreamType::PrivateData,
                ..
            } => {
                // Auto-emit KLVA Registration (6 bytes) unless caller already supplies one.
                let caller_has_reg = prog.stream_descriptors[i]
                    .iter()
                    .any(|d| !d.is_empty() && d[0] == 0x05);
                if caller_has_reg { 0 } else { 6 }
            }
            _ => 0,
        };
        es_loop_size += 5 + caller_descs_len + auto_klva_len;
    }
    // table_id(1) + section_syntax+length(2) + program_number(2) +
    // ver+curr(1) + section_number(1) + last_section_number(1) +
    // reserved+PCR_PID(2) + reserved+program_info_length(2) + es_loop + CRC(4)
    3 + 9 + es_loop_size + 4
}

/// Build the full 188-byte PMT packet for one program.
///
/// `prog.pmt_pid` is used as the TS packet PID (the PAT points receivers
/// here). `prog.program_number` is encoded in the PMT section header.
/// `pcr_pid` is the resolved PCR PID for this program (may differ from
/// `prog.pcr_pid` if auto-fallback was applied at `Muxer::new` time).
/// `streams` is the list of ES entries — order is preserved in the PMT.
pub(crate) fn write_pmt_packet(
    out: &mut [u8; 188],
    prog: &crate::mpegts::mux::ProgramConfig,
    pcr_pid: u16,
    streams: &[PmtStreamEntry<'_>],
    counters: &mut ContinuityCounters,
) -> Result<(), crate::error::MuxError> {
    // Compute the section body size first so we can fill in section_length.
    // PMT body layout:
    //   table_id(1) + section_syntax+length(2) + program_number(2) +
    //   ver+curr(1) + section_number(1) + last_section_number(1) +
    //   reserved+PCR_PID(2) + reserved+program_info_length(2) +
    //   [program_info_descriptors=0] +
    //   ES loop entries +
    //   CRC(4)
    let mut es_loop_size: usize = 0;
    for s in streams {
        // stream_type(1) + reserved+ES_PID(2) + reserved+ES_info_length(2) + descriptors
        es_loop_size += 5 + s.descriptors.len();
    }
    // section_length covers everything after itself: 9 (program/ver/sect/PCR/info_len header)
    // + es_loop_size + 4 (CRC).
    let section_length: u16 = 9 + es_loop_size as u16 + 4;
    let total_body_size: usize = 3 + section_length as usize; // table_id + length_bytes + section_length

    // PMT must fit in one TS packet. 188 - 4 (TS header) - 1 (pointer) = 183.
    if total_body_size > 183 {
        return Err(crate::error::MuxError::PmtTooLarge {
            used_bytes: total_body_size,
            max_bytes: 183,
        });
    }

    // Full 184-byte payload: pointer(1) + body(total_body_size) + 0xFF padding.
    let mut payload = [0xFFu8; 184];
    payload[0] = 0x00; // pointer field
    let body_start = 1;
    let mut idx = body_start;

    payload[idx] = 0x02; // table_id = PMT
    idx += 1;
    payload[idx] = 0xB0 | (((section_length >> 8) as u8) & 0x0F);
    payload[idx + 1] = (section_length & 0xFF) as u8;
    idx += 2;
    // program_number — use the actual program number from the config (not a constant).
    payload[idx] = (prog.program_number >> 8) as u8;
    payload[idx + 1] = (prog.program_number & 0xFF) as u8;
    idx += 2;
    // version+current
    payload[idx] = 0xC1;
    idx += 1;
    payload[idx] = 0x00; // section_number
    payload[idx + 1] = 0x00; // last_section_number
    idx += 2;
    // reserved(3) + PCR_PID(13)
    payload[idx] = 0xE0 | ((pcr_pid >> 8) as u8 & 0x1F);
    payload[idx + 1] = (pcr_pid & 0xFF) as u8;
    idx += 2;
    // reserved(4) + program_info_length(12) — always 0 (no program-level descriptors)
    payload[idx] = 0xF0;
    payload[idx + 1] = 0x00;
    idx += 2;

    // ES loop
    for s in streams {
        payload[idx] = s.stream_type.as_u8();
        idx += 1;
        payload[idx] = 0xE0 | ((s.elementary_pid >> 8) as u8 & 0x1F);
        payload[idx + 1] = (s.elementary_pid & 0xFF) as u8;
        idx += 2;
        let es_info_length = s.descriptors.len() as u16;
        payload[idx] = 0xF0 | (((es_info_length >> 8) as u8) & 0x0F);
        payload[idx + 1] = (es_info_length & 0xFF) as u8;
        idx += 2;
        payload[idx..idx + s.descriptors.len()].copy_from_slice(s.descriptors);
        idx += s.descriptors.len();
    }

    // CRC over body (table_id through last ES descriptor byte).
    let crc = crc32_mpeg2(&payload[body_start..idx]);
    payload[idx] = (crc >> 24) as u8;
    payload[idx + 1] = (crc >> 16) as u8;
    payload[idx + 2] = (crc >> 8) as u8;
    payload[idx + 3] = crc as u8;
    // Bytes after the CRC remain 0xFF from the initialiser.

    // Emit on prog.pmt_pid — the PAT entry points receivers to this PID.
    write_packet(
        out,
        prog.pmt_pid,
        true,
        AdaptationField::default(),
        &payload,
        counters,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpegts::common::pid;
    use crate::mpegts::mux::{Config, KlvStreamType, ProgramConfig, StreamSpec, VideoCodec};

    /// Build a minimal single-program config for unit tests.
    fn single_prog_config() -> Config {
        Config::default()
    }

    /// Build a minimal ProgramConfig for tests that call write_pmt_packet directly.
    fn prog_config_h264_klv() -> ProgramConfig {
        ProgramConfig {
            program_number: 1,
            pmt_pid: 0x1000,
            streams: vec![
                StreamSpec::Video {
                    pid: 0x1011,
                    codec: VideoCodec::H264,
                },
                StreamSpec::Klv {
                    pid: 0x1031,
                    stream_type: KlvStreamType::PrivateData,
                    carries_pts: false,
                },
            ],
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: vec![Vec::new(), Vec::new()],
        }
    }

    /// Re-decode a PAT from our generated bytes and assert PMT PID matches.
    #[test]
    fn pat_round_trips() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let cfg = single_prog_config();
        write_pat_packet(&mut buf, &cfg, &mut cc);

        // TS header sanity
        assert_eq!(buf[0], 0x47);
        // PID 0
        let pid_lo = buf[2];
        let pid_hi = buf[1] & 0x1F;
        assert_eq!(((pid_hi as u16) << 8) | pid_lo as u16, pid::PAT);
        // PUSI set
        assert_eq!(buf[1] & 0x40, 0x40);
        // afc = 01 (no AF, just payload — section + 0xFF padding inside payload)
        assert_eq!(buf[3] >> 4, 0b01);

        // Section starts at byte 4 (no AF) + 1 (pointer) = byte 5.
        assert_eq!(buf[4], 0x00); // pointer = 0
        assert_eq!(buf[5], 0x00); // table_id = PAT

        // Single program: body is 12 bytes (table_id through last byte before CRC) at buf[5..17].
        // CRC at buf[17..21].
        let body = &buf[5..17];
        let crc_computed = crc32_mpeg2(body);
        let crc_in_packet = ((buf[17] as u32) << 24)
            | ((buf[18] as u32) << 16)
            | ((buf[19] as u32) << 8)
            | (buf[20] as u32);
        assert_eq!(crc_computed, crc_in_packet);

        // PMT PID embedded in last 2 bytes of program loop = body[10..12] = buf[15..17].
        let pmt_pid = (((buf[15] as u16) & 0x1F) << 8) | (buf[16] as u16);
        assert_eq!(
            pmt_pid, 0x1000,
            "single-program PAT must list pmt_pid=0x1000"
        );

        // Padding after CRC should be 0xFF (sanity check).
        assert_eq!(buf[21], 0xFF);
        assert_eq!(buf[187], 0xFF);
    }

    #[test]
    fn pmt_round_trips_with_klva() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let prog = prog_config_h264_klv();
        let streams = [
            PmtStreamEntry {
                stream_type: StreamType::H264,
                elementary_pid: 0x1011,
                descriptors: &[],
            },
            PmtStreamEntry {
                stream_type: StreamType::KlvPrivate,
                elementary_pid: 0x1031,
                descriptors: KLVA_REGISTRATION_DESCRIPTOR,
            },
        ];
        write_pmt_packet(&mut buf, &prog, 0x1011, &streams, &mut cc).expect("PMT fits");

        assert_eq!(buf[0], 0x47);
        // PMT PID — must match prog.pmt_pid (0x1000)
        let pid_value = (((buf[1] as u16) & 0x1F) << 8) | (buf[2] as u16);
        assert_eq!(pid_value, 0x1000, "PMT packet PID must equal prog.pmt_pid");
        // afc = 01 (no AF)
        assert_eq!(buf[3] >> 4, 0b01);
        // pointer = 0
        assert_eq!(buf[4], 0x00);
        // table_id = PMT
        assert_eq!(buf[5], 0x02);

        // PCR_PID at body[8..10] = buf[13..15].
        let pcr_pid = (((buf[13] as u16) & 0x1F) << 8) | (buf[14] as u16);
        assert_eq!(pcr_pid, 0x1011);

        // ES loop starts at buf[17] (after program_info_length=0 at body[10..12] = buf[15..17]).
        // Entry 1: stream_type=0x1B, ES_PID=0x1011, ES_info_length=0
        assert_eq!(buf[17], 0x1B);
        let es_pid_1 = (((buf[18] as u16) & 0x1F) << 8) | (buf[19] as u16);
        assert_eq!(es_pid_1, 0x1011);
        let es_info_len_1 = (((buf[20] as u16) & 0x0F) << 8) | (buf[21] as u16);
        assert_eq!(es_info_len_1, 0);

        // Entry 2: stream_type=0x06, ES_PID=0x1031, ES_info_length=6 (KLVA descriptor)
        assert_eq!(buf[22], 0x06);
        let es_pid_2 = (((buf[23] as u16) & 0x1F) << 8) | (buf[24] as u16);
        assert_eq!(es_pid_2, 0x1031);
        let es_info_len_2 = (((buf[25] as u16) & 0x0F) << 8) | (buf[26] as u16);
        assert_eq!(es_info_len_2, 6);
        // KLVA descriptor: 0x05, 0x04, 'K', 'L', 'V', 'A'
        assert_eq!(&buf[27..33], &[0x05, 0x04, 0x4B, 0x4C, 0x56, 0x41]);
    }

    #[test]
    fn pmt_with_sync_klv_stream_type() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let prog = ProgramConfig {
            program_number: 1,
            pmt_pid: 0x1000,
            streams: vec![
                StreamSpec::Video {
                    pid: 0x1011,
                    codec: VideoCodec::H265,
                },
                StreamSpec::Klv {
                    pid: 0x1031,
                    stream_type: KlvStreamType::SynchronousMetadata,
                    carries_pts: true,
                },
            ],
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: vec![Vec::new(), Vec::new()],
        };
        let streams = [
            PmtStreamEntry {
                stream_type: StreamType::H265,
                elementary_pid: 0x1011,
                descriptors: &[],
            },
            PmtStreamEntry {
                stream_type: StreamType::KlvSyncMetadata,
                elementary_pid: 0x1031,
                descriptors: KLVA_REGISTRATION_DESCRIPTOR,
            },
        ];
        write_pmt_packet(&mut buf, &prog, 0x1011, &streams, &mut cc).expect("PMT fits");
        // H.265 stream_type
        assert_eq!(buf[17], 0x24);
        // Sync KLV stream_type
        assert_eq!(buf[22], 0x15);
    }

    #[test]
    fn pmt_emits_h266_stream_type_0x33() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let prog = ProgramConfig {
            program_number: 1,
            pmt_pid: 0x1000,
            streams: vec![StreamSpec::Video {
                pid: 0x1011,
                codec: VideoCodec::H266,
            }],
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: vec![Vec::new()],
        };
        let streams = [PmtStreamEntry {
            stream_type: StreamType::H266,
            elementary_pid: 0x1011,
            descriptors: &[],
        }];
        write_pmt_packet(&mut buf, &prog, 0x1011, &streams, &mut cc).expect("PMT fits");
        // ES loop entry 1 stream_type byte at buf[17] — same offset as
        // pmt_round_trips_with_klva (single-program, no program-info loop).
        assert_eq!(
            buf[17], 0x33,
            "expected H.266 stream_type 0x33, got {:#x}",
            buf[17]
        );
    }

    #[test]
    fn pmt_too_large_returns_error() {
        use crate::error::MuxError;
        use crate::mpegts::common::StreamType;

        // Build 4 streams, each carrying a 255-byte descriptor blob.
        // write_pmt_packet doesn't validate TLV well-formedness — it only
        // checks total section size. Sum: 4 * (5 ES-header + 255 desc) =
        // 1040 bytes — far beyond the 183-byte single-packet PMT budget.
        let big_desc = vec![0xFFu8; 255];
        let entries = [
            PmtStreamEntry {
                stream_type: StreamType::H264,
                elementary_pid: 0x100,
                descriptors: &big_desc,
            },
            PmtStreamEntry {
                stream_type: StreamType::H264,
                elementary_pid: 0x101,
                descriptors: &big_desc,
            },
            PmtStreamEntry {
                stream_type: StreamType::H264,
                elementary_pid: 0x102,
                descriptors: &big_desc,
            },
            PmtStreamEntry {
                stream_type: StreamType::H264,
                elementary_pid: 0x103,
                descriptors: &big_desc,
            },
        ];
        let prog = ProgramConfig {
            program_number: 1,
            pmt_pid: 0x1000,
            streams: Vec::new(), // streams field unused by write_pmt_packet directly
            pcr_pid: None,
            program_descriptors: Vec::new(),
            stream_descriptors: Vec::new(),
        };
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let err = write_pmt_packet(&mut buf, &prog, 0x100, &entries, &mut cc).unwrap_err();
        assert!(matches!(err, MuxError::PmtTooLarge { .. }));
    }

    #[test]
    fn pmt_crc_validates() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let prog = prog_config_h264_klv();
        let streams = [PmtStreamEntry {
            stream_type: StreamType::H264,
            elementary_pid: 0x1011,
            descriptors: &[],
        }];
        write_pmt_packet(&mut buf, &prog, 0x1011, &streams, &mut cc).expect("PMT fits");

        // section_length is at bytes 6-7
        let section_length = (((buf[6] as u16) & 0x0F) << 8) | (buf[7] as u16);
        // Body for CRC starts at buf[5] (table_id). Total section bytes from
        // buf[5] = 3 (table_id + length_bytes) + section_length. CRC is the last
        // 4 of those.
        let body_end = 5 + 3 + section_length as usize - 4;
        let body = &buf[5..body_end];
        let crc_computed = crc32_mpeg2(body);
        let crc_in_packet = ((buf[body_end] as u32) << 24)
            | ((buf[body_end + 1] as u32) << 16)
            | ((buf[body_end + 2] as u32) << 8)
            | (buf[body_end + 3] as u32);
        assert_eq!(crc_computed, crc_in_packet);
    }
}
