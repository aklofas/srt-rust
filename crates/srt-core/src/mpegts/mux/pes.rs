//! PES (Packetized Elementary Stream) header packing.
//!
//! Four call shapes on the muxer hot path:
//! - Video: stream_id 0xE0, PTS only, PES_packet_length = 0 (unbounded).
//! - KLV asynchronous: stream_id 0xFC, no PTS, bounded length.
//! - KLV synchronous: stream_id 0xFC, PTS only, bounded length.
//! - Audio: stream_id 0xC0..=0xCF (base + within-program index), PTS only, bounded length.
//!
//! PES header layout (per ISO/IEC 13818-1 §2.4.3.6):
//!   start_code(3): 0x00 0x00 0x01
//!   stream_id(1)
//!   PES_packet_length(2): 0 = unbounded (video only)
//!   flags1(1): '10' marker | scrambling=0 | priority=0 | data_alignment=0 | copyright=0 | original=0
//!   flags2(1): PTS_DTS_flags(2 high) | other flags (all 0 for our case)
//!   PES_header_data_length(1)
//!   [PTS(5)] [DTS(5)]

use crate::mpegts::common::Pts90khz;

pub(crate) const STREAM_ID_VIDEO: u8 = 0xE0;
pub(crate) const STREAM_ID_KLV: u8 = 0xFC;
/// Base PES `stream_id` for audio elementary streams.
///
/// Audio streams within a program use `STREAM_ID_AUDIO_BASE + within_program_index`,
/// consuming the `0xC0..=0xCF` range of ISO/IEC 13818-1's audio stream_id space
/// (supports up to 16 audio streams per program).
pub(crate) const STREAM_ID_AUDIO_BASE: u8 = 0xC0;

/// PES PTS/DTS field selector. Embeds the PTS so callers can't construct an
/// inconsistent state (e.g. PtsOnly with no value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PesPtsField {
    /// No PTS, no DTS — async metadata.
    None,
    /// PTS present, no DTS — video without B-frame reorder, sync KLV.
    PtsOnly(Pts90khz),
}

/// Maximum size of a PES header for the cases this muxer emits.
/// = 3(start) + 1(stream_id) + 2(length) + 1(flags1) + 1(flags2) + 1(header_data_length) + 5(PTS) = 14
pub(crate) const MAX_PES_HEADER_SIZE: usize = 14;

/// Write a complete audio PES packet (header + caller's frame bytes) into `out`.
///
/// `within_program_index` selects the PES `stream_id`:
/// `STREAM_ID_AUDIO_BASE + within_program_index` (range `0xC0..=0xCF`).
///
/// PES_packet_length is bounded (audio frames are bounded, unlike video's 0 =
/// unbounded sentinel). Callers must ensure `frames.len()` fits in u16 after
/// accounting for the 3-byte PES header overhead — this is enforced by the
/// per-stream payload cap checked in `push_audio_to`.
pub(crate) fn write_audio_pes(
    out: &mut Vec<u8>,
    within_program_index: u8,
    pts: PesPtsField,
    frames: &[u8],
) {
    debug_assert!(within_program_index < 16, "audio cap is 16 per program");
    let stream_id = STREAM_ID_AUDIO_BASE + within_program_index;
    let mut header = [0u8; MAX_PES_HEADER_SIZE];
    let header_len = write_pes_header(&mut header, stream_id, pts, Some(frames.len() as u16));
    out.extend_from_slice(&header[..header_len]);
    out.extend_from_slice(frames);
}

/// Write a PES header to `out`. Returns bytes written.
///
/// `payload_length`:
/// - `Some(n)` — bounded; emits PES_packet_length covering flags1, flags2,
///   header_data_length, PES header data, and ES payload (i.e., everything
///   after PES_packet_length itself).
/// - `None` — unbounded (PES_packet_length = 0). Used for video PES.
pub(crate) fn write_pes_header(
    out: &mut [u8],
    stream_id: u8,
    pts_field: PesPtsField,
    payload_length: Option<u16>,
) -> usize {
    debug_assert!(out.len() >= MAX_PES_HEADER_SIZE);

    // flags2 high two bits = PTS_DTS_flags. We only support None (0b00) and PtsOnly (0b10).
    let (pts_dts_flags, pts_size) = match pts_field {
        PesPtsField::None => (0b00u8, 0u8),
        PesPtsField::PtsOnly(_) => (0b10u8, 5u8),
    };

    // PES_header_data_length = bytes after this field that are PES-header (PTS/DTS/etc.)
    // Equal to pts_size in our cases.
    let header_data_length = pts_size;

    // PES_packet_length: covers flags1, flags2, header_data_length, PES header data,
    // and ES payload — i.e., everything after PES_packet_length itself.
    let pes_packet_length = match payload_length {
        None => 0u16, // unbounded (video)
        Some(n) => 3u16 + header_data_length as u16 + n,
    };

    // start_code prefix
    out[0] = 0x00;
    out[1] = 0x00;
    out[2] = 0x01;
    // stream_id
    out[3] = stream_id;
    // PES_packet_length
    out[4] = (pes_packet_length >> 8) as u8;
    out[5] = (pes_packet_length & 0xFF) as u8;
    // flags1: 0b10 marker (10000000) | rest 0
    out[6] = 0x80;
    // flags2: PTS_DTS_flags << 6 | rest 0
    out[7] = pts_dts_flags << 6;
    // PES_header_data_length
    out[8] = header_data_length;

    let mut idx = 9;
    if let PesPtsField::PtsOnly(pts) = pts_field {
        write_pts(&mut out[idx..idx + 5], pts, /*marker_high=*/ 0b0010);
        idx += 5;
    }
    idx
}

