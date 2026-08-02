//! Synthetic AU + KLV fixture factory for the interop test driver.
//!
//! Every byte recipe here is lifted from an existing tst-core generator,
//! example, or test rather than invented fresh, so the shapes are the same
//! ones tst-core's own muxer/demuxer and codec parsers already accept. See
//! each function's doc comment for its source.

use crate::profiles::VideoCodec;
use tst_core::klv::st0601::{UasDatalinkLs, encode_to_vec};

/// Every 30th frame (0, 30, 60, ...) is a keyframe.
const KEYFRAME_INTERVAL: u32 = 30;

/// Annex-B start code shared by the H.264/H.265/H.266 AUs below.
const ANNEX_B_START: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

/// Build one video access unit for `codec` at `frame_idx`. Returns
/// `(bytes, is_keyframe)` — a keyframe (parameter sets + IDR slice/frame)
/// every 30th frame, an inter slice/frame otherwise.
pub fn video_au(codec: VideoCodec, frame_idx: u32) -> (Vec<u8>, bool) {
    let keyframe = frame_idx % KEYFRAME_INTERVAL == 0;
    let bytes = match codec {
        VideoCodec::H264 => h264_au(frame_idx, keyframe),
        VideoCodec::H265 => h265_au(frame_idx, keyframe),
        VideoCodec::H266 => h266_au(frame_idx, keyframe),
        VideoCodec::Av1 => av1_au(frame_idx, keyframe),
    };
    (bytes, keyframe)
}

/// Filler bytes for a non-keyframe slice/frame, sized off `frame_idx` the
/// way `examples/muxing/mux_h265_with_klv.rs:133` varies its inter-AU
/// sizes (`au.resize(1000 + (i as usize % 200), 0xA5)`), scaled down for a
/// lightweight fixture.
fn filler(frame_idx: u32) -> Vec<u8> {
    let len = 32 + (frame_idx as usize % 32);
    vec![0xA5; len]
}

/// H.264 Annex-B AU. Keyframe = SPS (NAL type 7) + PPS (type 8) + IDR
/// slice (type 5), byte-for-byte from `build_h264_keyframe_au` in
/// `examples/muxing/mux_audio_video_klv.rs:163-185`. Non-keyframe = a
/// single non-IDR slice NAL (type 1, `nal_ref_idc=2` → header byte
/// `0x41`), the same NAL-header formula that recipe uses for its slice.
fn h264_au(frame_idx: u32, keyframe: bool) -> Vec<u8> {
    let mut au = Vec::new();
    if keyframe {
        au.extend_from_slice(&ANNEX_B_START);
        au.extend_from_slice(&[0x67, 0x42, 0x00, 0x1F, 0xE9, 0x02, 0x80, 0x14, 0x07, 0x80]);
        au.extend_from_slice(&ANNEX_B_START);
        au.extend_from_slice(&[0x68, 0xCE, 0x06, 0xE2]);
        au.extend_from_slice(&ANNEX_B_START);
        au.extend_from_slice(&[
            0x65, 0x88, 0x80, 0x40, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00,
        ]);
    } else {
        au.extend_from_slice(&ANNEX_B_START);
        au.push(0x41); // non-IDR slice: nal_ref_idc=2, nal_unit_type=1
        au.extend(filler(frame_idx));
    }
    au
}

/// H.265 Annex-B AU, the exact shape built inline in
/// `examples/muxing/mux_h265_with_klv.rs:115-133`: 2-byte NAL header
/// (`nal_type << 1`, then `0x01` for `nuh_layer_id=0` +
/// `nuh_temporal_id_plus1=1`), IDR_W_RADL (19) for the keyframe, TRAIL_N
/// (1) otherwise.
fn h265_au(frame_idx: u32, keyframe: bool) -> Vec<u8> {
    let nal_type: u8 = if keyframe { 19 } else { 1 };
    let mut au = Vec::new();
    au.extend_from_slice(&ANNEX_B_START);
    au.push(nal_type << 1);
    au.push(0x01);
    au.extend(filler(frame_idx));
    au
}

