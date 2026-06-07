//! `PairingDemuxer` self-validating golden round-trip.
//!
//! The oracle is the bare `Demuxer` + `Pairer` path fed the SAME TS
//! bytes. `PairingDemuxer` must produce a byte-identical `PairerOutput`
//! sequence — proving the composite is a faithful, lossless wrapper.

use std::time::Duration;
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::demux::Demuxer;
use tst_core::mpegts::mux::{
    KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec as MuxVideoCodec,
};
use tst_pipeline::ext::pairing::{
    Pairer, PairerConfig, PairerMode, PairerOutput, PairingDemuxer, PairingDemuxerConfig,
};

const VIDEO_PID: u16 = 0x100;
const KLV_PID: u16 = 0x102;

fn minimal_h264_au() -> Vec<u8> {
    vec![
        0x00, 0x00, 0x00, 0x01, 0x09, 0x10, 0x00, 0x00, 0x00, 0x01, 0x65, 0xAA, 0xBB, 0xCC,
    ]
}

fn dummy_klv() -> Vec<u8> {
    let mut v = vec![
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00,
    ];
    v.push(4);
    v.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    v
}

fn drain_mux(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 188 * 64];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// Sync-KLV stream, 5 video frames + 5 KLV records at matching PTS.
fn sync_klv_bytes() -> Vec<u8> {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(VIDEO_PID, MuxVideoCodec::H264);
        prog.add_klv(KLV_PID, KlvStreamType::SynchronousMetadata, true);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().unwrap()
    };
    let mut mux = Muxer::new(cfg).unwrap();
    for i in 0..5 {
        let pts = 90_000 + i * 3000;
        mux.push_video(&minimal_h264_au(), Pts90khz::new(pts), true)
            .unwrap();
        mux.push_klv(&dummy_klv(), Pts90khz::new(pts), 0x00)
            .unwrap();
    }
    drain_mux(&mut mux)
}

fn nearest_config() -> PairerConfig {
    let mut opts = PairerConfig::default();
    opts.mode = PairerMode::Realtime;
    opts.tolerance = Duration::from_millis(100);
    opts.max_buffered_klv = 16;
    opts.max_buffered_video = 16;
    opts
}

/// Oracle: bare Demuxer (default config) + the given Pairer, same bytes.
fn oracle(bytes: &[u8], mut pairer: Pairer) -> Vec<PairerOutput> {
    let mut demux = Demuxer::new();
    let mut out = Vec::new();
    demux.feed(bytes).unwrap();
    while let Some(e) = demux.next_event() {
        out.extend(pairer.feed(e));
    }
    demux.flush();
    while let Some(e) = demux.next_event() {
        out.extend(pairer.feed(e));
    }
    out.extend(pairer.flush());
    out
}

#[test]
fn with_config_matches_bare_pairer_oracle() {
    let bytes = sync_klv_bytes();

    let expected = oracle(
        &bytes,
        Pairer::with_config(VIDEO_PID, KLV_PID, nearest_config()),
    );

    let mut cfg = PairingDemuxerConfig::default();
    cfg.pairer = nearest_config();
    // cfg.demuxer keeps its Default value (default parsing settings).
    let mut pd = PairingDemuxer::with_config(VIDEO_PID, KLV_PID, cfg);
    let mut got = pd.feed(&bytes).unwrap();
    got.extend(pd.flush());

    assert_eq!(
        got, expected,
        "PairingDemuxer diverged from bare Pairer oracle"
    );
    // Sanity: the sync stream pairs all 5 frames.
    let paired = got
        .iter()
        .filter(|o| matches!(o, PairerOutput::Paired { .. }))
        .count();
    assert_eq!(paired, 5, "expected 5 Paired, got {paired}");
    assert_eq!(pd.stats().paired, 5);
}