/// Encode a 33-bit PTS into 5 bytes per ISO/IEC 13818-1 §2.4.3.6.
///
/// Layout (bits, MSB first):
///   marker_high(4) | PTS[32..30](3) | marker(1)
///   PTS[29..22](8)
///   PTS[21..15](7) | marker(1)
///   PTS[14..7](8)
///   PTS[6..0](7) | marker(1)
///
/// `marker_high` is 0b0010 for PTS-only ("0010"), 0b0011 for PTS+DTS PTS,
/// 0b0001 for the DTS half. We only emit PTS-only here.
fn write_pts(out: &mut [u8], pts: Pts90khz, marker_high: u8) {
    debug_assert!(out.len() >= 5);
    let pts_val: u64 = pts.masked_33bit();

    out[0] = (marker_high << 4) | ((((pts_val >> 30) & 0x07) << 1) as u8) | 0x01;
    out[1] = ((pts_val >> 22) & 0xFF) as u8;
    out[2] = ((((pts_val >> 15) & 0x7F) << 1) as u8) | 0x01;
    out[3] = ((pts_val >> 7) & 0xFF) as u8;
    out[4] = (((pts_val & 0x7F) << 1) as u8) | 0x01;
}

/// Decode the 33-bit PTS from a 5-byte field. Used by tests for round-trip
/// verification.
#[cfg(test)]
fn read_pts(buf: &[u8]) -> u64 {
    debug_assert!(buf.len() >= 5);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_pes_header_unbounded() {
        let mut buf = [0u8; MAX_PES_HEADER_SIZE];
        let n = write_pes_header(
            &mut buf,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(Pts90khz(90_000)),
            None,
        );
        assert_eq!(n, 14);
        assert_eq!(&buf[..3], &[0x00, 0x00, 0x01]);
        assert_eq!(buf[3], STREAM_ID_VIDEO);
        // PES_packet_length = 0 (unbounded)
        assert_eq!(buf[4], 0);
        assert_eq!(buf[5], 0);
        // flags1 marker
        assert_eq!(buf[6], 0x80);
        // PTS_DTS_flags = 10 (PTS only)
        assert_eq!(buf[7], 0x80);
        assert_eq!(buf[8], 5);
        // PTS round-trips
        assert_eq!(read_pts(&buf[9..14]), 90_000);
    }

    #[test]
    fn klv_async_pes_header_no_pts() {
        let mut buf = [0u8; MAX_PES_HEADER_SIZE];
        let n = write_pes_header(&mut buf, STREAM_ID_KLV, PesPtsField::None, Some(20));
        assert_eq!(n, 9);
        assert_eq!(buf[3], STREAM_ID_KLV);
        // PES_packet_length = 3 + 0 + 20 = 23
        assert_eq!(buf[4], 0);
        assert_eq!(buf[5], 23);
        // PTS_DTS_flags = 00
        assert_eq!(buf[7], 0x00);
        // PES_header_data_length = 0
        assert_eq!(buf[8], 0);
    }

    #[test]
    fn klv_sync_pes_header_with_pts() {
        let mut buf = [0u8; MAX_PES_HEADER_SIZE];
        let n = write_pes_header(
            &mut buf,
            STREAM_ID_KLV,
            PesPtsField::PtsOnly(Pts90khz(45_000)),
            Some(100),
        );
        assert_eq!(n, 14);
        // PES_packet_length = 3 + 5 + 100 = 108
        assert_eq!(buf[4], 0);
        assert_eq!(buf[5], 108);
        assert_eq!(buf[7], 0x80); // PTS only
        assert_eq!(buf[8], 5); // PES_header_data_length = 5
        assert_eq!(read_pts(&buf[9..14]), 45_000);
    }

    #[test]
    fn pts_marker_bits_set() {
        let mut buf = [0u8; 5];
        write_pts(&mut buf, Pts90khz(0), 0b0010);
        // Marker bits (low bit of bytes 0/2/4) must be 1.
        assert_eq!(buf[0] & 0x01, 0x01);
        assert_eq!(buf[2] & 0x01, 0x01);
        assert_eq!(buf[4] & 0x01, 0x01);
        // Marker high nibble = 0010 (PTS-only)
        assert_eq!(buf[0] >> 4, 0b0010);
    }

    #[test]
    fn pts_round_trip_max_value() {
        let mut buf = [0u8; 5];
        let max_33bit = (1u64 << 33) - 1;
        write_pts(&mut buf, Pts90khz(max_33bit as i64), 0b0010);
        assert_eq!(read_pts(&buf), max_33bit);
    }

    #[test]
    fn pts_round_trip_sweep() {
        let values: [u64; 6] = [0, 1, 90_000, 1 << 16, 1 << 30, (1u64 << 33) - 1];
        let mut buf = [0u8; 5];
        for v in values {
            write_pts(&mut buf, Pts90khz(v as i64), 0b0010);
            assert_eq!(read_pts(&buf), v, "value {}", v);
        }
    }
}
