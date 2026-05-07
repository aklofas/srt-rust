// crates/srt-core/src/mpegts/demux/payload.rs
//! ES payload parsers: H.264 / H.265 NAL split, KLV unwrap.

use crate::codec::av1::leb128::read_leb128;
use crate::mpegts::demux::event::{NalUnit, NonConformantIssue, Obu, ObuExtension, VideoCodec};

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
        VideoCodec::H266 => {
            if nal.len() < 2 {
                return None;
            }
            // Per H.266 V4 §7.3.1.2:
            //   byte 0: forbidden_zero_bit(1) | nuh_reserved_zero_bit(1) | nuh_layer_id(6)
            //   byte 1: nal_unit_type(5)      | nuh_temporal_id_plus1(3)
            //
            // Note nal_type lives in byte 1 (top 5 bits); H.265 has it in
            // byte 0 — different layout, not just renamed fields.
            let h0 = nal[0];
            let h1 = nal[1];
            let layer_id = h0 & 0x3F;
            let nal_type = (h1 >> 3) & 0x1F;
            let temporal_id_plus1 = h1 & 0x07;
            Some(NalUnit::H266 {
                nal_type,
                layer_id,
                temporal_id_plus1,
                payload: nal[2..].to_vec(),
            })
        }
        VideoCodec::Av1 => {
            // AV1 is OBU-shaped, not NAL-shaped — it should never reach
            // this NAL splitter. Phase 4 (Tasks 15-21) routes AV1 to a
            // separate OBU parser before this function is called.
            unimplemented!("AV1 uses OBU framing, not NAL — routed separately in Phase 4")
        }
    }
}

/// Split an AV1 PES payload into typed OBUs. Per AV1 spec §5.3
/// "low overhead bitstream format" framing.
///
/// Returns `(obus, issues)`. `issues` contains any non-conformance
/// issues raised during the walk (missing obu_size field, forbidden
/// Tile List OBU); the caller forwards these to the demuxer's
/// non-conformance pipeline. `pid` on each issue is left as a
/// sentinel `0` — caller patches it with the real value.
///
/// On a malformed buffer (truncated header, truncated LEB128, length
/// runs past buffer end) the splitter stops and returns what it has
/// accumulated, mirroring `split_nals`'s lenient stance.
pub fn split_obus(es_payload: &[u8]) -> (Vec<Obu>, Vec<NonConformantIssue>) {
    let mut out = Vec::new();
    let mut issues = Vec::new();
    let mut i = 0usize;
    while i < es_payload.len() {
        // OBU header byte (AV1 §5.3.2):
        //   obu_forbidden_bit f(1)  — must be 0
        //   obu_type           f(4)
        //   obu_extension_flag f(1)
        //   obu_has_size_field f(1)
        //   obu_reserved_1bit  f(1)
        let header = es_payload[i];
        let obu_type = (header >> 3) & 0x0F;
        let extension_flag = (header >> 2) & 0x01 != 0;
        let has_size_field = (header >> 1) & 0x01 != 0;
        i += 1;

        let extension = if extension_flag {
            if i >= es_payload.len() {
                break; // truncated extension — stop
            }
            let ext = es_payload[i];
            i += 1;
            // temporal_id(3) | spatial_id(2) | reserved(3)
            Some(ObuExtension {
                temporal_id: (ext >> 5) & 0x07,
                spatial_id: (ext >> 3) & 0x03,
            })
        } else {
            None
        };

        if !has_size_field {
            issues.push(NonConformantIssue::Av1ObuMissingSizeField {
                pid: 0, // caller patches with real pid; see Task 19
                obu_type,
            });
            let payload = es_payload[i..].to_vec();
            out.push(Obu {
                obu_type,
                extension,
                payload,
            });
            break;
        }

        let (obu_size, consumed) = match read_leb128(es_payload, i) {
            Ok(t) => t,
            Err(_) => break, // truncated LEB128 — stop
        };
        i += consumed;

        let body_end = i + obu_size as usize;
        if body_end > es_payload.len() {
            // Length runs past buffer end. Stop walking.
            break;
        }
        let payload = es_payload[i..body_end].to_vec();
        i = body_end;

        // Tile List OBU non-conformance issue per binding §3.3.
        if obu_type == 8 {
            issues.push(NonConformantIssue::Av1TileListNotAllowed { pid: 0 });
        }

        out.push(Obu {
            obu_type,
            extension,
            payload,
        });
    }
    (out, issues)
}

