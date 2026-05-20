//! Video push paths (`push_video` / `push_video_to`) + video handle
//! accessors (`video_handles` / `video_stream_handle` /
//! `video_handles_for_program`) + the single-stream-handle helper
//! (`single_video_handle`).
//!
//! Each public method's full rustdoc preamble stays with the method
//! body — the doc comments are part of the API contract, not the
//! source-file organization.

use crate::error::MuxError;
use crate::mpegts::common::{Pcr27mhz, Pts90khz};

use super::Muxer;
use super::pes::{MAX_PES_HEADER_SIZE, PesFlags, PesPtsField, STREAM_ID_VIDEO, write_pes_header};
use super::state::{ts_packets_for, validate_annex_b};
use super::ts::{AdaptationField, write_packet};
use super::types::{StreamKind, VideoCodec, VideoStreamHandle};

impl Muxer {
    /// Push one H.264 / H.265 access unit in Annex-B framing.
    ///
    /// `key_frame=true` causes the first TS packet of the resulting PES to
    /// carry an adaptation field with `random_access_indicator` set.
    ///
    /// State is unchanged on any error (push is atomic — either the AU lands
    /// in the muxer queue or none of its TS packets do).
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::AmbiguousTarget`] when zero or more than one video
    ///   stream is configured across all programs — call [`Self::push_video_to`]
    ///   with an explicit handle in that case.
    /// - [`MuxError::InvalidNal`] if `nal` does not begin with an Annex-B
    ///   start code (H.264 / H.265 / H.266 only — AV1 OBU payloads route
    ///   through `push_video_to` and skip this check).
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_video(
        &mut self,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        // The single-target API only resolves when exactly one video stream
        // is configured across all programs. N=0 and N>1 are both ambiguous.
        let total_video: usize = self.video_streams.iter().map(|v| v.len()).sum();
        if total_video != 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: StreamKind::Video,
                count: total_video,
            });
        }
        let handle = self.single_video_handle();
        self.push_video_to(handle, nal, pts, key_frame)
    }

    /// Locate the program containing the lone video stream.
    ///
    /// Precondition: caller has verified `total_video == 1` (typically via
    /// `push_video`'s `AmbiguousTarget` check). The `expect()` is safe because
    /// `total_video == 1` guarantees exactly one program has a non-empty
    /// video stream list.
    fn single_video_handle(&self) -> VideoStreamHandle {
        let (prog_idx, _within_idx) = self
            .video_streams
            .iter()
            .enumerate()
            .find(|(_p, v)| !v.is_empty())
            .map(|(p, _)| (p, 0))
            .expect("total_video == 1 guarantees one non-empty program");
        VideoStreamHandle::pack(prog_idx, 0)
    }

    /// All `VideoStreamHandle`s for this muxer, in `(program, within-program)`
    /// declaration order. One handle per `StreamSpec::Video` across all programs.
    pub fn video_handles(&self) -> Vec<VideoStreamHandle> {
        self.video_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| VideoStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Handle for the i-th video stream in `programs[0]`, or `None` if out of
    /// range. Convenience for single-program callers.
    pub fn video_stream_handle(&self, index: usize) -> Option<VideoStreamHandle> {
        if !self.video_streams.is_empty() && index < self.video_streams[0].len() {
            Some(VideoStreamHandle::pack(0, index))
        } else {
            None
        }
    }

    /// Video stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn video_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<VideoStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.video_streams[prog_idx].len())
            .map(|s_idx| VideoStreamHandle::pack(prog_idx, s_idx))
            .collect())
    }

    /// Push one H.264 / H.265 / H.266 / AV1 access unit on a specific
    /// video stream.
    ///
    /// `pts` and `key_frame` carry the same semantics as
    /// [`Self::push_video`]. The caller selects the destination stream
    /// via the [`VideoStreamHandle`] obtained from
    /// [`Self::video_handles`] / [`Self::video_stream_handle`].
    ///
    /// AV1 streams expect OBU bitstream input (AV1 spec §5) and skip
    /// the Annex-B start-code check; H.264 / H.265 / H.266 require
    /// Annex-B framing.
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video_to` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's configured video stream count.
    /// - [`MuxError::InvalidNal`] if `nal` does not begin with an Annex-B
    ///   start code (only checked for H.264 / H.265 / H.266; AV1 OBU
    ///   payloads pass through this check).
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_video_to(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.video_streams.len() || within_idx >= self.video_streams[prog_idx].len()
        {
            // Report the raw packed value as an opaque index so the error message
            // carries the full handle encoding without confusing prog vs within.
            return Err(MuxError::InvalidStreamHandle {
                kind: StreamKind::Video,
                index: handle.0 as usize,
            });
        }
        let video_pid = self.video_streams[prog_idx][within_idx].pid;
        // AV1 carries OBUs (AV1 spec §5), not Annex-B NAL units — its push
        // payload is the OBU bitstream and must skip the Annex-B start-code
        // check that H.264 / H.265 / H.266 require.
        if !matches!(
            self.video_streams[prog_idx][within_idx].codec,
            VideoCodec::Av1
        ) {
            validate_annex_b(nal)?;
        }

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        // AV1-in-MPEG-2-TS binding (`av1-mpeg2-ts-binding.html`) carriage notes:
        // - §3.4 `data_alignment_indicator=1` — REQUIRED, set below per binding.
        // - §3.4 `stream_id=0xBD` (private_stream_1) — DEVIATION: library uses
        //   `STREAM_ID_VIDEO` (0xE0) for ffmpeg + libaom interop. See
        //   `docs/deferred-features.md` §"AV1-in-MPEG-2-TS binding §3.2 / §3.4
        //   carriage conformance" for rationale + trigger-to-revisit.
        // - §3.2 `ts_open_bitstream_unit()` framing (start codes + emulation
        //   prevention bytes) — DEVIATION: library carries raw OBUs in the
        //   PES payload (low-overhead bitstream format directly). Same
        //   deferred-features.md entry covers this; strict-mode receivers
        //   would surface `NonConformantIssue::Av1MissingTsObuFraming` /
        //   `Av1WrongStreamId` (planned additions — not yet in
        //   `mpegts::demux::event`).
        // H.222.0 §2.4.3.7 leaves the alignment bit codec-defined for
        // H.264 / H.265 / H.266 — keep them unset.
        let pes_flags = PesFlags {
            data_alignment_indicator: matches!(
                self.video_streams[prog_idx][within_idx].codec,
                VideoCodec::Av1
            ),
        };
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(pts),
            None,
            pes_flags,
        );

