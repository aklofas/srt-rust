//! ES payload parsers: H.264 / H.265 NAL split, KLV unwrap.

use crate::codec::av1::leb128::read_leb128;
use crate::mpegts::demux::event::{
    Av1ObuHeaderKind, NalHeaderKind, NalUnit, NonConformantIssue, Obu, ObuExtension, VideoCodec,
};

/// Split an Annex-B-framed elementary stream payload into typed NAL units.
///
/// Looks for `0x000001` or `0x00000001` start codes. NAL bytes between
/// start codes are passed through (RBSP with emulation-prevention bytes
/// preserved); the consumer's decoder removes the 0x03 escapes.
///
/// Returns `(nals, issues)`. `issues` carries any NAL-header constraint
/// violations detected per H.264 §7.3.1 / H.265 §7.3.1.2 / H.266 §7.3.1.2
/// (forbidden_zero_bit set, reserved bits non-zero, etc.). Issues use
/// sentinel `codec` carried verbatim; the caller annotates with PID at
/// queue-time. NALs for which the spec mandates discard (H.266 reserved /
/// layer>55) are dropped from the output but the issue is still emitted.
pub fn split_nals(es_payload: &[u8], codec: VideoCodec) -> (Vec<NalUnit>, Vec<NonConformantIssue>) {
    let mut out = Vec::new();
    let mut issues = Vec::new();
    let starts = find_start_codes(es_payload);
    for win in starts.windows(2) {
        // `data_start` is the offset of the first NAL byte after this NAL's
        // start-code prefix; `prefix_start` of the next entry is where the
        // following NAL's start-code begins. Slicing `[data_start..prefix_start]`
        // yields exactly this NAL's bytes with no inter-NAL prefix bleed.
        let data_start = win[0].data_start;
        let nal_end = win[1].prefix_start;
        if let Some(unit) = parse_one_nal(&es_payload[data_start..nal_end], codec, &mut issues) {
            out.push(unit);
        }
    }
    if let Some(&last) = starts.last() {
        if let Some(unit) = parse_one_nal(&es_payload[last.data_start..], codec, &mut issues) {
            out.push(unit);
        }
    }
    (out, issues)
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

fn parse_one_nal(
    nal: &[u8],
    codec: VideoCodec,
    issues: &mut Vec<NonConformantIssue>,
) -> Option<NalUnit> {
    match codec {
        VideoCodec::H264 => {
            if nal.is_empty() {
                return None;
            }
            let header = nal[0];
            // forbidden_zero_bit (1) | nal_ref_idc (2) | nal_unit_type (5)
            // Per H.264 §7.3.1: forbidden_zero_bit MUST be 0. H.264 has no
            // reserved bit and no temporal-id field at the NAL header level.
            if (header & 0x80) != 0 {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ForbiddenZeroBit,
                });
            }
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
            // Per H.265 §7.3.1.2: forbidden_zero_bit MUST be 0;
            // nuh_temporal_id_plus1 MUST be != 0 (plus-1 encoding reserves
            // 0 as forbidden so decoders can distinguish missing/sync).
            let h0 = nal[0];
            let h1 = nal[1];
            if (h0 & 0x80) != 0 {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ForbiddenZeroBit,
                });
            }
            let nal_type = (h0 >> 1) & 0x3F;
            let layer_id = ((h0 & 0x01) << 5) | (h1 >> 3);
            let temporal_id_plus1 = h1 & 0x07;
            if temporal_id_plus1 == 0 {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ZeroTemporalIdPlus1,
                });
            }
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
            //
            // Spec constraints:
            // - forbidden_zero_bit MUST be 0.
            // - nuh_reserved_zero_bit MUST be 0; receivers MUST discard
            //   NALs that violate this (H.266 §7.3.1.2). Drop the NAL.
            // - nuh_layer_id MUST be in 0..=55 (§7.4.2.2); values 56..=63
            //   are reserved and receivers MUST discard such NALs.
            // - nuh_temporal_id_plus1 MUST be != 0.
            let h0 = nal[0];
            let h1 = nal[1];
            let forbidden = (h0 & 0x80) != 0;
            let reserved = (h0 & 0x40) != 0;
            let layer_id = h0 & 0x3F;
            let nal_type = (h1 >> 3) & 0x1F;
            let temporal_id_plus1 = h1 & 0x07;
            if forbidden {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ForbiddenZeroBit,
                });
            }
            if reserved {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ReservedBit,
                });
                // H.266 §7.3.1.2 requires receivers to discard NALs with
                // nuh_reserved_zero_bit set. Emit issue + drop.
                return None;
            }
            if layer_id > 55 {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::LayerIdOutOfRange { id: layer_id },
                });
                // H.266 §7.4.2.2: layer_id in 56..=63 is reserved; receivers
                // MUST discard. Emit issue + drop.
                return None;
            }
            if temporal_id_plus1 == 0 {
                issues.push(NonConformantIssue::NalHeader {
                    codec,
                    kind: NalHeaderKind::ZeroTemporalIdPlus1,
                });
            }
            Some(NalUnit::H266 {
                nal_type,
                layer_id,
                temporal_id_plus1,
                payload: nal[2..].to_vec(),
            })
        }
        VideoCodec::Av1 => {
            // AV1 is OBU-shaped, not NAL-shaped. The demuxer dispatches AV1
            // to `split_obus` before this function is ever called; reaching
            // this arm means demuxer state is inconsistent. Defense-in-depth:
            // don't panic — return None and emit a debug_assert so test runs
            // catch any regression that routes AV1 here.
            //
            // Typed-error promotion (`parse_one_nal -> Result<Option<_>, _>`)
            // is deferred to Phase 1's SemVer ratchet — would cascade through
            // `split_nals`, the demuxer call site, and existing tests.
            debug_assert!(
                false,
                "internal: AV1 reached NAL splitter; demuxer state is inconsistent"
            );
            None
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
        // Validate the spec-mandated forbidden + reserved bits before
        // peeling off the field-level decode. `pid` is sentinel 0; the
        // demuxer patches it in pes_emit.rs (same pattern as the other
        // AV1 issues this splitter emits).
        if (header & 0x80) != 0 {
            issues.push(NonConformantIssue::Av1ObuHeader {
                pid: 0,
                kind: Av1ObuHeaderKind::ForbiddenBit,
            });
        }
        if (header & 0x01) != 0 {
            issues.push(NonConformantIssue::Av1ObuHeader {
                pid: 0,
                kind: Av1ObuHeaderKind::ReservedBit,
            });
        }
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
            // Per AV1 §5.3.3 the low 3 reserved bits MUST be 0.
            if (ext & 0x07) != 0 {
                issues.push(NonConformantIssue::Av1ObuHeader {
                    pid: 0,
                    kind: Av1ObuHeaderKind::ExtensionReservedBits,
                });
            }
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

/// Outcome of the AV1-binding `ts_open_bitstream_unit()` unwrap step.
///
/// Used by the demuxer when `DemuxerConfig::av1_carriage ==
/// Av1CarriageMode::Mpeg2TsBinding` to detect binding-non-conformant input
/// and fall back to raw-OBU parsing while surfacing a
/// [`NonConformantIssue::Av1MissingTsObuFraming`] issue.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Av1BindingUnwrap {
    /// PES payload was binding-framed: 4-byte start code `0x00 0x00 0x00 0x02`
    /// observed at offset 0, OBU bytes unwrapped (emulation-prevention `0x03`
    /// bytes stripped). Returned vector is ready for [`split_obus`].
    Conformant(Vec<u8>),
    /// PES payload did not start with the `ts_open_bitstream_unit` start
    /// code. Caller should surface
    /// [`NonConformantIssue::Av1MissingTsObuFraming`] and fall through to
    /// raw-OBU parsing.
    MissingFraming,
}