/// KLV payload classification result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KlvShape {
    /// H.222.0 V9 §2.12.4.2 Metadata_AU_cell — sync KLV. Returns the
    /// unwrapped inner KLV bytes plus the parsed AU cell header. PES PTS
    /// is carried separately by the demuxer event (per H.222.0 §2.12.4.1
    /// the AU cell carries no embedded timestamp).
    SyncAuCell {
        klv: Vec<u8>,
        header: crate::mpegts::au_cell::AuCellHeader,
    },
    /// Async-shape KLV (bare SMPTE UL at offset 0). `klv` is the unwrapped
    /// LS bytes ready to feed to `klv::st0601::decode`.
    Async { klv: Vec<u8> },
    /// AU cell header parsed cleanly but `cell_fragment_indication != Complete`
    /// (i.e. First / Middle / Last). Multi-cell reassembly is not implemented;
    /// the demuxer drops the partial payload and surfaces a
    /// `NonConformantIssue::MultiCellAu` event for observability.
    ///
    /// `dropped_bytes` is the declared inner AU cell payload length.
    PartialAuCell { dropped_bytes: usize },
    /// Payload is something else; pass-through as `Unknown`.
    Other,
}

/// Sniff a KLV PES payload to decide sync vs. async vs. unknown.
///
/// Used by both lenient and strict modes. In lenient mode the demuxer
/// pairs the sniff result with the declared `stream_type` and emits a
/// `NonConformantIssue::StreamTypeMismatch*` if they disagree.
pub fn classify_klv(payload: &[u8]) -> KlvShape {
    use crate::mpegts::au_cell::{CellFragmentIndication, read_metadata_au_cell};

    // First try: H.222.0 V9 §2.12.4.2 Metadata_AU_cell (sync metadata,
    // PMT stream_type 0x15, mandated by STANAG 4609 / MISB ST 1402.2
    // §9.4.1 + Appendix B Table 2). Recognized by a valid 5-byte header
    // whose declared AU_cell_data_length doesn't overrun the payload.
    if payload.len() >= 5 + 16 {
        if let Ok((header, inner)) = read_metadata_au_cell(payload) {
            match header.cell_fragment_indication {
                CellFragmentIndication::Complete => {
                    // Inner KLV-LS sniff gates the SyncAuCell path.
                    // (Task 3.5 will drop this gate — see plan #30.)
                    if inner.len() >= 16 && inner[0..4] == [0x06, 0x0E, 0x2B, 0x34] {
                        return KlvShape::SyncAuCell {
                            klv: inner.to_vec(),
                            header,
                        };
                    }
                    // Fall through to async detection if AU cell parsed
                    // cleanly but inner isn't KLV-LS shaped.
                }
                _ => {
                    // Partial cell (First / Middle / Last): surface the
                    // dropped byte count. Reassembly is deferred.
                    return KlvShape::PartialAuCell {
                        dropped_bytes: inner.len(),
                    };
                }
            }
        }
    }

    // Second try: bare KLV LS (SMPTE UL at offset 0). Async metadata.
    if payload.len() >= 16 && payload[0..4] == [0x06, 0x0E, 0x2B, 0x34] {
        return KlvShape::Async {
            klv: payload.to_vec(),
        };
    }

    KlvShape::Other
}