/// H.266 Annex-B AU. Parameter-set RBSP bytes are lifted verbatim from
/// `crates/tst-core/tests/tools/gen_h266_fixtures.rs` (`vps_main10`,
/// `sps_main10`, `pps_main10` — the same bytes as the
/// `codec::h266::{vps,sps,pps}::tests::minimal_*_rbsp()` unit fixtures),
/// wrapped in the 2-byte VVC NAL header per H.266 V4 Table 5
/// (`nal_unit_type << 3 | nuh_temporal_id_plus1`; VPS_NUT=14, SPS_NUT=15,
/// PPS_NUT=16, IDR_W_RADL=7, TRAIL_NUT=0 — confirmed against
/// `codec::h266::mod::parse_parameter_sets` and
/// `codec::h266::slice_header_light::is_idr_nal`).
fn h266_au(frame_idx: u32, keyframe: bool) -> Vec<u8> {
    fn nal(nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
        let mut v = ANNEX_B_START.to_vec();
        v.push(0x00); // forbidden_zero_bit | nuh_reserved_zero_bit | nuh_layer_id[5:0]=0
        v.push((nal_type << 3) | 0x01); // nal_unit_type(5) | nuh_temporal_id_plus1=1
        v.extend_from_slice(rbsp);
        v
    }
    let mut au = Vec::new();
    if keyframe {
        au.extend(nal(14, &[0x00, 0x02])); // VPS
        au.extend(nal(
            15,
            &[
                0x00, 0x09, 0x02, 0x3f, 0x00, 0x00, 0x00, 0x28, 0x20, 0x3c, 0x48, 0x00, 0x5d, 0xb0,
                0xf8, 0x06, 0x02, 0x08, 0x00, 0x02,
            ],
        )); // SPS
        au.extend(nal(16, &[0x00, 0x20])); // PPS
        au.extend(nal(7, &filler(frame_idx))); // IDR_W_RADL slice
    } else {
        au.extend(nal(0, &filler(frame_idx))); // TRAIL_NUT slice
    }
    au
}

/// AV1 low-overhead OBU sequence — AV1 has no Annex-B framing, so this is
/// the codec's own "start-code" equivalent. Structure and the
/// `(obu_type << 3) | 0x02` header formula (`obu_has_size_field=1`,
/// single-byte LEB128 size) are lifted verbatim from
/// `synthetic_av1_au`/`obu` in
/// `crates/tst-core/tests/codec/av1_carriage_roundtrip.rs:25-47`: a
/// Temporal Delimiter + Sequence Header + Frame Header + Tile Group. The
/// Sequence Header body and the keyframe Frame Header body are the exact
/// bytes from `crates/tst-core/tests/tools/gen_av1_fixtures.rs`
/// (`seq_header_main_320x240`, `frame_header_keyframe`); the non-keyframe
/// Frame Header body flips `frame_type` from KEY_FRAME(0) to
/// INTER_FRAME(1) per that file's documented bit layout
/// (`show_existing_frame(1) | frame_type(2) | show_frame(1)` in the high
/// nibble).
fn av1_au(frame_idx: u32, keyframe: bool) -> Vec<u8> {
    fn obu(obu_type: u8, body: &[u8]) -> Vec<u8> {
        let header = (obu_type << 3) | 0x02;
        let mut v = vec![header, body.len() as u8];
        v.extend_from_slice(body);
        v
    }
    const SEQ_HEADER: [u8; 10] = [0, 0, 0, 4, 60, 255, 188, 0, 0, 0];
    let frame_header: [u8; 1] = if keyframe { [0x10] } else { [0x30] };

    let mut au = Vec::new();
    au.extend(obu(2, &[])); // Temporal Delimiter (always empty body)
    if keyframe {
        au.extend(obu(1, &SEQ_HEADER)); // Sequence Header
    }
    au.extend(obu(3, &frame_header)); // Frame Header
    let tile_len = 3 + (frame_idx as usize % 8);
    au.extend(obu(4, &vec![0xA5; tile_len])); // Tile Group
    au
}

/// Build an ST 0601 UAS Datalink Local Set with a handful of numeric tags
/// derived from `seq`, so consecutive records differ on the wire. Returns
/// raw KLV LS bytes (Universal Label + BER length + TLVs + checksum) —
/// **not** AU-cell-wrapped. Per this workspace's KLV convention,
/// `Muxer::push_klv` / `MuxSender::send_klv` prepend the 5-byte
/// `Metadata_AU_cell` header themselves for `SynchronousMetadata`
/// streams, so callers always pass raw LS bytes here.
pub fn klv_record(seq: u32) -> Vec<u8> {
    let rec = UasDatalinkLs {
        // Tag 2: Precision Time Stamp (µs since Unix epoch) — same base
        // value as `gen_synthetic_fixtures.rs::minimal()`, offset by
        // `seq` seconds.
        timestamp_us: Some(1_700_000_000_000_000 + (seq as u64) * 1_000_000),
        // Tag 5: Platform Heading Angle — encode range [0, 360] deg.
        platform_heading_deg: Some((seq % 360) as f64),
        // Tag 13/14: Sensor Latitude/Longitude — walk a small grid so
        // records differ without leaving valid encode ranges.
        sensor_lat_deg: Some(38.0 + (seq as f64) * 0.0001),
        sensor_lon_deg: Some(-121.5 - (seq as f64) * 0.0001),
        ..Default::default()
    };
    encode_to_vec(&rec).expect("well-formed UasDatalinkLs always encodes")
}