/// Unwrap AV1-binding `ts_open_bitstream_unit()` framing from a PES payload.
///
/// Per AV1-in-MPEG-2-TS binding §3.2:
/// - Each OBU is prefixed with the 4-byte sequence `0x00 0x00 0x00 0x02`.
/// - Inside the body, any sequence `0x00 0x00 0x0X` (X ≤ 0x02) has had a
///   `0x03` emulation-prevention byte inserted between the second `0x00`
///   and the `0x0X`; the unwrap strips those.
///
/// On a malformed input (start code missing, truncated escape) the unwrap
/// returns [`Av1BindingUnwrap::MissingFraming`]; the demuxer treats that
/// as a non-conformance signal and falls back to raw-OBU parsing in
/// lenient mode.
pub(crate) fn unwrap_av1_binding(payload: &[u8]) -> Av1BindingUnwrap {
    // Binding §3.2 start code is 4 bytes; anything shorter can't carry it.
    if payload.len() < 4 || payload[0..4] != [0x00, 0x00, 0x00, 0x02] {
        return Av1BindingUnwrap::MissingFraming;
    }
    // Strip the start-code prefix and walk the body, removing
    // emulation-prevention 0x03 bytes that follow a run of two 0x00s
    // when the next byte after the 0x03 is ≤ 0x02 (matches the muxer's
    // injection rule in `wrap_av1_obus_binding`).
    let mut out = Vec::with_capacity(payload.len().saturating_sub(4));
    let mut zero_run = 0u8;
    let mut i = 4usize;
    while i < payload.len() {
        let b = payload[i];
        if zero_run >= 2 && b == 0x03 && i + 1 < payload.len() && payload[i + 1] <= 0x02 {
            // Drop the emulation-prevention byte; reset zero_run so the
            // next 0x00 starts a fresh run.
            zero_run = 0;
            i += 1;
            continue;
        }
        out.push(b);
        if b == 0x00 {
            zero_run = zero_run.saturating_add(1);
        } else {
            zero_run = 0;
        }
        i += 1;
    }
    Av1BindingUnwrap::Conformant(out)
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
    //
    // The AU cell wrapper IS the structural primitive we recognize — the
    // inner payload's shape (KLV-LS or otherwise opaque metadata) is
    // the consumer's concern, not the demuxer's. `read_metadata_au_cell`
    // validates the 5-byte header (reserved-bit checks, declared
    // AU_cell_data_length consistency); a successful parse + Complete
    // CFI is enough to surface SyncAuCell.
    if payload.len() >= 5 {
        if let Ok((header, inner)) = read_metadata_au_cell(payload) {
            match header.cell_fragment_indication {
                CellFragmentIndication::Complete => {
                    return KlvShape::SyncAuCell {
                        klv: inner.to_vec(),
                        header,
                    };
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

/// Outcome of parsing the EN 300 743 §6.2 PES_data_field envelope on a
/// DVB-subtitle PES payload.
///
/// The envelope wire format is:
///
/// ```text
///   data_identifier(1) + subtitle_stream_id(1) + segments(N) + end_marker(0xFF)
/// ```
///
/// Three terminal shapes per the spec + permissive carriage handling:
///
/// * `Conformant` — `data_identifier == 0x20`, `subtitle_stream_id == 0x00`,
///   end marker present. Strip and emit.
/// * `NonConformantDataId` — envelope is well-formed (stream_id + marker
///   match) but `data_identifier` falls in the legacy permissive carriage
///   range (`0x20..=0x3F | 0x70..=0x7F` per EN 300 743 §7.1) other than the
///   exact `0x20` required by §6.2 Table 3. Stripping still produces
///   sensible segment bytes; caller decides whether to emit a sample.
/// * `Malformed` — envelope shape doesn't match (too short, bad stream_id,
///   missing end marker, or data_identifier completely outside the
///   permissive range). Caller falls through to passthrough so strict-mode
///   consumers can still observe the unexpected payload shape.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DvbSubStripResult<'a> {
    /// Conformant envelope per EN 300 743 §6.2 Table 3.
    Conformant(&'a [u8]),
    /// Envelope well-formed but `data_identifier != 0x20`. The legacy
    /// permissive range (`0x20..=0x3F | 0x70..=0x7F`) is accepted here so
    /// the caller can choose strict reject vs. lenient pass-through.
    NonConformantDataId { observed: u8, stripped: &'a [u8] },
    /// Envelope shape invalid — caller should fall back to passthrough.
    Malformed,
}

/// Parse the EN 300 743 §6.2 PES_data_field envelope from a DVB-subtitle
/// PES payload.
///
/// See [`DvbSubStripResult`] for the per-outcome contract. The
/// `subtitle_stream_id` is fixed at `0x00` per §6.2. The end-marker byte
/// `0xFF` per §6.2.
pub(crate) fn strip_dvb_sub_envelope(bytes: &[u8]) -> DvbSubStripResult<'_> {
    if bytes.len() < 3 {
        return DvbSubStripResult::Malformed;
    }
    let data_id = bytes[0];
    let in_permissive_range = matches!(data_id, 0x20..=0x3F | 0x70..=0x7F);
    if !in_permissive_range || bytes[1] != 0x00 || *bytes.last().unwrap() != 0xFF {
        return DvbSubStripResult::Malformed;
    }
    let stripped = &bytes[2..bytes.len() - 1];
    if data_id == 0x20 {
        DvbSubStripResult::Conformant(stripped)
    } else {
        DvbSubStripResult::NonConformantDataId {
            observed: data_id,
            stripped,
        }
    }
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
        let (nals, issues) = split_nals(&buf, VideoCodec::H264);
        assert_eq!(nals.len(), 2);
        assert!(issues.is_empty(), "conformant input emits no issues");
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
        let (nals, issues) = split_nals(&buf, VideoCodec::H265);
        assert_eq!(nals.len(), 2);
        assert!(issues.is_empty(), "conformant input emits no issues");
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
        let (nals, issues) = split_nals(&buf, VideoCodec::H264);
        assert_eq!(nals.len(), 1);
        assert!(issues.is_empty());
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
        let (nals_264, issues_264) = split_nals(&[], VideoCodec::H264);
        assert!(nals_264.is_empty() && issues_264.is_empty());
        let (nals_265, issues_265) = split_nals(&[], VideoCodec::H265);
        assert!(nals_265.is_empty() && issues_265.is_empty());
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
        let (nals, issues) = split_nals(&buf, VideoCodec::H266);
        assert_eq!(nals.len(), 2);
        assert!(issues.is_empty(), "conformant H.266 emits no issues");
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
        let (nals, issues) = split_nals(&buf, VideoCodec::H264);
        assert!(nals.is_empty() && issues.is_empty());
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
        assert_eq!(
            strip_dvb_sub_envelope(&wrapped),
            DvbSubStripResult::Conformant(&segs[..])
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_hd_range_is_non_conformant_data_id() {
        // 0x70..=0x7F is the legacy permissive HD-subtitle range per
        // EN 300 743 §7.1, but §6.2 Table 3 binds DVB-subtitle to exactly
        // 0x20. Surface as NonConformantDataId so caller (strict vs lenient)
        // decides whether to emit the sample.
        let wrapped = [0x70, 0x00, 0x0F, 0x10, 0xFF];
        assert_eq!(
            strip_dvb_sub_envelope(&wrapped),
            DvbSubStripResult::NonConformantDataId {
                observed: 0x70,
                stripped: &[0x0F, 0x10][..],
            }
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_permissive_range_above_0x20_is_non_conformant() {
        // 0x21 sits in 0x20..=0x3F but isn't the §6.2 binding (which is 0x20
        // exact). Surface as NonConformantDataId.
        let wrapped = [0x21, 0x00, 0x0F, 0xAB, 0xFF];
        assert_eq!(
            strip_dvb_sub_envelope(&wrapped),
            DvbSubStripResult::NonConformantDataId {
                observed: 0x21,
                stripped: &[0x0F, 0xAB][..],
            }
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_missing_marker() {
        assert_eq!(
            strip_dvb_sub_envelope(&[0x20, 0x00, 0xAB, 0xCD]),
            DvbSubStripResult::Malformed
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_bad_data_identifier() {
        // 0x40 is outside both 0x20..=0x3F and 0x70..=0x7F — Malformed (not
        // even in the legacy permissive range).
        assert_eq!(
            strip_dvb_sub_envelope(&[0x40, 0x00, 0xAB, 0xFF]),
            DvbSubStripResult::Malformed
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_rejects_bad_stream_id() {
        // subtitle_stream_id must be 0x00 per §6.2.
        assert_eq!(
            strip_dvb_sub_envelope(&[0x20, 0x01, 0xAB, 0xFF]),
            DvbSubStripResult::Malformed
        );
    }

    #[test]
    fn strip_dvb_sub_envelope_too_short() {
        assert_eq!(
            strip_dvb_sub_envelope(&[0x20]),
            DvbSubStripResult::Malformed
        );
        assert_eq!(strip_dvb_sub_envelope(&[]), DvbSubStripResult::Malformed);
    }

    // The AV1 arm of `parse_one_nal` is unreachable in normal demuxer flow
    // (AV1 routes to `split_obus`). Pre-Phase-0 it called `unimplemented!`,
    // which abort-panicked the host on hostile or buggy upstream. The fix:
    // `debug_assert!` + `None` — debug builds still detect regressions via
    // assert; release builds gracefully yield no NALs. The two tests below
    // pin both behaviors.
    //
    // Typed-error promotion (`parse_one_nal -> Result<Option<NalUnit>, _>`)
    // is deferred to Phase 1 where signature SemVer ratchets are in scope.

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "AV1 reached NAL splitter")]
    fn split_nals_av1_debug_asserts_no_unimplemented_panic() {
        // Pre-fix: panicked with `unimplemented!("AV1 uses OBU framing...")`.
        // Post-fix in debug: panics with the debug_assert message instead,
        // which is the regression detector.
        let _ = split_nals(&[0x00, 0x00, 0x01, 0xAA, 0xBB], VideoCodec::Av1);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn split_nals_av1_no_panic_release() {
        // Release builds (no debug_assertions): the defense-in-depth arm
        // returns None from parse_one_nal, so split_nals yields an empty
        // Vec without panicking.
        let (nals, _issues) = split_nals(&[0x00, 0x00, 0x01, 0xAA, 0xBB], VideoCodec::Av1);
        assert!(nals.is_empty(), "AV1 must yield no NALs from NAL splitter");
    }

    // -----------------------------------------------------------------
    // B9 — NAL-header constraint enforcement tests
    // (validate-1 Phase 2 Wave B Task 9)
    // -----------------------------------------------------------------

    #[test]
    fn h264_forbidden_zero_bit_set_emits_nal_header_issue() {
        // forbidden_zero_bit=1 → header high bit set. Byte 0x80 alone:
        // forbidden=1, ref_idc=00, nal_type=0 (unspecified). The NAL is
        // still emitted (no spec-mandated discard for H.264) but the
        // issue is surfaced.
        let buf = vec![0x00, 0x00, 0x01, 0x80, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H264);
        assert_eq!(nals.len(), 1, "H.264 forbidden-bit NAL still emitted");
        assert_eq!(issues.len(), 1, "exactly one NalHeader issue raised");
        match &issues[0] {
            NonConformantIssue::NalHeader {
                codec: VideoCodec::H264,
                kind: NalHeaderKind::ForbiddenZeroBit,
            } => {}
            other => panic!("expected H.264 ForbiddenZeroBit, got {other:?}"),
        }
    }

    #[test]
    fn h265_forbidden_zero_bit_set_emits_nal_header_issue() {
        // H.265 byte 0: forbidden(1) | nal_type(6) | top-bit-of-layer_id(1).
        // 0x80 sets forbidden=1, leaves nal_type=0, layer_id top=0.
        // Byte 1: layer_id_low(5) | temporal_id_plus1(3); use 0x01 →
        // temporal_id_plus1=1 (valid) so we isolate the forbidden-bit issue.
        let buf = vec![0x00, 0x00, 0x01, 0x80, 0x01, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H265);
        assert_eq!(nals.len(), 1, "H.265 forbidden-bit NAL still emitted");
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::NalHeader {
                    codec: VideoCodec::H265,
                    kind: NalHeaderKind::ForbiddenZeroBit
                }
            )),
            "expected H.265 ForbiddenZeroBit issue, got {issues:?}"
        );
    }

    #[test]
    fn h265_zero_temporal_id_plus1_emits_issue() {
        // H.265: temporal_id_plus1=0 (forbidden per §7.3.1.2). Header
        // bytes: byte0=0x40 (forbidden=0, nal_type=32 VPS, layer_id_top=0),
        // byte1=0x00 (layer_id_low=0, temporal_id_plus1=0).
        let buf = vec![0x00, 0x00, 0x01, 0x40, 0x00, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H265);
        assert_eq!(nals.len(), 1);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::NalHeader {
                    codec: VideoCodec::H265,
                    kind: NalHeaderKind::ZeroTemporalIdPlus1
                }
            )),
            "expected H.265 ZeroTemporalIdPlus1 issue, got {issues:?}"
        );
    }

    #[test]
    fn h266_reserved_bit_set_drops_nal_and_emits_issue() {
        // H.266: byte0 forbidden(1) | reserved(1) | layer_id(6).
        // 0x40 → forbidden=0, reserved=1, layer_id=0. byte1=0x71 →
        // nal_type=14 (VPS), temporal_id_plus1=1. Per H.266 §7.3.1.2
        // receivers MUST discard NALs with nuh_reserved_zero_bit set;
        // the NAL is dropped from `nals` but the issue is still emitted.
        let buf = vec![0x00, 0x00, 0x01, 0x40, 0x71, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H266);
        assert!(
            nals.is_empty(),
            "H.266 spec mandates discard for reserved-bit NALs"
        );
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::NalHeader {
                    codec: VideoCodec::H266,
                    kind: NalHeaderKind::ReservedBit
                }
            )),
            "expected H.266 ReservedBit issue, got {issues:?}"
        );
    }

    #[test]
    fn h266_layer_id_out_of_range_drops_nal_and_emits_issue() {
        // H.266: layer_id=56 violates §7.4.2.2 (allowed range 0..=55).
        // byte0 forbidden(1) | reserved(1) | layer_id(6) = 0x38 →
        // forbidden=0, reserved=0, layer_id=56 (0x38 = 0b00111000;
        // low 6 bits = 0b111000 = 56). byte1=0x71 → nal_type=14, t+1=1.
        // Receivers MUST discard.
        let buf = vec![0x00, 0x00, 0x01, 0x38, 0x71, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H266);
        assert!(
            nals.is_empty(),
            "H.266 spec mandates discard for layer_id > 55"
        );
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::NalHeader {
                    codec: VideoCodec::H266,
                    kind: NalHeaderKind::LayerIdOutOfRange { id: 56 }
                }
            )),
            "expected H.266 LayerIdOutOfRange{{id=56}}, got {issues:?}"
        );
    }

    #[test]
    fn h266_forbidden_zero_bit_set_emits_issue_keeps_nal() {
        // H.266 forbidden_zero_bit set, otherwise valid (reserved=0,
        // layer_id=0, t+1=1). byte0=0x80, byte1=0x71. forbidden-bit
        // violation alone is NOT spec-mandated discard for H.266 (only
        // reserved + layer>55 are); the NAL stays in the output.
        let buf = vec![0x00, 0x00, 0x01, 0x80, 0x71, 0xAA];
        let (nals, issues) = split_nals(&buf, VideoCodec::H266);
        assert_eq!(
            nals.len(),
            1,
            "H.266 forbidden-bit alone does not mandate discard"
        );
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::NalHeader {
                    codec: VideoCodec::H266,
                    kind: NalHeaderKind::ForbiddenZeroBit
                }
            )),
            "expected H.266 ForbiddenZeroBit issue, got {issues:?}"
        );
    }

    // -----------------------------------------------------------------
    // B10 — AV1 OBU header bit validation tests
    // (validate-1 Phase 2 Wave B Task 10)
    // -----------------------------------------------------------------

    #[test]
    fn split_obus_forbidden_bit_set_emits_issue() {
        // obu_type=1 (Seq Header), ext_flag=0, has_size=1, reserved_1bit=0,
        // forbidden_bit=1. Header byte = 0x80 | (1<<3) | 0x02 = 0x8A.
        let buf = vec![0x8A, 0x00];
        let (_obus, issues) = split_obus(&buf);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::Av1ObuHeader {
                    pid: 0,
                    kind: Av1ObuHeaderKind::ForbiddenBit
                }
            )),
            "expected ForbiddenBit issue, got {issues:?}"
        );
    }

    #[test]
    fn split_obus_reserved_bit_set_emits_issue() {
        // obu_type=1, ext_flag=0, has_size=1, reserved_1bit=1 → low bit set.
        // Header = (1<<3) | 0b011 = 0x0B.
        let buf = vec![0x0B, 0x00];
        let (_obus, issues) = split_obus(&buf);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::Av1ObuHeader {
                    pid: 0,
                    kind: Av1ObuHeaderKind::ReservedBit
                }
            )),
            "expected ReservedBit issue, got {issues:?}"
        );
    }

    #[test]
    fn split_obus_extension_reserved_bits_set_emits_issue() {
        // obu_type=1, ext_flag=1, has_size=1 → header = (1<<3) | 0b110 = 0x0E.
        // Extension byte: temporal_id(3) | spatial_id(2) | reserved(3).
        // Set the low 3 reserved bits: ext = 0x07. Then size LEB128 = 0x00.
        let buf = vec![0x0E, 0x07, 0x00];
        let (_obus, issues) = split_obus(&buf);
        assert!(
            issues.iter().any(|i| matches!(
                i,
                NonConformantIssue::Av1ObuHeader {
                    pid: 0,
                    kind: Av1ObuHeaderKind::ExtensionReservedBits
                }
            )),
            "expected ExtensionReservedBits issue, got {issues:?}"
        );
    }
}
