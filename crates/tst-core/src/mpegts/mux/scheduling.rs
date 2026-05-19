//! PSI + PCR cadence scheduling for `Muxer`.
//!
//! Owns the `psi_last` / `pcr_last` per-program ring-buffer state and
//! the `psi_due` / `pcr_due` predicates that gate PSI/PCR emission per
//! the configured intervals.
//!
//! `maybe_emit_psi` is the centerpiece — it emits one PAT + one PMT per
//! program when the per-program PSI tick is due, tracks the last-emission
//! PTS, and updates `psi_last[prog_idx]` so the next call's `psi_due`
//! returns false until the next interval boundary.

use super::psi::{PmtStreamEntry, write_pat_packet, write_pmt_packet};
use super::{AudioCodec, KlvStreamType, Muxer, StreamSpec, StreamType, VideoCodec};
use crate::mpegts::common::Pts90khz;

impl Muxer {
    pub(super) fn psi_due(&self, prog_idx: usize, pts_90khz: i64) -> bool {
        match self.psi_last[prog_idx] {
            None => true,
            Some(last_masked) => {
                let now_masked = Pts90khz::new(pts_90khz).masked_33bit();
                let delta = crate::mpegts::common::pts_diff_33bit(now_masked, last_masked);
                delta >= self.psi_interval_90khz
            }
        }
    }

    pub(super) fn pcr_due(&self, prog_idx: usize, pts_90khz: i64) -> bool {
        match self.pcr_last[prog_idx] {
            None => true,
            Some(last) => {
                // PCR is at 27 MHz; the 33-bit base wraps at 2^33 base ticks.
                // Convert both to 33-bit base and use the same modular helper,
                // then compare in 90 kHz units.
                let now_base_masked = Pts90khz::new(pts_90khz).masked_33bit();
                let last_base_masked = (last / 300) & ((1u64 << 33) - 1);
                let delta_90khz =
                    crate::mpegts::common::pts_diff_33bit(now_base_masked, last_base_masked);
                let threshold_90khz = (self.pcr_interval_27mhz / 300) as i64;
                delta_90khz >= threshold_90khz
            }
        }
    }

    pub(super) fn maybe_emit_psi(&mut self, prog_idx: usize, pts_90khz: i64) {
        if !self.psi_due(prog_idx, pts_90khz) {
            return;
        }
        // Emit one PAT that lists all programs, then one PMT per program.
        // The PAT is emitted on every PSI tick regardless of which program
        // triggered the tick, so all programs' state is always visible to
        // receivers after a single PSI interval.
        let mut pat = [0u8; 188];
        write_pat_packet(&mut pat, &self.config, &mut self.counters);
        self.queue.push_back(pat);

        // One PMT per program — iterate the full program set so every program
        // gets a fresh PMT on the tick (not just the triggering program).
        for pidx in 0..self.config.programs.len() {
            let prog = &self.config.programs[pidx];
            let mut entries: Vec<PmtStreamEntry> = Vec::with_capacity(prog.streams.len());
            for (i, spec) in prog.streams.iter().enumerate() {
                let stream_type = match spec {
                    StreamSpec::Video {
                        codec: VideoCodec::H264,
                        ..
                    } => StreamType::H264,
                    StreamSpec::Video {
                        codec: VideoCodec::H265,
                        ..
                    } => StreamType::H265,
                    StreamSpec::Video {
                        codec: VideoCodec::H266,
                        ..
                    } => StreamType::H266,
                    // AV1 rides PMT stream_type 0x06; the AV01
                    // registration_descriptor (auto-emitted at the top of the
                    // PMT descriptor loop, suppressed when caller supplies
                    // their own) disambiguates the codec at the wire level.
                    StreamSpec::Video {
                        codec: VideoCodec::Av1,
                        ..
                    } => StreamType::KlvPrivate,
                    StreamSpec::Klv {
                        stream_type: KlvStreamType::PrivateData,
                        ..
                    } => StreamType::KlvPrivate,
                    StreamSpec::Klv {
                        stream_type: KlvStreamType::SynchronousMetadata,
                        ..
                    } => StreamType::KlvSyncMetadata,
                    StreamSpec::Audio {
                        codec: AudioCodec::Mp2,
                        ..
                    } => StreamType::AudioMp2,
                    StreamSpec::Audio {
                        codec: AudioCodec::Aac,
                        ..
                    } => StreamType::AudioAac,
                    StreamSpec::Audio {
                        codec: AudioCodec::AacLatm,
                        ..
                    } => StreamType::AudioAacLatm,
                    StreamSpec::Audio {
                        codec: AudioCodec::Ac3,
                        ..
                    } => StreamType::AudioAc3,
                    // All four subtitle codecs share PMT stream_type 0x06
                    // (PrivateData); the per-stream descriptor cache carries
                    // the codec-specific disambiguator (subtitling_descriptor /
                    // teletext_descriptor / Registration "GA94" / "VTTC").
                    StreamSpec::Subtitle { .. } => StreamType::KlvPrivate,
                };
                entries.push(PmtStreamEntry {
                    stream_type,
                    elementary_pid: spec.pid(),
                    descriptors: &self.pmt_descriptor_caches[pidx][i],
                });
            }

            let mut pmt = [0u8; 188];
            write_pmt_packet(
                &mut pmt,
                prog,
                self.pcr_pids[pidx],
                &entries,
                &mut self.counters,
            )
            .expect("validated MuxerConfig must produce single-section PMT");
            self.queue.push_back(pmt);
        }

        self.psi_last[prog_idx] = Some(Pts90khz::new(pts_90khz).masked_33bit());
    }
}