/// Build a single 7-byte-header ADTS AAC frame (no CRC). Header layout
/// lifted verbatim from `make_adts_buf` in
/// `crates/tst-core/benches/codec_parsers.rs:105-140` (MPEG-2 ID, AAC-LC
/// profile, 44.1 kHz, stereo, 1 raw data block) — the same bit layout
/// `codec::aac::frames` parses. Body bytes vary with `seq` so consecutive
/// frames differ on the wire.
pub fn aac_frame(seq: u32) -> Vec<u8> {
    const BODY_LEN: usize = 100;
    const FRAME_LEN: u32 = 7 + BODY_LEN as u32;
    const SAMPLE_RATE_INDEX: u8 = 4; // 44100 Hz
    const CHANNEL_CONFIG: u8 = 2; // stereo

    let mut h = [0u8; 7];
    h[0] = 0xFF;
    h[1] = 0b1111_0000 | (1 << 3) | 1; // ID=MPEG-2, layer=0, no CRC
    h[2] = (1 << 6) | ((SAMPLE_RATE_INDEX & 0xF) << 2) | ((CHANNEL_CONFIG >> 2) & 1); // profile=LC
    h[3] = ((CHANNEL_CONFIG & 0b11) << 6) | (((FRAME_LEN >> 11) & 0b11) as u8);
    h[4] = ((FRAME_LEN >> 3) & 0xFF) as u8;
    h[5] = (((FRAME_LEN & 0b111) as u8) << 5) | 0b1_1111;
    h[6] = 0b11_1111 << 2; // buffer_fullness low bits | num_raw_data_blocks=0

    let mut frame = h.to_vec();
    frame.extend((0..BODY_LEN).map(|i| (seq as u8).wrapping_add(i as u8)));
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use tst_core::klv::st0601::decode;

    #[test]
    fn klv_record_varies_with_seq_and_decodes_back() {
        let a = klv_record(1);
        let b = klv_record(2);
        assert_ne!(a, b, "records for different seq must differ on the wire");
        for bytes in [&a, &b] {
            let _ = decode(bytes).expect("well-formed ST 0601 record must decode");
        }
    }

    #[test]
    fn video_au_frame_zero_is_keyframe_frame_one_is_not() {
        for codec in [
            VideoCodec::H264,
            VideoCodec::H265,
            VideoCodec::H266,
            VideoCodec::Av1,
        ] {
            let (frame0, key0) = video_au(codec, 0);
            let (frame1, key1) = video_au(codec, 1);
            assert!(key0, "{codec:?} frame 0 must be a keyframe");
            assert!(!key1, "{codec:?} frame 1 must not be a keyframe");
            assert!(!frame0.is_empty(), "{codec:?} frame 0 must be non-empty");
            assert!(!frame1.is_empty(), "{codec:?} frame 1 must be non-empty");
            assert_prefix(codec, &frame0);
            assert_prefix(codec, &frame1);
        }
    }

    fn assert_prefix(codec: VideoCodec, bytes: &[u8]) {
        match codec {
            VideoCodec::H264 | VideoCodec::H265 | VideoCodec::H266 => {
                assert!(
                    bytes.starts_with(&ANNEX_B_START),
                    "{codec:?} AU must start with the Annex-B start code"
                );
            }
            VideoCodec::Av1 => {
                // Temporal Delimiter OBU header: (obu_type=2 << 3) | has_size_field(0x02).
                assert_eq!(
                    bytes[0], 0x12,
                    "AV1 AU must start with a Temporal Delimiter OBU header"
                );
            }
        }
    }

    #[test]
    fn aac_frame_parses_as_one_lc_stereo_44100_frame() {
        let a = aac_frame(1);
        let b = aac_frame(2);
        assert_ne!(a, b, "frames for different seq must differ on the wire");

        let mut frames = tst_core::codec::aac::frames(&a);
        let frame = frames
            .next()
            .expect("frame should parse")
            .expect("frame should parse");
        assert_eq!(frame.profile, tst_core::codec::aac::AacProfile::Lc);
        assert_eq!(frame.sample_rate_hz, 44_100);
        assert_eq!(frame.channel_configuration, 2);
        assert!(frames.next().is_none(), "buffer holds exactly one frame");
    }
}
