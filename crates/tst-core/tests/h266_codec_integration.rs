//! End-to-end: synthetic H.266 NALs -> mux -> demux -> codec::h266 parse.
//!
//! Composes three layers shipped earlier in this phase:
//!   1. `mpegts::mux` carries an Annex-B H.266 access unit at PMT
//!      stream_type 0x33.
//!   2. `mpegts::demux` recovers NALs and tags them
//!      `NalUnit::H266 { .. }` under `VideoCodec::H266`.
//!   3. `codec::h266::parse_parameter_sets` decodes the recovered
//!      parameter-set NALs into `H266ParameterSets`.
//!
//! The minimal VPS/SPS/PPS bodies match the per-set unit tests at
//! `codec/h266/{vps,sps,pps}.rs`. The SPS bit-builder is replicated
//! inline (see `BitWriter` below) so this integration test has zero
//! coupling to internal `pub(crate)` helpers.

use tst_core::codec::h266::{H266ParameterSets, parse_parameter_sets};
use tst_core::mpegts::demux::Demuxer;
use tst_core::mpegts::demux::event::{
    DemuxEvent, NalUnit, SamplePayload, VideoCodec, VideoPayload,
};
use tst_core::mpegts::mux::{Config, Muxer, VideoCodec as MuxVideoCodec};

/// Wrap a NAL body in an Annex-B start code + 2-byte H.266 NAL header.
/// Per H.266 V4 §7.3.1.2:
///   byte 0: forbidden_zero_bit(1)=0 | nuh_reserved_zero_bit(1)=0 | nuh_layer_id(6)=0
///   byte 1: nal_unit_type(5) | nuh_temporal_id_plus1(3)=1
fn h266_nal(nal_type: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![0x00, 0x00, 0x00, 0x01, 0x00, (nal_type << 3) | 0x01];
    v.extend_from_slice(body);
    v
}

/// Minimal VPS RBSP: vps_id=0, max_layers=1, max_sub_layers=1.
/// Matches `codec::h266::vps::tests` minimal fixture.
fn minimal_vps_rbsp() -> Vec<u8> {
    vec![0x00, 0x02]
}

/// Minimal PPS RBSP: pps_id=0, sps_id=0.
/// Matches `codec::h266::pps::tests` minimal fixture.
fn minimal_pps_rbsp() -> Vec<u8> {
    vec![0x00, 0x20]
}

/// Inline bit-builder, copied verbatim from
/// `codec::h266::sps::tests::BitWriter`. Kept here so this test does
/// not depend on `pub(crate)` internals.
struct BitWriter {
    bytes: Vec<u8>,
    pos: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            pos: 0,
        }
    }
    fn write(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            let bit = ((value >> i) & 1) as u8;
            let byte_idx = (self.pos / 8) as usize;
            let bit_in_byte = 7 - (self.pos % 8);
            if byte_idx == self.bytes.len() {
                self.bytes.push(0);
            }
            self.bytes[byte_idx] |= bit << bit_in_byte;
            self.pos += 1;
        }
    }
    /// Exp-Golomb ue(v) per H.266 §9.3.2.2.
    fn write_ue(&mut self, value: u32) {
        let v = value + 1;
        let leading_zeros = 31 - v.leading_zeros();
        for _ in 0..leading_zeros {
            self.write(0, 1);
        }
        self.write(v, leading_zeros + 1);
    }
    /// rbsp_trailing_bits(): one '1' bit + zero-pad to byte align.
    fn end_rbsp(&mut self) {
        self.write(1, 1);
        while self.pos % 8 != 0 {
            self.write(0, 1);
        }
    }
}