        let total = header_len + nal.len();
        let video_packets = ts_packets_for(total);
        let psi_packets = self.psi_packets_due(prog_idx, pts.as_ticks());
        // Validate-1 C3: when the PCR PID hasn't received payload within
        // `pcr_interval_ms`, the muxer injects a standalone PCR-only
        // adaptation-only packet on it. Reserve one packet for that.
        // Returns 0 when the current push lands on the PCR PID (the
        // in-band PCR-on-push path below handles emission instead).
        let pcr_only_packets = self.pcr_only_packets_due(prog_idx, pts.as_ticks(), video_pid);

        if self.queue.len() + psi_packets + pcr_only_packets + video_packets
            > self.config.buffer_packets
        {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets as u64,
            });
        }

        self.maybe_emit_psi(prog_idx, pts.as_ticks());
        self.maybe_emit_pcr_only(prog_idx, pts.as_ticks(), video_pid);

        self.pes_scratch.clear();
        self.pes_scratch.extend_from_slice(&header[..header_len]);
        self.pes_scratch.extend_from_slice(nal);

        let mut cursor = 0;
        let mut first = true;
        while cursor < self.pes_scratch.len() {
            let mut adaptation = AdaptationField::default();
            if first {
                if key_frame {
                    adaptation.random_access = true;
                }
                if self.pcr_pids[prog_idx] == video_pid {
                    // Per H.222.0 V9 §2.4.3.5: random_access_indicator may
                    // only be set on PCR_PID packets that also carry the PCR
                    // fields. Force PCR emission when key-frame coincides
                    // with this PID even if pcr_due() would otherwise return
                    // false — matches TSDuck / ffmpeg behavior. Random-access
                    // point + PCR coincide; downstream seekers benefit.
                    if self.pcr_due(prog_idx, pts.as_ticks()) || key_frame {
                        let pcr = Pcr27mhz::from_pts(pts);
                        adaptation.pcr = Some(pcr);
                        self.pcr_last[prog_idx] = Some(pcr.as_ticks());
                    }
                }
            }
            let mut pkt = [0u8; 188];
            let payload_start = cursor;
            let result = write_packet(
                &mut pkt,
                video_pid,
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
        if let Some(s) = self.per_stream.get_mut(&video_pid) {
            s.items += 1;
            s.bytes += nal.len() as u64;
        }

        // Codec-counter bump. Codec is known per-stream via VideoCodec on
        // the stream config; key_frame is a clean caller-supplied signal
        // for random-access (no NAL inspection required).
        let codec = self.video_streams[prog_idx][within_idx].codec;
        let nals_count = crate::codec::util::count_nal_units(nal, codec);
        let ra_count = u64::from(key_frame);
        self.bump_video_counters(video_pid, nals_count, ra_count);

        Ok(())
    }
}
