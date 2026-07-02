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
use alloc::vec::Vec;

use super::Muxer;
use super::pes::{
    MAX_PES_HEADER_SIZE, PesFlags, PesPtsField, STREAM_ID_PRIVATE_STREAM_1, STREAM_ID_VIDEO,
    write_pes_header,
};
use super::state::{ts_packets_for, validate_annex_b, wrap_av1_obus_binding};
use super::ts::{AdaptationField, write_packet};
use super::types::{Av1CarriageMode, StreamKind, VideoCodec, VideoStreamHandle};

/// Whether the caller-supplied bytes are elementary (need binding
/// framing in `Mpeg2TsBinding` mode) or already in on-wire carriage form
/// (emitted verbatim — used by the wire push for byte-faithful re-mux).
enum VideoInputForm {
    Elementary,
    Wire,
}

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
    /// `tst_muxer_push_video` — see `bindings/c/include/tstrans.h`.
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
    /// The emitted PES carries `PTS_DTS_flags = '10'` (PTS only) per
    /// ISO/IEC 13818-1 §2.4.3.6. Streams with B-frame reorder where
    /// `composition_time != decode_time` must instead use
    /// [`Self::push_video_to_with_dts`] which emits `PTS_DTS_flags = '11'`.
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video_to` — see `bindings/c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's configured video stream count.
    /// - [`MuxError::InvalidNal`] if `nal` does not begin with an Annex-B
    ///   start code or is not structurally composed of start-code-delimited
    ///   NAL units (only checked for H.264 / H.265 / H.266; AV1 OBU
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
        self.push_video_to_internal(
            handle,
            nal,
            PesPtsField::PtsOnly(pts),
            pts,
            key_frame,
            VideoInputForm::Elementary,
        )
    }

    /// Push one access unit with explicit composition (PTS) and decode (DTS)
    /// timestamps. Required for codecs that emit reordered output (B-frames
    /// in H.264 / H.265 / H.266 / AV1).
    ///
    /// Emits PES with `PTS_DTS_flags = '11'` per ISO/IEC 13818-1 §2.4.3.6 —
    /// 10 bytes of PES header data carrying both PTS (composition time)
    /// and DTS (decode time). When `pts == dts`, prefer
    /// [`Self::push_video_to`] for the smaller 5-byte PTS-only encoding.
    ///
    /// **Caller invariant:** `dts <= pts` per §2.4.3.6 (decode order precedes
    /// composition order). The muxer does not enforce this — receivers
    /// will reject inverted timestamps.
    ///
    /// **Internal cadence:** PCR pacing, PSI emission cadence, and buffer
    /// reservation all key off `pts`. DTS does not influence the
    /// wall-clock or scheduling state.
    ///
    /// AV1 streams expect OBU bitstream input (AV1 spec §5) and skip
    /// the Annex-B start-code check; H.264 / H.265 / H.266 require
    /// Annex-B framing.
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video_to_with_dts` — see `bindings/c/include/tstrans.h`.
    /// DTS push is handle-targeted only; there is no single-stream C
    /// shorthand (resolve the lone stream's handle from
    /// `tst_mux_config_add_video_stream`).
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of
    ///   range for this muxer's configured video stream count.
    /// - [`MuxError::InvalidNal`] if `nal` does not begin with an Annex-B
    ///   start code or is not structurally composed of start-code-delimited
    ///   NAL units (only checked for H.264 / H.265 / H.266; AV1 OBU
    ///   payloads pass through this check).
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    pub fn push_video_to_with_dts(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts: Pts90khz,
        dts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        // Pacing keys off PTS — see method-level rustdoc.
        self.push_video_to_internal(
            handle,
            nal,
            PesPtsField::PtsAndDts { pts, dts },
            pts,
            key_frame,
            VideoInputForm::Elementary,
        )
    }

    /// Push one video access unit that is ALREADY in the muxer's configured
    /// on-wire carriage form — emitted verbatim as the PES payload with no
    /// framing transformation and no Annex-B validation.
    ///
    /// This is the byte-faithful re-mux counterpart to demuxed
    /// [`SamplePayload::Video::raw`](crate::mpegts::demux::SamplePayload):
    /// configure this muxer's carriage to the sample's `av1_carriage`
    /// provenance and feed `raw` straight back here. For AV1 in
    /// `Mpeg2TsBinding` mode the input is expected to already be
    /// `ts_open_bitstream_unit()`-framed (the demuxer's raw payload);
    /// [`Self::push_video_to`] would re-wrap it and corrupt it (AV1-01).
    ///
    /// For elementary OBU / Annex-B input use [`Self::push_video_to`].
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video_wire_to` — see `bindings/c/include/tstrans.h`.
    /// The single-stream shorthand is `tst_muxer_push_video_wire`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of range
    ///   for this muxer's configured video stream count.
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    ///
    /// `InvalidNal` and `InvalidAv1Obu` are never raised on this path — the
    /// bytes are emitted verbatim with no Annex-B/OBU validation or framing.
    pub fn push_video_wire_to(
        &mut self,
        handle: VideoStreamHandle,
        wire: &[u8],
        pts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        self.push_video_to_internal(
            handle,
            wire,
            PesPtsField::PtsOnly(pts),
            pts,
            key_frame,
            VideoInputForm::Wire,
        )
    }

    /// PTS+DTS variant of [`Self::push_video_wire_to`] for reordered streams.
    ///
    /// # C ABI
    ///
    /// `tst_muxer_push_video_wire_to_with_dts` — see
    /// `bindings/c/include/tstrans.h`.
    ///
    /// # Errors
    /// - [`MuxError::InvalidStreamHandle`] if `handle`'s index is out of range
    ///   for this muxer's configured video stream count.
    /// - [`MuxError::BufferFull`] if the resulting TS packets would exceed
    ///   `MuxerConfig::buffer_packets`.
    ///
    /// `InvalidNal` and `InvalidAv1Obu` are never raised on this path — the
    /// bytes are emitted verbatim with no Annex-B/OBU validation or framing.
    pub fn push_video_wire_to_with_dts(
        &mut self,
        handle: VideoStreamHandle,
        wire: &[u8],
        pts: Pts90khz,
        dts: Pts90khz,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        self.push_video_to_internal(
            handle,
            wire,
            PesPtsField::PtsAndDts { pts, dts },
            pts,
            key_frame,
            VideoInputForm::Wire,
        )
    }

    /// Shared body for `push_video_to`, `push_video_to_with_dts`,
    /// `push_video_wire_to`, and `push_video_wire_to_with_dts`.
    ///
    /// `pts_field` controls the PES header shape (PTS-only vs PTS+DTS);
    /// `pacing_pts` is the timestamp used for PCR / PSI cadence and stats
    /// (always the presentation time — DTS doesn't drive scheduling).
    /// `form` selects whether `nal` is an elementary AU that may need
    /// binding-mode framing (`Elementary`) or is already in on-wire form
    /// and must be emitted verbatim (`Wire`).
    fn push_video_to_internal(
        &mut self,
        handle: VideoStreamHandle,
        nal: &[u8],
        pts_field: PesPtsField,
        pacing_pts: Pts90khz,
        key_frame: bool,
        form: VideoInputForm,
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
        let codec = self.video_streams[prog_idx][within_idx].codec;
        // AV1 carries OBUs (AV1 spec §5), not Annex-B NAL units — its push
        // payload is the OBU bitstream and must skip the Annex-B start-code
        // check that H.264 / H.265 / H.266 require. Wire-form pushes also
        // skip validation — the bytes are already on-wire payloads, not
        // elementary bitstream.
        if matches!(form, VideoInputForm::Elementary) && !matches!(codec, VideoCodec::Av1) {
            validate_annex_b(nal)?;
        }

        // AV1-in-MPEG-2-TS binding (`av1-mpeg2-ts-binding.html`) carriage:
        // - §3.4 `data_alignment_indicator=1` — REQUIRED for AV1, set below.
        // - §3.4 `stream_id` — `0xBD` (private_stream_1) in
        //   `Mpeg2TsBinding` mode; `0xE0` (video) in `InteropRawObu`.
        // - §3.2 `ts_open_bitstream_unit()` framing — applied in
        //   `Mpeg2TsBinding` mode via [`wrap_av1_obus_binding`]; raw OBUs
        //   in `InteropRawObu` mode.
        // H.222.0 §2.4.3.7 leaves the alignment bit codec-defined for
        // H.264 / H.265 / H.266 — keep them unset.
        //
        // `av1_binding` keys off the CARRIAGE configuration, not the input
        // form — a Wire-form binding-AV1 push still emits stream_id 0xBD.
        let av1_binding = matches!(codec, VideoCodec::Av1)
            && matches!(self.config.av1_carriage, Av1CarriageMode::Mpeg2TsBinding);
        let stream_id = if av1_binding {
            STREAM_ID_PRIVATE_STREAM_1
        } else {
            STREAM_ID_VIDEO
        };
        let pes_flags = PesFlags {
            data_alignment_indicator: matches!(codec, VideoCodec::Av1),
        };
        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(&mut header, stream_id, pts_field, None, pes_flags);
        let pts = pacing_pts;

        // Bytes that will land in the PES payload (after the PES header).
        // In `Mpeg2TsBinding` mode for AV1 elementary input, the raw OBUs
        // are wrapped in `ts_open_bitstream_unit()` framing here so the
        // payload size accounting below (ts_packets_for) sees the final
        // on-wire length. Wire-form input is already framed and passes
        // through verbatim — no re-wrap.
        let do_wrap = av1_binding && matches!(form, VideoInputForm::Elementary);
        let wrapped_scratch: Vec<u8> = if do_wrap {
            // Reserve an upper bound: 3-byte start code + body + worst-case
            // ~1.5x for emulation-prevention escapes. The wrap function
            // grows the Vec as needed; this preallocation just avoids
            // a couple of reallocations on typical input.
            let mut v = Vec::with_capacity(3 + nal.len() + (nal.len() >> 1));
            let wrap = wrap_av1_obus_binding(nal, &mut v);
            // A non-empty input that does not fully consume is NOT a valid
            // elementary OBU stream (most commonly already-carried wire bytes
            // mistakenly sent to the wrapping push). Never emit a successful
            // empty/partial AU.
            if !nal.is_empty() && !wrap.fully_consumed {
                return Err(MuxError::InvalidAv1Obu);
            }
            v
        } else {
            Vec::new()
        };
        let payload_bytes: &[u8] = if do_wrap { &wrapped_scratch } else { nal };

        let total = header_len + payload_bytes.len();
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
        self.pes_scratch.extend_from_slice(payload_bytes);

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
        // Stats track *caller-supplied* AU bytes — binding-mode framing
        // overhead is wire-level and not counted here.
        if let Some(s) = self.per_stream.get_mut(&video_pid) {
            s.items += 1;
            s.bytes += nal.len() as u64;
        }

        // Codec-counter bump. Codec already bound above; key_frame is a
        // clean caller-supplied signal for random-access (no NAL inspection
        // required). NAL-unit count walks the caller-supplied AU bytes
        // regardless of carriage mode.
        let nals_count = crate::codec::util::count_nal_units(nal, codec);
        let ra_count = u64::from(key_frame);
        self.bump_video_counters(video_pid, nals_count, ra_count);

        Ok(())
    }
}