/// Construct a minimal valid H.266 SPS bitstream:
/// sps_id=0, vps_id=0, 320x240, 8-bit 4:2:0, Main 10 profile @ Level 4.0.
/// Mirror of `codec::h266::sps::tests::minimal_sps_rbsp`.
fn minimal_sps_rbsp() -> Vec<u8> {
    let mut bw = BitWriter::new();

    // §7.3.2.4 SPS header.
    bw.write(0, 4); // sps_seq_parameter_set_id
    bw.write(0, 4); // sps_video_parameter_set_id
    bw.write(0, 3); // sps_max_sublayers_minus1
    bw.write(1, 2); // sps_chroma_format_idc = 1 (4:2:0)
    bw.write(0, 2); // sps_log2_ctu_size_minus5
    bw.write(1, 1); // sps_ptl_dpb_hrd_params_present_flag = 1

    // §7.3.3.1 profile_tier_level(profileTierPresentFlag=1, MaxNumSubLayersMinus1=0).
    bw.write(1, 7); // general_profile_idc = 1 (Main 10)
    bw.write(0, 1); // general_tier_flag = 0 (Main tier)
    bw.write(63, 8); // general_level_idc = 63 (Level 4.0)
    bw.write(0, 1); // ptl_frame_only_constraint_flag
    bw.write(0, 1); // ptl_multilayer_enabled_flag
    bw.write(0, 1); // gci_present_flag = 0
    bw.write(0, 5); // byte-align PTL to 24 bits
    bw.write(0, 8); // ptl_num_sub_profiles = 0

    bw.write(0, 1); // sps_gdr_enabled_flag
    bw.write(0, 1); // sps_ref_pic_resampling_enabled_flag = 0

    bw.write_ue(320); // sps_pic_width_max_in_luma_samples
    bw.write_ue(240); // sps_pic_height_max_in_luma_samples

    bw.write(0, 1); // sps_conformance_window_flag = 0
    bw.write(0, 1); // sps_subpic_info_present_flag = 0
    bw.write_ue(0); // sps_bitdepth_minus8 = 0 -> bit_depth = 8

    bw.end_rbsp();
    bw.bytes
}

fn drain_mux(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

fn collect_events(d: &mut Demuxer) -> Vec<DemuxEvent> {
    let mut out = Vec::new();
    while let Some(e) = d.next_event() {
        out.push(e);
    }
    out
}

#[test]
fn h266_end_to_end_parses_minimal_vps_sps_pps() {
    let cfg = Config::builder()
        .add_program(1, 0x100)
        .add_video(0x101, MuxVideoCodec::H266)
        .end_program()
        .build()
        .unwrap();
    let mut mux = Muxer::new(cfg).unwrap();
    let h = mux.video_handles()[0];

    // Build an AU with valid minimal VPS/SPS/PPS from the per-task tests.
    let mut au = Vec::new();
    au.extend(h266_nal(20, &[0x10])); // AUD_NUT
    au.extend(h266_nal(14, &minimal_vps_rbsp())); // VPS_NUT
    au.extend(h266_nal(15, &minimal_sps_rbsp())); // SPS_NUT
    au.extend(h266_nal(16, &minimal_pps_rbsp())); // PPS_NUT

    mux.push_video_to(h, &au, 90_000, true).unwrap();
    let ts_bytes = drain_mux(&mut mux);

    let mut demux = Demuxer::new();
    demux.feed(&ts_bytes).unwrap();
    // Unbounded video PES (PES_packet_length=0) buffers in-flight; flush
    // drains it. Live receive loops do this on TransportError::Closed.
    demux.flush();
    let events = collect_events(&mut demux);

    let nals: Vec<NalUnit> = events
        .iter()
        .filter_map(|e| match e {
            DemuxEvent::Sample {
                payload:
                    SamplePayload::Video {
                        codec: VideoCodec::H266,
                        payload: VideoPayload::Nals(nals),
                    },
                ..
            } => Some(nals.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    assert!(
        !nals.is_empty(),
        "expected H.266 NALs from demuxer, got events: {events:?}"
    );

    let sets: H266ParameterSets =
        parse_parameter_sets(&nals).expect("parse_parameter_sets should succeed");
    assert_eq!(sets.vpses.len(), 1);
    assert_eq!(sets.spses.len(), 1);
    assert_eq!(sets.ppses.len(), 1);

    // Spot-check parsed values match the inputs.
    assert_eq!(sets.vpses[0].vps_id, 0);
    assert_eq!(sets.spses[0].sps_id, 0);
    assert_eq!(sets.spses[0].vps_id, 0);
    assert_eq!(sets.spses[0].width, 320);
    assert_eq!(sets.spses[0].height, 240);
    assert_eq!(sets.ppses[0].pps_id, 0);
}