/// Strip the EN 300 743 §6.2 PES_data_field envelope from a DVB-subtitle PES
/// payload. The wire format is:
///
/// ```text
///   data_identifier(1) + subtitle_stream_id(1) + segments(N) + end_marker(0xFF)
/// ```
///
/// Returns the segments slice if the envelope is well-formed, `None`
/// otherwise — caller falls through to passthrough on malformed input so
/// strict-mode consumers can still observe the unexpected payload shape.
///
/// `data_identifier` valid range per ETSI EN 300 743 §7.1: `0x20..=0x3F`
/// (DVB subtitle) or `0x70..=0x7F` (DVB subtitle for HD). The
/// `subtitle_stream_id` is fixed at `0x00` per §6.2.
pub(crate) fn strip_dvb_sub_envelope(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 3 {
        return None;
    }
    let valid_id = matches!(bytes[0], 0x20..=0x3F | 0x70..=0x7F);
    if !valid_id || bytes[1] != 0x00 || *bytes.last().unwrap() != 0xFF {
        return None;
    }
    Some(&bytes[2..bytes.len() - 1])
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
        assert_eq!(classify_klv(&buf), KlvShape::Async { klv: buf });
    }

    #[test]
    fn classifies_sync_klv_via_h222_au_cell_header() {
        // 5-byte H.222.0 §2.12.4.2 Metadata_AU_cell header
        // [metadata_service_id][sequence_number][flags][AU_cell_data_length BE]
        // followed by 16-byte SMPTE UL + 10 filler bytes.
        // AU_cell_data_length = 26 bytes (16 UL + 10 body). This is the
        // spec-conformant sync KLV wire form per ST 1402.2 §9.4.1.
        let inner: Vec<u8> = [0x06, 0x0E, 0x2B, 0x34]
            .into_iter()
            .chain(std::iter::repeat_n(0xAA, 12))
            .chain(std::iter::repeat_n(0x55, 10))
            .collect();
        // flags byte 0xCF: cfi=11 (Complete), dcf=0, rai=0, reserved=1111.
        // Complete = 0b11 in bits [7:6]; 0b11_0_0_1111 = 0xCF.
        // (Muxer auto-wrap defaults cfi=11 rai=1 reserved=1111 = 0xDF;
        // this test uses rai=0 to keep the fixture value distinct.)
        let mut buf = vec![0x00, 0xB7, 0xCF, 0x00, 0x1A];
        buf.extend_from_slice(&inner);
        match classify_klv(&buf) {
            KlvShape::SyncAuCell { klv, header } => {
                assert_eq!(klv, inner);
                assert_eq!(header.metadata_service_id, 0x00);
                assert_eq!(header.sequence_number, 0xB7);
            }
            other => panic!("expected SyncAuCell, got {other:?}"),
        }
    }

    #[test]
    fn classifies_unknown_payload() {
        assert_eq!(classify_klv(&[0xDE, 0xAD, 0xBE, 0xEF]), KlvShape::Other);
    }

    #[test]
    fn h264_single_nal_3byte_start() {
        // Single NAL preceded by the 3-byte start code — exercises the
        // trailing-NAL branch of split_nals where the inner-NAL loop
        // doesn't fire. Locks in the fix for the find_start_codes
        // slice-boundary bug (NAL bytes must NOT include any next-NAL
        // prefix bytes — there's no next NAL here).
        let buf = vec![0x00, 0x00, 0x01, 0x67, 0xAA, 0xBB];
        let nals = split_nals(&buf, VideoCodec::H264);
        assert_eq!(nals.len(), 1);
        match &nals[0] {
            NalUnit::H264 {
                nal_type, payload, ..
            } => {
                assert_eq!(*nal_type, 7); // SPS
                assert_eq!(payload, &vec![0xAA, 0xBB]);
            }
            _ => panic!("wrong codec"),
        }
    }

    #[test]
    fn split_nals_empty_input() {
        // Empty input produces no NALs. find_start_codes returns vec![],
        // both the inner-window loop and the trailing-NAL branch no-op.
        assert_eq!(split_nals(&[], VideoCodec::H264), vec![]);
        assert_eq!(split_nals(&[], VideoCodec::H265), vec![]);
    }

    #[test]
    fn h266_two_nals() {
        // VPS_NUT (14) + IDR_W_RADL (7) with layer_id=0, temporal_id_plus1=1.
        // H.266 NAL header (per H.266 V4 §7.3.1.2):
        //   byte 0: forbidden(1) | reserved(1) | nuh_layer_id(6)
        //   byte 1: nal_unit_type(5) | nuh_temporal_id_plus1(3)
        //
        // VPS: layer_id=0, nal_type=14(0b01110), temporal_id_plus1=1 →
        //   byte0 = 0x00 (forbidden=0, reserved=0, layer_id=0)
        //   byte1 = (14 << 3) | 1 = 0x71
        //
        // IDR_W_RADL: layer_id=0, nal_type=7(0b00111), temporal_id_plus1=1 →
        //   byte0 = 0x00
        //   byte1 = (7 << 3) | 1 = 0x39
        let buf = vec![
            0x00, 0x00, 0x00, 0x01, 0x00, 0x71, 0xAA, 0x00, 0x00, 0x01, 0x00, 0x39, 0xBB,
        ];
        let nals = split_nals(&buf, VideoCodec::H266);
        assert_eq!(nals.len(), 2);
        match &nals[0] {
            NalUnit::H266 {
                nal_type,
                layer_id,
                temporal_id_plus1,
                payload,
            } => {
                assert_eq!(*nal_type, 14);
                assert_eq!(*layer_id, 0);
                assert_eq!(*temporal_id_plus1, 1);
                assert_eq!(payload, &vec![0xAA]);
            }
            _ => panic!("wrong codec variant"),
        }
        match &nals[1] {
            NalUnit::H266 { nal_type, .. } => assert_eq!(*nal_type, 7),
            _ => panic!("wrong codec variant"),
        }
    }

    #[test]
    fn split_nals_no_start_codes() {
        // Bytes with no start code at all produce no NALs. Garbage
        // before/between expected boundaries is silently discarded —
        // sync recovery is the demuxer state machine's job (Task 7),
        // not the leaf NAL splitter's.
        let buf = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE];
        assert_eq!(split_nals(&buf, VideoCodec::H264), vec![]);
    }

    fn build_obu_with_size(obu_type: u8, payload: &[u8]) -> Vec<u8> {
        // Header: obu_type(4) | ext_flag(1)=0 | has_size(1)=1 | reserved(1)=0
        // → byte = (obu_type << 3) | 0b010 = (obu_type << 3) | 0x02
        let header = (obu_type << 3) | 0x02;
        let mut v = vec![header];
        // LEB128 size — for sizes < 128, single byte equal to size.
        let size = payload.len();
        assert!(size < 128, "test helper supports only small payloads");
        v.push(size as u8);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn split_obus_two_obus() {
        let mut buf = Vec::new();
        buf.extend(build_obu_with_size(2, &[])); // Temporal Delimiter (empty)
        buf.extend(build_obu_with_size(1, &[0xAA, 0xBB])); // Sequence Header (placeholder)
        let (obus, issues) = split_obus(&buf);
        assert_eq!(obus.len(), 2);
        assert_eq!(obus[0].obu_type, 2);
        assert_eq!(obus[1].obu_type, 1);
        assert_eq!(obus[1].payload, vec![0xAA, 0xBB]);
        assert!(issues.is_empty());
    }

    #[test]
    fn split_obus_missing_size_field_reports_issue() {
        // obu_type=1 (Seq Header), ext_flag=0, has_size=0
        let header = 1 << 3;
        let buf = vec![header, 0xAA, 0xBB, 0xCC];
        let (obus, issues) = split_obus(&buf);
        assert_eq!(obus.len(), 1);
        assert_eq!(obus[0].payload, vec![0xAA, 0xBB, 0xCC]);
        assert!(matches!(
            issues.first(),
            Some(NonConformantIssue::Av1ObuMissingSizeField { .. })
        ));
    }

    #[test]
    fn split_obus_tile_list_reports_issue() {
        let buf = build_obu_with_size(8, &[0x00]); // Tile List (forbidden in TS)
        let (obus, issues) = split_obus(&buf);
        assert_eq!(obus.len(), 1);
        assert!(matches!(
            issues.first(),
            Some(NonConformantIssue::Av1TileListNotAllowed { .. })
        ));
    }

    #[test]
    fn split_obus_truncated_leb128_stops_walking() {
        // Header byte then a continuation byte with no terminator, and one
        // more (header for a hypothetical second OBU we should never reach).
        let buf = vec![0x12, 0x80]; // single OBU header + truncated LEB128
        let (obus, _) = split_obus(&buf);
        assert!(obus.is_empty(), "truncated LEB128 should abort the walk");
    }

    #[test]
    fn split_obus_empty_input_returns_empty() {
        let (obus, issues) = split_obus(&[]);
        assert!(obus.is_empty());
        assert!(issues.is_empty());
    }

    #[test]
    fn classify_klv_recognizes_h222_metadata_au_cell_with_header_fields() {
        // Build an AU cell with non-default header field values, carrying a
        // synthetic ST 0601 LS payload. classify_klv must surface ALL 5
        // header fields verbatim through KlvShape (per H.222.0 §2.12.4.2
        // Table 2-156).
        use crate::mpegts::au_cell::{
            AuCellHeader, CellFragmentIndication, write_metadata_au_cell,
        };
        let mut inner_klv = Vec::new();
        inner_klv.extend_from_slice(&[
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00,
        ]);
        inner_klv.push(0x04);
        inner_klv.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let hdr_in = AuCellHeader {
            metadata_service_id: 0x42, // non-default to detect drift
            sequence_number: 0x07,
            cell_fragment_indication: CellFragmentIndication::Complete,
            decoder_config_flag: true,
            random_access_indicator: false,
        };
        let mut wrapped = Vec::new();
        write_metadata_au_cell(&mut wrapped, hdr_in, &inner_klv).unwrap();

        match classify_klv(&wrapped) {
            KlvShape::SyncAuCell { klv, header } => {
                assert_eq!(klv, inner_klv);
                assert_eq!(header.metadata_service_id, 0x42);
                assert_eq!(header.sequence_number, 0x07);
                assert_eq!(
                    header.cell_fragment_indication,
                    CellFragmentIndication::Complete
                );
                assert!(header.decoder_config_flag);
                assert!(!header.random_access_indicator);
            }
            other => panic!("expected SyncAuCell, got {other:?}"),
        }
    }

    #[test]
    fn classify_klv_async_arm_unchanged_for_bare_ls() {
        let bare_klv = [
            0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00,
            0x00, 0x00, 0x04, 0xDE, 0xAD, 0xBE, 0xEF,
        ];
        match classify_klv(&bare_klv) {
            KlvShape::Async { klv } => assert_eq!(klv, &bare_klv[..]),
            other => panic!("expected Async, got {other:?}"),
        }
    }

    #[test]
    fn strip_dvb_sub_envelope_round_trip() {
        let segs = [0x0F, 0x10, 0xAB, 0xCD];
        let wrapped = [&[0x20u8, 0x00][..], &segs[..], &[0xFF][..]].concat();
        assert_eq!(strip_dvb_sub_envelope(&wrapped), Some(&segs[..]));
    }

    #[test]
    fn strip_dvb_sub_envelope_accepts_hd_data_identifier_range() {
        // 0x70..=0x7F is HD subtitle per EN 300 743 §7.1.
        let wrapped = [0x70, 0x00, 0x0F, 0x10, 0xFF];
        assert_eq!(strip_dvb_sub_envelope(&wrapped), Some(&[0x0F, 0x10][..]));
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_missing_marker() {
        assert!(strip_dvb_sub_envelope(&[0x20, 0x00, 0xAB, 0xCD]).is_none());
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_bad_data_identifier() {
        // 0x40 is outside both 0x20..=0x3F and 0x70..=0x7F.
        assert!(strip_dvb_sub_envelope(&[0x40, 0x00, 0xAB, 0xFF]).is_none());
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_bad_stream_id() {
        // subtitle_stream_id must be 0x00 per §6.2.
        assert!(strip_dvb_sub_envelope(&[0x20, 0x01, 0xAB, 0xFF]).is_none());
    }

    #[test]
    fn strip_dvb_sub_envelope_too_short() {
        assert!(strip_dvb_sub_envelope(&[0x20]).is_none());
        assert!(strip_dvb_sub_envelope(&[]).is_none());
    }
}
