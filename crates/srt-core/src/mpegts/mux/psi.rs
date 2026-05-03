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

/// Single program, fixed at program_number = 0x0001.
pub(crate) const PROGRAM_NUMBER: u16 = 0x0001;

/// PMT lives at this PID. Not configurable — receivers find it via PAT,
/// so the value is internal.
pub(crate) const PMT_PID: u16 = 0x1000;

/// Build the full 188-byte PAT packet for a single-program TS.
///
/// Caller passes a fresh `ContinuityCounters` for the very first PAT, then
/// the same instance on subsequent calls so the CC field increments.
pub(crate) fn write_pat_packet(out: &mut [u8; 188], counters: &mut ContinuityCounters) {
    // Section body (table_id through last byte before CRC) — 12 bytes.
    let mut body = [0u8; 12];
    body[0] = 0x00; // table_id = PAT
    // section_syntax_indicator=1 | '0' | reserved '11' | section_length(12 bits)
    // section_length covers everything after itself: tsid(2) + ver/sect/last(3)
    // + prog_entry(4) + CRC(4) = 13.
    let section_length: u16 = 13;
    body[1] = 0xB0 | (((section_length >> 8) as u8) & 0x0F);
    body[2] = (section_length & 0xFF) as u8;
    // transport_stream_id = 0x0001
    body[3] = 0x00;
    body[4] = 0x01;
    // version_number(5)=0 | current_next_indicator=1 -> 0xC1 (reserved bits 11)
    body[5] = 0xC1;
    body[6] = 0x00; // section_number
    body[7] = 0x00; // last_section_number
    // Program loop: program_number(2) + reserved(3 bits) + pmt_pid(13 bits)
    body[8] = (PROGRAM_NUMBER >> 8) as u8;
    body[9] = (PROGRAM_NUMBER & 0xFF) as u8;
    body[10] = 0xE0 | ((PMT_PID >> 8) as u8 & 0x1F); // reserved bits = 0b111
    body[11] = (PMT_PID & 0xFF) as u8;

    let crc = crc32_mpeg2(&body[..12]);

    // Assemble the full 184-byte payload: pointer(1) + body(12) + CRC(4) + 0xFF padding(167).
    let mut payload = [0xFFu8; 184];
    payload[0] = 0x00; // pointer field
    payload[1..13].copy_from_slice(&body[..12]);
    payload[13] = (crc >> 24) as u8;
    payload[14] = (crc >> 16) as u8;
    payload[15] = (crc >> 8) as u8;
    payload[16] = crc as u8;
    // payload[17..184] already 0xFF from the [0xFFu8; 184] initialiser.

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

/// Build the full 188-byte PMT packet.
///
/// `pcr_pid` is the PID carrying PCR (typically the video PID).
/// `streams` is the list of ES entries — order is preserved in the PMT.
pub(crate) fn write_pmt_packet(
    out: &mut [u8; 188],
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

    payload[idx] = 0x02; // table_id
    idx += 1;
    payload[idx] = 0xB0 | (((section_length >> 8) as u8) & 0x0F);
    payload[idx + 1] = (section_length & 0xFF) as u8;
    idx += 2;
    // program_number
    payload[idx] = (PROGRAM_NUMBER >> 8) as u8;
    payload[idx + 1] = (PROGRAM_NUMBER & 0xFF) as u8;
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

    write_packet(
        out,
        PMT_PID,
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

    /// Re-decode a PAT from our generated bytes and assert PMT PID matches.
    #[test]
    fn pat_round_trips() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        write_pat_packet(&mut buf, &mut cc);

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

        // Body is 12 bytes (table_id through last byte before CRC) at buf[5..17].
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
        assert_eq!(pmt_pid, PMT_PID);

        // Padding after CRC should be 0xFF (sanity check).
        assert_eq!(buf[21], 0xFF);
        assert_eq!(buf[187], 0xFF);
    }

    #[test]
    fn pmt_round_trips_with_klva() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
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
        write_pmt_packet(&mut buf, 0x1011, &streams, &mut cc).expect("PMT fits");

        assert_eq!(buf[0], 0x47);
        // PMT PID
        let pid_value = (((buf[1] as u16) & 0x1F) << 8) | (buf[2] as u16);
        assert_eq!(pid_value, PMT_PID);
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
        write_pmt_packet(&mut buf, 0x1011, &streams, &mut cc).expect("PMT fits");
        // H.265 stream_type
        assert_eq!(buf[17], 0x24);
        // Sync KLV stream_type
        assert_eq!(buf[22], 0x15);
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
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let err = write_pmt_packet(&mut buf, 0x100, &entries, &mut cc).unwrap_err();
        assert!(matches!(err, MuxError::PmtTooLarge { .. }));
    }

    #[test]
    fn pmt_crc_validates() {
        let mut buf = [0u8; 188];
        let mut cc = ContinuityCounters::new();
        let streams = [PmtStreamEntry {
            stream_type: StreamType::H264,
            elementary_pid: 0x1011,
            descriptors: &[],
        }];
        write_pmt_packet(&mut buf, 0x1011, &streams, &mut cc).expect("PMT fits");

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
