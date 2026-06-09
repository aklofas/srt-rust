//! End-to-end test: feed an IDR access unit (SPS + PPS + IDR slice) through
//! `mpegts::mux` → `mpegts::demux` → `codec::h264::parse_parameter_sets` and
//! assert the demuxer-emitted NALs carry the expected SPS dimensions.
//!
//! This exercises the full receive-side codec path: the muxer wraps the Annex-B
//! AU into PES + TS packets, the demuxer re-assembles and strips framing back
//! to individual NAL units, and the codec parser reads the SPS RBSP.

use tst_core::codec::h264;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer, SamplePayload, VideoPayload, split_video};
use tst_core::mpegts::mux::{Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

const SPS_RBSP: &[u8] = include_bytes!("../fixtures/codec/h264/h264_1080p_high40_bt709_sps.bin");
const PPS_RBSP: &[u8] = include_bytes!("../fixtures/codec/h264/h264_1080p_high40_bt709_pps.bin");

/// Build a minimal Annex-B access unit: SPS NAL + PPS NAL + IDR slice NAL.
///
/// Each NAL is prefixed with a 4-byte start code (0x00 0x00 0x00 0x01) and
/// the one-byte NAL unit header (forbidden_zero_bit=0, nal_ref_idc, nal_unit_type).
/// The RBSP payloads for SPS and PPS come from the shared test fixtures; the
/// IDR slice is a minimal stub (just the header byte + a few bytes) — ffprobe
/// identifies the codec from the PMT stream_type, not the slice content, so
/// the stub is enough for the routing and parser round-trip test.
fn build_annexb_au(sps_rbsp: &[u8], pps_rbsp: &[u8]) -> Vec<u8> {
    let sc = [0u8, 0, 0, 1];
    let mut au = Vec::new();

    // SPS: NAL type 7, nal_ref_idc 3 → header byte 0x67
    au.extend_from_slice(&sc);
    au.push(0x67);
    au.extend_from_slice(sps_rbsp);

    // PPS: NAL type 8, nal_ref_idc 3 → header byte 0x68
    au.extend_from_slice(&sc);
    au.push(0x68);
    au.extend_from_slice(pps_rbsp);

    // IDR slice: NAL type 5, nal_ref_idc 3 → header byte 0x65.
    // A slice header requires at least a couple of RBSP bytes to be
    // syntactically non-empty; the demuxer passes raw bytes without parsing
    // slice headers, so any non-empty stub works.
    au.extend_from_slice(&sc);
    au.extend_from_slice(&[0x65, 0x88, 0x84, 0x00, 0x00, 0x00]);

    au
}

#[test]
fn h264_idr_au_round_trips_through_mux_demux_parse() {
    // --- Mux side ---
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x0100, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("config builds")
    };

    let mut muxer = Muxer::new(cfg).expect("muxer construction");
    let video_h = muxer.video_stream_handle(0).expect("video handle");

    let au = build_annexb_au(SPS_RBSP, PPS_RBSP);
    // Push as a keyframe at PTS 0.
    muxer
        .push_video_to(video_h, &au, Pts90khz::new(0), true)
        .expect("push_video_to");

    // Drain all TS packets.  `pull` fills the provided slice one 188-byte
    // packet at a time and returns the number of bytes written (always a
    // multiple of 188, or 0 when the queue is empty).
    let mut ts_bytes = Vec::new();
    let mut buf = vec![0u8; 188 * 64];
    loop {
        let n = muxer.pull(&mut buf);
        if n == 0 {
            break;
        }
        ts_bytes.extend_from_slice(&buf[..n]);
    }
    assert!(!ts_bytes.is_empty(), "muxer produced no TS output");

    // --- Demux side ---
    let mut dx = Demuxer::new();

    // Feed all bytes in one shot.  Lenient mode (the default) never errors on
    // PSI/PES non-conformance — those surface as NonConformant events.
    dx.feed(&ts_bytes).expect("demux feed");

    // flush signals end-of-stream so the PES reassembler commits any trailing
    // AU that hasn't yet been terminated by a subsequent PES start code.
    // Without this the demuxer would hold the last (here only) video frame
    // in the reassembler and never emit a Sample event.
    dx.flush();

    // --- Parse side ---
    let mut found_sps = false;
    while let Some(ev) = dx.next_event() {
        if let DemuxEvent::Sample {
            payload: SamplePayload::Video { codec, raw, .. },
            ..
        } = ev
        {
            // Raw-first: split the encoded AU into NALs via the opt-in call.
            let VideoPayload::Nals(nals) = split_video(&raw, codec).0 else {
                continue;
            };
            // parse_parameter_sets walks every NalUnit in the AU and collects
            // SPS and PPS entries into typed maps.  Non-parameter-set NALs
            // (slice headers, IDR slices) are silently skipped.
            let ps = h264::parse_parameter_sets(&nals).expect("parse_parameter_sets");
            if let Some(sps) = ps.sps_by_id.get(&0) {
                assert_eq!(sps.width, 1920, "width mismatch");
                assert_eq!(sps.height, 1080, "height mismatch");
                assert_eq!(
                    sps.profile_idc, 100,
                    "profile_idc mismatch (expected High=100)"
                );
                assert_eq!(sps.level_idc, 40, "level_idc mismatch (expected Level 4.0)");
                found_sps = true;
            }
        }
    }

    assert!(
        found_sps,
        "demuxer did not emit a Sample event carrying the SPS NAL"
    );
}
