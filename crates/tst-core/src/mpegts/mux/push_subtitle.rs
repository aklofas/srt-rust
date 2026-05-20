//! Subtitle push paths (`push_subtitle` / `push_subtitle_to`) + subtitle
//! handle accessors (`subtitle_handles` / `subtitle_handles_for_program`).
//!
//! Each public method's full rustdoc preamble stays with the method
//! body — the doc comments are part of the API contract, not the
//! source-file organization.

use crate::error::MuxError;
use crate::mpegts::common::Pts90khz;

use super::Muxer;
use super::pes::{
    SubtitlePesShape, dvb_teletext_total_pes_bytes, dvb_teletext_will_auto_prepend,
    write_subtitle_pes,
};
use super::state::ts_packets_for;
use super::ts::{AdaptationField, write_packet};
use super::types::{StreamKind, SubtitleCodec, SubtitleStreamHandle};

impl Muxer {
    /// Push one subtitle PES unit, single-stream shorthand.
    ///
    /// `pts` is required and becomes the PES PTS — subtitles are
    /// rendered at presentation time, never reordered. `payload` is one
    /// complete logical subtitle unit (DVB-sub composition page,
    /// teletext data field, CEA-708 service block, or WebVTT cue);
    /// fragmentation across PES is not used.
    ///
    /// Resolves only when exactly one subtitle stream is configured
    /// across all programs. Otherwise rejects with
    /// [`MuxError::AmbiguousTarget`].
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_subtitle` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::NoSubtitleStreamsConfigured`] if no subtitle streams
    ///   are configured on this muxer.
    /// - [`MuxError::AmbiguousTarget`] when more than one subtitle stream
    ///   is configured — call [`Self::push_subtitle_to`] with an explicit
    ///   handle.
    /// - [`MuxError::SubtitleTooLarge`] if `payload.len()` would overflow
    ///   `PES_packet_length`.
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_subtitle(&mut self, pts: Pts90khz, payload: &[u8]) -> Result<(), MuxError> {
        let total_subtitle: usize = self.subtitle_streams.iter().map(|s| s.len()).sum();
        if total_subtitle == 0 {
            return Err(MuxError::NoSubtitleStreamsConfigured);
        }
        if total_subtitle > 1 {
            return Err(MuxError::AmbiguousTarget {
                kind: StreamKind::Subtitle,
                count: total_subtitle,
            });
        }
        // Locate the program with the lone subtitle stream — same iterate-to-find
        // pattern as `push_audio` / `push_klv` (the lone stream may not be in
        // program 0).
        let (prog_idx, _within_idx) = self
            .subtitle_streams
            .iter()
            .enumerate()
            .find(|(_p, s)| !s.is_empty())
            .map(|(p, _)| (p, 0))
            .expect("total_subtitle == 1 guarantees one non-empty program");
        let handle = SubtitleStreamHandle::pack(prog_idx, 0);
        self.push_subtitle_to(handle, pts, payload)
    }

    /// Push one subtitle PES unit on a specific subtitle stream.
    ///
    /// Routes to the subtitle stream identified by `handle`. Use the
    /// bare [`push_subtitle`][Self::push_subtitle] shorthand when
    /// exactly one subtitle stream is configured. Handles are obtained
    /// from [`subtitle_handles`][Self::subtitle_handles].
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_subtitle_to` — see `crates/tst-c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's configured subtitle stream count.
    /// - [`MuxError::SubtitleTooLarge`] if `payload.len()` would overflow
    ///   `PES_packet_length` (max 65527 bytes for DVB-sub / CEA-708 /
    ///   WebVTT shapes; DVB-teletext caps at 65458 when the writer
    ///   auto-prepends the EN 300 472 §4.4.1 `data_identifier` byte and
    ///   65459 when the caller's first payload byte is already in
    ///   `0x10..=0x1F`).
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_subtitle_to(
        &mut self,
        handle: SubtitleStreamHandle,
        pts: Pts90khz,
        payload: &[u8],
    ) -> Result<(), MuxError> {
        let (prog_idx, within_idx) = handle.unpack();
        if prog_idx >= self.subtitle_streams.len()
            || within_idx >= self.subtitle_streams[prog_idx].len()
        {
            return Err(MuxError::InvalidStreamHandle {
                kind: StreamKind::Subtitle,
                index: handle.0 as usize,
            });
        }

        // Resolve codec-specific PES envelope shape. DVB-sub auto-wraps the
        // caller's segments in EN 300 743 §6.2's PES_data_field envelope
        // (data_identifier=0x20 + subtitle_stream_id=0x00 + segments + 0xFF
        // marker), adding 3 bytes of overhead. Other codecs pass through.
        let pes_shape = match &self.subtitle_streams[prog_idx][within_idx].codec {
            SubtitleCodec::DvbSubtitling { .. } => SubtitlePesShape::DvbSub,
            SubtitleCodec::DvbTeletext { .. } => SubtitlePesShape::DvbTeletext,
            SubtitleCodec::Cea708Standalone | SubtitleCodec::WebVttInTs => {
                SubtitlePesShape::Passthrough
            }
        };
        let envelope_overhead = match pes_shape {
            SubtitlePesShape::DvbSub => 3, // 0x20 + 0x00 + 0xFF
            // DVB-teletext writes its own 45-byte PES header per EN 300 472 §4.2
            // (rather than reusing the shared 14-byte header path), so it does
            // not contribute envelope bytes — its overhead is folded into
            // `pes_overhead` below.
            SubtitlePesShape::DvbTeletext => 0,
            SubtitlePesShape::Passthrough => 0,
        };

        // PES size cap differs by codec shape:
        // - DVB-teletext: writer emits a 45-byte stuffed PES header and pads
        //   the PES to exactly N×184 bytes per EN 300 472 §4.2. The wire
        //   `PES_packet_length` is `N*184 - 6` and must fit in u16, so the
        //   max acceptable payload is whatever fits inside `dvb_teletext_
        //   total_pes_bytes(...) ≤ 65541`. Auto-prepend of `0x10` (when the
        //   caller's first byte is not already in `0x10..=0x1F`) costs 1
        //   byte of headroom: 65458 max with auto-prepend, 65459 without.
        // - Other codecs: standard 14-byte header (3 byte prefix + flags(3) +
        //   PTS(5)), so PES_packet_length covers flags(3) + PTS(5) + envelope
        //   + payload.
        if matches!(pes_shape, SubtitlePesShape::DvbTeletext) {
            let auto_prepend = dvb_teletext_will_auto_prepend(payload);
            if dvb_teletext_total_pes_bytes(payload.len(), auto_prepend).is_none() {
                // Compute the largest payload we WOULD accept for this
                // `auto_prepend` flag so the error reports a useful max.
                // Per EN 300 472 §4.2 + H.222.0 §2.4.3.7: max total_pes_bytes
                // = 356*184 = 65504; subtract HEADER(45) and the auto-prepend
                // byte (if any) to recover max payload.
                let max = 65504 - 45 - usize::from(auto_prepend);
                return Err(MuxError::SubtitleTooLarge {
                    size: payload.len(),
                    max,
                });
            }
        } else {
            let pes_overhead = 3usize + 5 + envelope_overhead;
            let max_subtitle = (u16::MAX as usize) - pes_overhead;
            if payload.len() > max_subtitle {
                return Err(MuxError::SubtitleTooLarge {
                    size: payload.len(),
                    max: max_subtitle,
                });
            }
        }

        let subtitle_pid = self.subtitle_streams[prog_idx][within_idx].pid;

        // The pes_scratch capacity hint is a comment only — the scratch buf
        // reuses whatever allocation it already holds and grows if needed.
        // DVB-teletext tail-stuffing can add up to one TS payload (184 B)
        // beyond header + payload; that growth is handled transparently by Vec.
        self.pes_scratch.clear();
        write_subtitle_pes(&mut self.pes_scratch, pts.as_ticks(), pes_shape, payload);

        let subtitle_packets = ts_packets_for(self.pes_scratch.len());
        let psi_packets = self.psi_packets_due(prog_idx, pts.as_ticks());
        // Validate-1 C3: validate() bans subtitle PIDs as PCR PIDs, so
        // current_pid here will never equal self.pcr_pids[prog_idx] and
        // pcr_only_packets_due reduces to the pure pcr_due predicate.
        // Subtitle pushes are sparse; this is the prototypical case where
        // PCR injection is needed.
        let pcr_only_packets = self.pcr_only_packets_due(prog_idx, pts.as_ticks(), subtitle_pid);

        if self.queue.len() + psi_packets + pcr_only_packets + subtitle_packets
            > self.config.buffer_packets
        {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets as u64,
            });
        }

        self.maybe_emit_psi(prog_idx, pts.as_ticks());
        self.maybe_emit_pcr_only(prog_idx, pts.as_ticks(), subtitle_pid);

        // Subtitles do NOT extend the PCR fallback chain — they are sparse
        // and event-driven, and the validate path rejects them as PCR PIDs
        // outright (SubtitlePidUsedAsPcrPid). The first packet here will
        // never carry PCR.
        let mut cursor = 0;
        let mut first = true;
        while cursor < self.pes_scratch.len() {
            let adaptation = AdaptationField::default();
            let mut pkt = [0u8; 188];
            let payload_start = cursor;
            let result = write_packet(
                &mut pkt,
                subtitle_pid,
                first,
                adaptation,
                &self.pes_scratch[payload_start..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        // Per-stream stats — Ok-path only.
        if let Some(s) = self.per_stream.get_mut(&subtitle_pid) {
            s.items += 1;
            s.bytes += payload.len() as u64;
        }

        Ok(())
    }

    /// All `SubtitleStreamHandle`s for this muxer, in
    /// `(program, within-program)` declaration order. One handle per
    /// `StreamSpec::Subtitle` across all programs.
    pub fn subtitle_handles(&self) -> Vec<SubtitleStreamHandle> {
        self.subtitle_streams
            .iter()
            .enumerate()
            .flat_map(|(p_idx, prog)| {
                (0..prog.len()).map(move |s_idx| SubtitleStreamHandle::pack(p_idx, s_idx))
            })
            .collect()
    }

    /// Subtitle stream handles for the named program, in declaration order.
    ///
    /// Returns `Err(MuxError::ProgramNotFound)` if no program with the given
    /// number exists.
    pub fn subtitle_handles_for_program(
        &self,
        program_number: u16,
    ) -> Result<Vec<SubtitleStreamHandle>, MuxError> {
        let prog_idx = self
            .config
            .programs
            .iter()
            .position(|p| p.program_number == program_number)
            .ok_or(MuxError::ProgramNotFound { program_number })?;
        Ok((0..self.subtitle_streams[prog_idx].len())
            .map(|s_idx| SubtitleStreamHandle::pack(prog_idx, s_idx))
            .collect())
    }
}
