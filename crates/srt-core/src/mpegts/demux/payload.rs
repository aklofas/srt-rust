// crates/srt-core/src/mpegts/demux/payload.rs
//! ES payload parsers: H.264 / H.265 NAL split, KLV unwrap.

use crate::mpegts::demux::event::{NalUnit, VideoCodec};

/// Split an Annex-B-framed elementary stream payload into typed NAL units.
///
/// Looks for `0x000001` or `0x00000001` start codes. NAL bytes between
/// start codes are passed through (RBSP with emulation-prevention bytes
/// preserved); the consumer's decoder removes the 0x03 escapes.
pub fn split_nals(es_payload: &[u8], codec: VideoCodec) -> Vec<NalUnit> {
    let mut out = Vec::new();
    let starts = find_start_codes(es_payload);
    for win in starts.windows(2) {
        // `data_start` is the offset of the first NAL byte after this NAL's
        // start-code prefix; `prefix_start` of the next entry is where the
        // following NAL's start-code begins. Slicing `[data_start..prefix_start]`
        // yields exactly this NAL's bytes with no inter-NAL prefix bleed.
        let data_start = win[0].data_start;
        let nal_end = win[1].prefix_start;
        if let Some(unit) = parse_one_nal(&es_payload[data_start..nal_end], codec) {
            out.push(unit);
        }
    }
    if let Some(&last) = starts.last() {
        if let Some(unit) = parse_one_nal(&es_payload[last.data_start..], codec) {
            out.push(unit);
        }
    }
    out
}

/// Offsets of one Annex-B start-code occurrence: where the prefix starts
/// (run of 00s plus 01) and where the NAL data begins (immediately after).
#[derive(Debug, Clone, Copy)]
struct StartCode {
    prefix_start: usize,
    data_start: usize,
}

/// Locate every Annex-B start code (`00 00 01` or `00 00 00 01`) in `buf`.
fn find_start_codes(buf: &[u8]) -> Vec<StartCode> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        if buf[i] == 0 && buf[i + 1] == 0 {
            if buf[i + 2] == 1 {
                out.push(StartCode {
                    prefix_start: i,
                    data_start: i + 3,
                });
                i += 3;
                continue;
            }
            if i + 4 <= buf.len() && buf[i + 2] == 0 && buf[i + 3] == 1 {
                out.push(StartCode {
                    prefix_start: i,
                    data_start: i + 4,
                });
                i += 4;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn parse_one_nal(nal: &[u8], codec: VideoCodec) -> Option<NalUnit> {
    match codec {
        VideoCodec::H264 => {
            if nal.is_empty() {
                return None;
            }
            let header = nal[0];
            // forbidden_zero_bit (1) | nal_ref_idc (2) | nal_unit_type (5)
            let ref_idc = (header >> 5) & 0x03;
            let nal_type = header & 0x1F;
            Some(NalUnit::H264 {
                nal_type,
                ref_idc,
                payload: nal[1..].to_vec(),
            })
        }
        VideoCodec::H265 => {
            if nal.len() < 2 {
                return None;
            }
            // forbidden_zero_bit (1) | nal_unit_type (6) | nuh_layer_id (6) | nuh_temporal_id_plus1 (3)
            let h0 = nal[0];
            let h1 = nal[1];
            let nal_type = (h0 >> 1) & 0x3F;
            let layer_id = ((h0 & 0x01) << 5) | (h1 >> 3);
            let temporal_id_plus1 = h1 & 0x07;
            Some(NalUnit::H265 {
                nal_type,
                layer_id,
                temporal_id_plus1,
                payload: nal[2..].to_vec(),
            })
        }
    }
}

/// KLV payload classification result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KlvShape {
    /// Payload begins with the ST 1910 AU cell UL — sync KLV.
    /// Returns the unwrapped inner KLV bytes plus the AU cell's PTS pack.
    SyncAuCell { klv: Vec<u8>, au_cell_pts: i64 },
    /// Payload looks like bare KLV (recognized SMPTE / MISB UL prefix).
    Async,
    /// Payload is something else; pass-through as `Unknown`.
    Other,
}

/// Sniff a KLV PES payload to decide sync vs. async vs. unknown.
///
/// Used by both lenient and strict modes. In lenient mode the demuxer
/// pairs the sniff result with the declared `stream_type` and emits a
/// `NonConformantIssue::StreamTypeMismatch*` if they disagree.
pub fn classify_klv(payload: &[u8]) -> KlvShape {
    use crate::klv::st1910::{AU_CELL_UL, unwrap_au_cell};
    if payload.len() >= 16 && payload[..16] == AU_CELL_UL.0 {
        if let Ok((klv, ts)) = unwrap_au_cell(payload) {
            // Convert the AU cell's microseconds-since-1970 to a 90 kHz PTS the
            // demuxer can carry on the event. The conversion is `µs * 9 / 100`.
            let micros = ts.timestamp_us;
            let pts_90khz = (micros as i128 * 9 / 100) as i64;
            return KlvShape::SyncAuCell {
                klv: klv.to_vec(),
                au_cell_pts: pts_90khz,
            };
        }
    }
    // Bare KLV LS: starts with a 16-byte SMPTE UL. The first 4 bytes are the
    // canonical UL header `06 0E 2B 34`; treat any payload with those as
    // bare KLV.
    if payload.len() >= 16 && payload[0..4] == [0x06, 0x0E, 0x2B, 0x34] {
        return KlvShape::Async;
    }
    KlvShape::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_two_nals() {
        // 0x09 access_unit_delimiter then 0x05 IDR.
        let buf = vec![
            0x00, 0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB,
        ];
        let nals = split_nals(&buf, VideoCodec::H264);
        assert_eq!(nals.len(), 2);
        match &nals[0] {
            NalUnit::H264 {
                nal_type, payload, ..
            } => {
                assert_eq!(*nal_type, 9);
                assert_eq!(payload, &vec![0x10]);
            }
            _ => panic!("wrong codec"),
        }
        match &nals[1] {
            NalUnit::H264 { nal_type, .. } => assert_eq!(*nal_type, 5),
            _ => panic!("wrong codec"),
        }
    }

    #[test]
    fn h265_two_nals() {
        // VPS (32) + IDR_W_RADL (19) with layer_id=0, temporal_id_plus1=1.
        let buf = vec![
            0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0xAA, 0x00, 0x00, 0x01, 0x26, 0x01, 0xBB,
        ];
        let nals = split_nals(&buf, VideoCodec::H265);
        assert_eq!(nals.len(), 2);
        match &nals[0] {
            NalUnit::H265 {
                nal_type,
                layer_id,
                temporal_id_plus1,
                ..
            } => {
                assert_eq!(*nal_type, 32);
                assert_eq!(*layer_id, 0);
                assert_eq!(*temporal_id_plus1, 1);
            }
            _ => panic!("wrong codec"),
        }
        match &nals[1] {
            NalUnit::H265 { nal_type, .. } => assert_eq!(*nal_type, 19),
            _ => panic!("wrong codec"),
        }
    }

    #[test]
    fn classifies_async_klv_by_ul_prefix() {
        let mut buf = vec![0x06, 0x0E, 0x2B, 0x34];
        buf.extend(std::iter::repeat_n(0xAA, 30));
        assert_eq!(classify_klv(&buf), KlvShape::Async);
    }

    #[test]
    fn classifies_unknown_payload() {
        assert_eq!(classify_klv(&[0xDE, 0xAD, 0xBE, 0xEF]), KlvShape::Other);
    }
}
