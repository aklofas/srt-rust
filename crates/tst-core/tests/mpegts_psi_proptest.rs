//! Property tests for MPEG-TS PSI build→parse round-trip + descriptors
//! build→parse round-trip + arbitrary-chunking invariance through
//! `Demuxer::feed`.
//!
//! All tests go through the public mux→demux path to avoid exposing
//! internal PSI build helpers — keeps Phase 6 zero-API-change.

use proptest::prelude::*;
use tst_core::mpegts::demux::psi::walk_descriptors;
use tst_core::mpegts::demux::{DemuxEvent, Demuxer};
use tst_core::mpegts::descriptors;
use tst_core::mpegts::mux::{Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

/// Drain all TS packets from a muxer into a single `Vec<u8>`.
fn drain(mux: &mut Muxer) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 188 * 64];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    bytes
}

/// Drive a fixed-config Muxer once, return the muxed TS bytes used by
/// the chunking-invariance test. Fixed inputs keep the chunking test
/// focused on the boundary-sequence property rather than on muxer
/// behavior — that's covered by `psi_roundtrip` above.
fn mux_fixed_one_program_stream() -> Vec<u8> {
    let mut prog = MuxerProgramConfigBuilder::new(1, 0x100);
    prog.add_video(0x101, VideoCodec::H264);
    let mut b = MuxerConfig::builder();
    b.add_program(prog.build());
    let cfg = b.build().expect("fixed config valid");

    let mut mux = Muxer::new(cfg).expect("muxer construct");
    // Minimal Annex-B NAL: 4-byte start code + AUD (NAL type 9, primary_pic_type=0)
    let nal = [0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
    for i in 0..16 {
        mux.push_video(&nal, i * 3000, true).expect("push_video");
    }
    drain(&mut mux)
}

proptest! {
    /// PSI mux→demux round-trip: configure a Muxer with a single program +
    /// video stream, pull all TS bytes, feed to a Demuxer, assert a
    /// `ProgramMap` event carries matching `program_number` and a
    /// `StreamInfo` with the configured video PID.
    #[test]
    fn psi_roundtrip(
        program_number in 1u16..=0xFFFE,
        pmt_pid in 0x0010u16..=0x1FFE,
        video_pid in 0x0010u16..=0x1FFE,
    ) {
        // Valid user PID range is 0x0010..=0x1FFE per pid::is_user_pid; both
        // PMT and stream PIDs must be distinct.
        prop_assume!(pmt_pid != video_pid);

        let mut prog = MuxerProgramConfigBuilder::new(program_number, pmt_pid);
        prog.add_video(video_pid, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        let cfg = b.build().expect("config valid");

        let mut mux = Muxer::new(cfg).expect("muxer construct");
        // Minimal Annex-B AUD NAL — just enough to trigger PES + PSI emission.
        let nal = [0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        mux.push_video(&nal, 0, true).expect("push_video");

        let bytes = drain(&mut mux);
        prop_assume!(!bytes.is_empty());

        let mut demux = Demuxer::new();
        demux.feed(&bytes).expect("demux feed");

        let mut found = false;
        while let Some(evt) = demux.next_event() {
            if let DemuxEvent::ProgramMap(pm) = evt {
                if pm.program_number == program_number
                    && pm.streams.iter().any(|s| s.pid == video_pid)
                {
                    found = true;
                    break;
                }
            }
        }
        prop_assert!(found, "PMT round-trip lost program/stream identity");
    }

    /// Descriptors build/parse round-trip:
    /// `descriptors::registration([_;4], &[])` then `walk_descriptors` yields
    /// one `RawDescriptor` with `tag=0x05` and `data == format_id`.
    #[test]
    fn descriptor_roundtrip(format_id in any::<[u8; 4]>()) {
        let bytes = descriptors::registration(format_id, &[]);
        let parsed = walk_descriptors(&bytes).expect("walk_descriptors ok");
        prop_assert_eq!(parsed.len(), 1);
        prop_assert_eq!(parsed[0].tag, 0x05);
        prop_assert_eq!(&parsed[0].data[..], &format_id[..]);
    }

    /// Demuxer chunking invariance: feeding a fixed muxed TS stream as one
    /// whole feed must produce the same event sequence as feeding it split
    /// at arbitrary boundaries. Catches buffer-boundary bugs in the framing
    /// state machine.
    #[test]
    fn demuxer_chunking_invariant(
        // 8192 chosen to comfortably exceed the fixed stream length
        // (~5.2 KiB with 16 video pushes) so the modulo mapping at line
        // 126 actually distributes boundaries across the full stream
        // rather than clustering in the first half.
        boundaries in proptest::collection::vec(0usize..=8192, 0..=32),
    ) {
        let bytes = mux_fixed_one_program_stream();
        prop_assume!(!bytes.is_empty());

        let mut once = Demuxer::new();
        once.feed(&bytes).expect("feed whole");
        let events_once: Vec<_> = std::iter::from_fn(|| once.next_event()).collect();

        // Map proptest-generated raw indices into the valid `[0, bytes.len()]`
        // range, sort + dedup, and always include the final byte index so the
        // last chunk gets fed.
        let mut splits: Vec<usize> = boundaries
            .into_iter()
            .map(|b| b % (bytes.len() + 1))
            .collect();
        splits.sort_unstable();
        splits.dedup();
        if splits.last().copied() != Some(bytes.len()) {
            splits.push(bytes.len());
        }

        let mut chunked = Demuxer::new();
        let mut last = 0;
        for s in splits {
            chunked.feed(&bytes[last..s]).expect("feed chunk");
            last = s;
        }
        let events_chunked: Vec<_> =
            std::iter::from_fn(|| chunked.next_event()).collect();

        prop_assert_eq!(events_once, events_chunked);
    }
}
