//! Profile → `MuxerConfig` mapping: the single place a [`Profile`] becomes
//! a concrete mux configuration plus the stream handles to push onto.
//!
//! `gen::run` (this task) and `send::run` (Task 6) both build their muxer
//! from [`build_config`], so the wire shape a profile produces is defined
//! in exactly one place.

use tst_core::mpegts::mux::{
    AudioCodec, AudioStreamHandle, KlvStreamHandle, KlvStreamType, Muxer, MuxerConfig,
    MuxerProgramConfigBuilder, VideoCodec as MuxVideoCodec, VideoStreamHandle,
};

use crate::profiles::{KlvMode, Profile, VideoCodec};

/// Program 1's PIDs — the same conventional values `MuxerConfig::default()`
/// and the mux examples use (`examples/muxing/mux_to_file.rs`,
/// `mux_av1_with_klv.rs`).
const PROG1_PMT_PID: u16 = 0x1000;
const PROG1_VIDEO_PID: u16 = 0x1011;
const PROG1_KLV_PID: u16 = 0x1031;
const PROG1_AUDIO_PID: u16 = 0x1041;

/// Program 2's PIDs, for `two-program` — a clean 0x100 stride off program
/// 1's range (`examples/muxing/repack_two_programs.rs`'s renumbering
/// scheme), so the two programs' PIDs never collide.
const PROG2_PMT_PID: u16 = 0x1100;
const PROG2_VIDEO_PID: u16 = 0x1111;
const PROG2_KLV_PID: u16 = 0x1131;

/// The stream handles [`build_config`] resolved for a [`Profile`]'s
/// [`MuxerConfig`], in program-declaration order. `video`/`klv` have one
/// entry per configured program (2 for `two-program`, 1 otherwise);
/// `audio` is `Some` only when [`Profile::audio`].
pub struct StreamHandles {
    pub video: Vec<VideoStreamHandle>,
    pub klv: Vec<KlvStreamHandle>,
    pub audio: Option<AudioStreamHandle>,
}

fn mux_video_codec(c: VideoCodec) -> MuxVideoCodec {
    match c {
        VideoCodec::H264 => MuxVideoCodec::H264,
        VideoCodec::H265 => MuxVideoCodec::H265,
        VideoCodec::H266 => MuxVideoCodec::H266,
        VideoCodec::Av1 => MuxVideoCodec::Av1,
    }
}

/// Build the `MuxerConfig` a [`Profile`] describes, plus the stream
/// handles to push onto it.
///
/// KLV carriage: `Sync` maps to `SynchronousMetadata` + `carries_pts:
/// true` (required by `MuxerConfig::validate` for that stream type);
/// `Async`/`AsyncWithMisp` both map to `PrivateData` + `carries_pts:
/// false` — MISP only changes what rides inside the video SEI, not the
/// KLV PID's carriage (mirrors `verify::expected_klv_carriage`).
pub fn build_config(p: &Profile) -> (MuxerConfig, StreamHandles) {
    let codec = mux_video_codec(p.video);
    let (klv_stream_type, carries_pts) = match p.klv {
        KlvMode::Sync => (KlvStreamType::SynchronousMetadata, true),
        KlvMode::Async | KlvMode::AsyncWithMisp => (KlvStreamType::PrivateData, false),
    };

    let mut prog1 = MuxerProgramConfigBuilder::new(1, PROG1_PMT_PID);
    prog1.add_video(PROG1_VIDEO_PID, codec);
    prog1.add_klv(PROG1_KLV_PID, klv_stream_type, carries_pts);
    if p.audio {
        prog1.add_audio(PROG1_AUDIO_PID, AudioCodec::Aac);
    }

    let mut builder = MuxerConfig::builder();
    builder.add_program(prog1.build());

    if p.programs == 2 {
        let mut prog2 = MuxerProgramConfigBuilder::new(2, PROG2_PMT_PID);
        prog2.add_video(PROG2_VIDEO_PID, codec);
        prog2.add_klv(PROG2_KLV_PID, klv_stream_type, carries_pts);
        builder.add_program(prog2.build());
    }

    builder.pcr_interval_ms(p.pcr_interval_ms);
    builder.psi_interval_ms(p.psi_interval_ms);
    if let Some(mode) = p.av1_mode {
        builder.av1_carriage(mode);
    }

    let cfg = builder
        .build()
        .expect("every registered Profile must build a valid MuxerConfig");

    // Handles are a deterministic function of `cfg` (program-declaration
    // order) — construct a throwaway `Muxer` purely to read them back via
    // its own accessors, rather than re-deriving the packed
    // (program, within-program) layout here.
    let muxer = Muxer::new(cfg.clone())
        .expect("a MuxerConfig that just passed builder validation always constructs");
    let handles = StreamHandles {
        video: muxer.video_handles(),
        klv: muxer.klv_handles(),
        audio: muxer.audio_handles().into_iter().next(),
    };

    (cfg, handles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles;

    #[test]
    fn single_program_profile_has_one_video_and_klv_handle_no_audio() {
        let p = profiles::by_name("baseline").expect("baseline profile must exist");
        let (cfg, handles) = build_config(p);
        assert_eq!(cfg.programs.len(), 1);
        assert_eq!(handles.video.len(), 1);
        assert_eq!(handles.klv.len(), 1);
        assert!(handles.audio.is_none());
    }

    #[test]
    fn audio_profile_has_one_audio_handle() {
        let p = profiles::by_name("audio").expect("audio profile must exist");
        let (_, handles) = build_config(p);
        assert!(handles.audio.is_some());
    }

    #[test]
    fn two_program_profile_has_two_video_and_klv_handles() {
        let p = profiles::by_name("two-program").expect("two-program profile must exist");
        let (cfg, handles) = build_config(p);
        assert_eq!(cfg.programs.len(), 2);
        assert_eq!(handles.video.len(), 2);
        assert_eq!(handles.klv.len(), 2);
    }

    #[test]
    fn klv_sync_profile_sets_synchronous_metadata_carries_pts() {
        let p = profiles::by_name("klv-sync").expect("klv-sync profile must exist");
        let (cfg, _) = build_config(p);
        let klv_spec = cfg.programs[0]
            .streams
            .iter()
            .find(|s| matches!(s, tst_core::mpegts::mux::StreamSpec::Klv { .. }))
            .expect("klv-sync config must have a KLV stream");
        match klv_spec {
            tst_core::mpegts::mux::StreamSpec::Klv {
                stream_type,
                carries_pts,
                ..
            } => {
                assert_eq!(*stream_type, KlvStreamType::SynchronousMetadata);
                assert!(*carries_pts);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn av1_profile_carries_the_configured_carriage_mode() {
        let p = profiles::by_name("av1-klv-a").expect("av1-klv-a profile must exist");
        let (cfg, _) = build_config(p);
        assert_eq!(
            cfg.av1_carriage,
            tst_core::mpegts::mux::Av1CarriageMode::InteropRawObu
        );
    }
}
