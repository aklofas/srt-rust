//! Audio push paths (`push_audio` / `push_audio_to`) + audio handle
//! accessors (`audio_handles` / `audio_handles_for_program`).
//!
//! Each public method's full rustdoc preamble stays with the method
//! body — the doc comments are part of the API contract, not the
//! source-file organization.

use crate::error::MuxError;
use crate::mpegts::common::{Pcr27mhz, Pts90khz};

use super::Muxer;
use super::pes::{PesPtsField, write_audio_pes};
use super::state::ts_packets_for;
use super::ts::{AdaptationField, write_packet};
use super::types::{AudioStreamHandle, StreamKind};

impl Muxer {
    /// Push one audio frame buffer, single-stream shorthand.
    ///
    /// `pts` is required and becomes the PES PTS; audio has no DTS
    /// (no B-frame reorder). `frames` is one or more pre-framed audio frames
    /// concatenated by the caller.
    ///
    /// Resolves only when exactly one audio stream is configured across all
    /// programs. Otherwise rejects with [`MuxError::AmbiguousTarget`].
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_audio` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::NoAudioStreamsConfigured`] if no audio streams are
    ///   configured on this muxer.
    /// - [`MuxError::AmbiguousTarget`] when more than one audio stream is
    ///   configured — call [`Self::push_audio_to`] with an explicit handle.
    /// - [`MuxError::AudioTooLarge`] if `frames.len()` would overflow
    ///   `PES_packet_length`.
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_audio(&mut self, frames: &[u8], pts: Pts90khz) -> Result<(), MuxError> {
        let total_audio: usize = self.audio_streams.iter().map(|a| a.len()).sum();
        if total_audio == 0 {
            return Err(MuxError::NoAudioStreamsConfigured);
        }
        if total_audio > 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: StreamKind::Audio,
                count: total_audio,
            });
        }
        // Mirror push_video / push_klv: when exactly one stream exists, it is
        // at (prog_idx=0, within_idx=0) in audio_streams — the first program
        // that has audio is always index 0 in the nested vec. Note: if the lone
        // audio stream is in program 1 (prog_idx=1 in config), audio_streams[1]
        // is non-empty and audio_streams[0] is empty; pack(0,0) would resolve
        // to the wrong slot. Iterate to find the actual location.
        let (prog_idx, _within_idx) = self
            .audio_streams
            .iter()
            .enumerate()
            .find(|(_p, a)| !a.is_empty())
            .map(|(p, _)| (p, 0))
            .expect("total_audio == 1 guarantees one non-empty program");
        let handle = AudioStreamHandle::pack(prog_idx, 0);
        self.push_audio_to(handle, pts, frames)
    }

    /// Push one audio frame buffer on a specific audio stream.
    ///
    /// Routes to the audio stream identified by `handle`. Use the bare
    /// [`push_audio`][Self::push_audio] shorthand when exactly one audio
    /// stream is configured. Handles are obtained from
    /// [`audio_handles`][Self::audio_handles] /
    /// [`audio_handles_for_program`][Self::audio_handles_for_program].
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_audio_to` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's configured audio stream count (across all
    ///   programs).
    /// - [`MuxError::AudioTooLarge`] if `frames.len()` would overflow
    ///   `PES_packet_length` (max ~65527 bytes after PES overhead).
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_audio_to(
        &mut self,
        handle: AudioStreamHandle,
        pts: Pts90khz,
        frames: &[u8],
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.audio_streams.len() || within_idx >= self.audio_streams[prog_idx].len()
        {
            return Err(MuxError::InvalidStreamHandle {
                kind: StreamKind::Audio,
                index: handle.0 as usize,
            });
        }
        let audio_pid = self.audio_streams[prog_idx][within_idx].pid;
        let audio_codec = self.audio_streams[prog_idx][within_idx].codec;

        // Audio always uses PTS, so PES overhead is 3 (start code) + 5 (PTS) = 8 bytes.
        // The remaining space in the u16 PES_packet_length field is for flags, header_data_length,
        // and the payload. Guard against frames that would overflow PES_packet_length.
        let pes_overhead = 3usize + 5;
        let max_audio = (u16::MAX as usize) - pes_overhead;
        if frames.len() > max_audio {
            return Err(MuxError::AudioTooLarge {
                size: frames.len(),
                max: max_audio,
            });
        }

        let pes_pts = PesPtsField::PtsOnly(pts);
        self.pes_scratch.clear();
        write_audio_pes(
            &mut self.pes_scratch,
            audio_codec,
            within_idx as u8,
            pes_pts,
            frames,
        );

        let total = self.pes_scratch.len();
        let audio_packets = ts_packets_for(total);
        let psi_packets = self.psi_packets_due(prog_idx, pts.as_ticks());
        // Validate-1 C3: see push_video for the rationale. Audio is
        // typically high-cadence, but a low-frame-rate stream (sparse
        // language tracks, sign-language audio) could still drift.
        let pcr_only_packets = self.pcr_only_packets_due(prog_idx, pts.as_ticks(), audio_pid);

        if self.queue.len() + psi_packets + pcr_only_packets + audio_packets
            > self.config.buffer_packets
        {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets as u64,
            });
        }

        self.maybe_emit_psi(prog_idx, pts.as_ticks());
        self.maybe_emit_pcr_only(prog_idx, pts.as_ticks(), audio_pid);

        let mut cursor = 0;
        let mut first = true;
        while cursor < self.pes_scratch.len() {
            let mut adaptation = AdaptationField::default();
            if first
                && self.pcr_pids[prog_idx] == audio_pid
                && self.pcr_due(prog_idx, pts.as_ticks())
            {
                let pcr = Pcr27mhz::from_pts(pts);
                adaptation.pcr = Some(pcr);
                self.pcr_last[prog_idx] = Some(pcr.as_ticks());
            }
            let mut pkt = [0u8; 188];
            let payload_start = cursor;
            let result = write_packet(
                &mut pkt,
                audio_pid,
                first,
                adaptation,
                &self.pes_scratch[payload_start..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        // Count on the Ok path only — after all early-returns above.
        if let Some(s) = self.per_stream.get_mut(&audio_pid) {
            s.items += 1;
            s.bytes += frames.len() as u64;
        }

        // Codec-counter bump. AAC + MP2 have lazy-stateless frame iterators
        // (codec::aac::frames / codec::mpegaudio::frames). LATM and AC-3
        // don't yet — those PIDs leave the codec counter unmaterialized
        // so the accessor returns Some(Unknown) via per_stream fallback.
        let frames_delta: u64 = match audio_codec {
            crate::mpegts::mux::AudioCodec::Aac => crate::codec::aac::frames(frames)
                .filter_map(Result::ok)
                .count() as u64,
            crate::mpegts::mux::AudioCodec::Mp2 => crate::codec::mpegaudio::frames(frames)
                .filter_map(Result::ok)
                .count() as u64,
            crate::mpegts::mux::AudioCodec::AacLatm | crate::mpegts::mux::AudioCodec::Ac3 => 0,
        };
        if frames_delta > 0 {
            self.bump_audio_counters(audio_pid, frames_delta);
        }

        Ok(())
    }

    /// All `AudioStreamHandle`s for this muxer, in `(program, within-program)`
    /// declaration order. One handle per `StreamSpec::Audio` across all programs.
    pub fn audio_handles(&self) -> Vec<AudioStreamHandle> {
        self.audio_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| AudioStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Audio stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn audio_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<AudioStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.audio_streams[prog_idx].len())
            .map(|s_idx| AudioStreamHandle::pack(prog_idx, s_idx))
            .collect())
    }
}
