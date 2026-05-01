//! Sender-side MPEG-TS muxer.
//!
//! See `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md` for the full
//! design. The public surface is `Muxer`, `Config`, `VideoCodec`,
//! `KlvStreamType`. Internal helpers live in `ts`, `psi`, `pes` submodules.

pub(crate) mod pes;
pub(crate) mod psi;
pub(crate) mod ts;

use crate::error::MuxError;
use crate::mpegts::common::pid;

/// Video codec carried by the muxer's video PID.
///
/// Drives the PMT `stream_type` byte: 0x1B for H.264 / AVC,
/// 0x24 for H.265 / HEVC. v0 supports both; mid-stream codec change is
/// out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
}

/// Transport-stream type for the KLV PID.
///
/// `PrivateData` (PMT stream_type 0x06) is the broadly-recognized form;
/// `SynchronousMetadata` (0x15) is strict ST 1402 sync.
///
/// Whether the KLV PES carries a PTS is controlled separately via
/// `Config::klv_carries_pts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KlvStreamType {
    PrivateData,
    SynchronousMetadata,
}

/// Muxer construction parameters.
///
/// All defaults match the dominant pattern measured in real STANAG 4609
/// captures (see the design doc's corpus measurement). Override with field
/// updates on a `Config::default()` value.
#[derive(Debug, Clone)]
pub struct Config {
    /// PID for the video PES stream. Default 0x1011.
    pub video_pid: u16,

    /// Video codec — drives PMT stream_type (0x1B for H.264, 0x24 for H.265).
    pub video_codec: VideoCodec,

    /// PID for the KLV metadata stream. Default 0x1031.
    pub klv_pid: u16,

    /// Transport-stream type for the KLV PID. Default `PrivateData` (0x06).
    pub klv_stream_type: KlvStreamType,

    /// Whether the KLV PES carries a PTS in its header.
    /// `false` (default) = ST 1402 async — no PTS.
    /// `true` = sync KLV (PTS aligns with video).
    /// Combination `SynchronousMetadata` + `false` is invalid.
    pub klv_carries_pts: bool,

    /// PID carrying the PCR. `None` (default) = use `video_pid`.
    pub pcr_pid: Option<u16>,

    /// PCR re-emission interval, in milliseconds. Default 40.
    /// Validation: 1..=100 (spec ceiling).
    pub pcr_interval_ms: u32,

    /// PAT/PMT re-emission interval, in milliseconds. Default 100.
    /// Validation: >= 10.
    pub psi_interval_ms: u32,

    /// Maximum buffered TS packets before push returns `BufferFull`.
    /// Default 10000 (~1.88 MB, ~600 ms at 25 Mbps). Validation: >= 10.
    pub buffer_packets: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            video_pid: 0x1011,
            video_codec: VideoCodec::H264,
            klv_pid: 0x1031,
            klv_stream_type: KlvStreamType::PrivateData,
            klv_carries_pts: false,
            pcr_pid: None,
            pcr_interval_ms: 40,
            psi_interval_ms: 100,
            buffer_packets: 10_000,
        }
    }
}

impl Config {
    /// Validate the configuration. Returns `Err(MuxError::InvalidConfig)`
    /// with a static message describing the failed rule.
    pub fn validate(&self) -> Result<(), MuxError> {
        if !pid::is_user_pid(self.video_pid) {
            return Err(MuxError::InvalidConfig(
                "video_pid must be in 0x0010..=0x1FFE",
            ));
        }
        if !pid::is_user_pid(self.klv_pid) {
            return Err(MuxError::InvalidConfig(
                "klv_pid must be in 0x0010..=0x1FFE",
            ));
        }
        if self.video_pid == self.klv_pid {
            return Err(MuxError::InvalidConfig("video_pid and klv_pid must differ"));
        }
        if let Some(pcr) = self.pcr_pid {
            if pcr != self.video_pid && pcr != self.klv_pid {
                return Err(MuxError::InvalidConfig(
                    "pcr_pid must equal video_pid or klv_pid",
                ));
            }
        }
        if !(1..=100).contains(&self.pcr_interval_ms) {
            return Err(MuxError::InvalidConfig(
                "pcr_interval_ms must be in 1..=100",
            ));
        }
        if self.psi_interval_ms < 10 {
            return Err(MuxError::InvalidConfig("psi_interval_ms must be >= 10"));
        }
        if self.buffer_packets < 10 {
            return Err(MuxError::InvalidConfig("buffer_packets must be >= 10"));
        }
        if self.klv_stream_type == KlvStreamType::SynchronousMetadata && !self.klv_carries_pts {
            return Err(MuxError::InvalidConfig(
                "klv_stream_type=SynchronousMetadata requires klv_carries_pts=true",
            ));
        }
        Ok(())
    }

    /// Resolve the PCR PID, defaulting to `video_pid` when `pcr_pid` is None.
    pub(crate) fn resolved_pcr_pid(&self) -> u16 {
        self.pcr_pid.unwrap_or(self.video_pid)
    }
}

use crate::mpegts::common::{Pcr27mhz, Pts90khz, StreamType};
use std::collections::VecDeque;

use self::pes::{
    MAX_PES_HEADER_SIZE, PesPtsField, STREAM_ID_KLV, STREAM_ID_VIDEO, write_pes_header,
};
use self::psi::{KLVA_REGISTRATION_DESCRIPTOR, PmtStreamEntry, write_pat_packet, write_pmt_packet};
use self::ts::{AdaptationField, ContinuityCounters, write_packet};

/// Sender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
///
/// See the design doc for full semantics:
/// `docs/specs/2026-05-01-srt-core-mpegts-mux-design.md`.
pub struct Muxer {
    config: Config,
    pcr_pid: u16,
    pcr_interval_27mhz: u64,
    psi_interval_90khz: i64,

    queue: VecDeque<[u8; 188]>,
    counters: ContinuityCounters,

    /// Last PSI emission PTS, masked to 33 bits (0..2^33). None until first.
    last_psi_emission_pts: Option<u64>,
    /// 27 MHz PCR value at the most recent PCR emission. None until first.
    last_pcr_emission_27mhz: Option<u64>,
}

impl Muxer {
    /// Construct and validate.
    pub fn new(config: Config) -> Result<Self, MuxError> {
        config.validate()?;
        let pcr_pid = config.resolved_pcr_pid();
        let pcr_interval_27mhz = (config.pcr_interval_ms as u64) * 27_000;
        let psi_interval_90khz = (config.psi_interval_ms as i64) * 90;
        Ok(Self {
            config,
            pcr_pid,
            pcr_interval_27mhz,
            psi_interval_90khz,
            queue: VecDeque::with_capacity(64),
            counters: ContinuityCounters::new(),
            last_psi_emission_pts: None,
            last_pcr_emission_27mhz: None,
        })
    }

    /// Push one H.264 / H.265 access unit in Annex-B framing.
    ///
    /// `key_frame=true` causes the first TS packet of the resulting PES to
    /// carry an adaptation field with `random_access_indicator` set.
    ///
    /// Returns `Err(MuxError::InvalidNal)` if `nal` doesn't begin with an
    /// Annex-B start code.
    /// Returns `Err(MuxError::BufferFull)` if the resulting TS packets would
    /// exceed `Config::buffer_packets`. State is unchanged in either error
    /// case.
    pub fn push_video(
        &mut self,
        nal: &[u8],
        pts_90khz: i64,
        key_frame: bool,
    ) -> Result<(), MuxError> {
        validate_annex_b(nal)?;

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_VIDEO,
            PesPtsField::PtsOnly(Pts90khz(pts_90khz)),
            None, // unbounded for video
        );

        let total = header_len + nal.len();
        let video_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(pts_90khz) { 2 } else { 0 };

        if self.queue.len() + psi_packets + video_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(pts_90khz);

        // Build a Vec for the full PES bytes (header + NAL).
        let mut pes_buf = Vec::with_capacity(total);
        pes_buf.extend_from_slice(&header[..header_len]);
        pes_buf.extend_from_slice(nal);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            if first {
                if key_frame {
                    adaptation.random_access = true;
                }
                if self.pcr_pid == self.config.video_pid && self.pcr_due(pts_90khz) {
                    let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                    adaptation.pcr = Some(pcr);
                    self.last_pcr_emission_27mhz = Some(pcr.0);
                }
            }
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                self.config.video_pid,
                first,
                adaptation,
                &pes_buf[cursor..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        Ok(())
    }

    /// Push one KLV metadata blob.
    ///
    /// `pts_90khz` becomes the PES PTS when `Config::klv_carries_pts` is
    /// true; ignored otherwise.
    /// Returns `Err(MuxError::BufferFull)` like `push_video`.
    pub fn push_klv(&mut self, klv: &[u8], pts_90khz: i64) -> Result<(), MuxError> {
        let pts_field = if self.config.klv_carries_pts {
            PesPtsField::PtsOnly(Pts90khz(pts_90khz))
        } else {
            PesPtsField::None
        };

        // PES_packet_length is u16; subtract flags1+flags2+header_data_length
        // (3 bytes) and the optional PTS (5 bytes if klv_carries_pts).
        let pes_overhead = 3usize + if self.config.klv_carries_pts { 5 } else { 0 };
        let max_klv = (u16::MAX as usize) - pes_overhead;
        if klv.len() > max_klv {
            return Err(MuxError::KlvTooLarge {
                size: klv.len(),
                max: max_klv,
            });
        }

        let mut header = [0u8; MAX_PES_HEADER_SIZE];
        let header_len = write_pes_header(
            &mut header,
            STREAM_ID_KLV,
            pts_field,
            Some(klv.len() as u16),
        );

        let total = header_len + klv.len();
        let klv_packets = ts_packets_for(total);
        let psi_packets = if self.psi_due(pts_90khz) { 2 } else { 0 };

        if self.queue.len() + psi_packets + klv_packets > self.config.buffer_packets {
            return Err(MuxError::BufferFull {
                capacity_packets: self.config.buffer_packets,
            });
        }

        self.maybe_emit_psi(pts_90khz);

        let mut pes_buf = Vec::with_capacity(total);
        pes_buf.extend_from_slice(&header[..header_len]);
        pes_buf.extend_from_slice(klv);

        let mut cursor = 0;
        let mut first = true;
        while cursor < pes_buf.len() {
            let mut adaptation = AdaptationField::default();
            // PCR rides on KLV PID only if pcr_pid was explicitly set to it.
            if first && self.pcr_pid == self.config.klv_pid && self.pcr_due(pts_90khz) {
                let pcr = Pcr27mhz::from_pts(Pts90khz(pts_90khz));
                adaptation.pcr = Some(pcr);
                self.last_pcr_emission_27mhz = Some(pcr.0);
            }
            let mut pkt = [0u8; 188];
            let result = write_packet(
                &mut pkt,
                self.config.klv_pid,
                first,
                adaptation,
                &pes_buf[cursor..],
                &mut self.counters,
            );
            cursor += result.payload_consumed;
            self.queue.push_back(pkt);
            first = false;
        }

        Ok(())
    }

    /// Drain ready TS packets into `out`.
    ///
    /// Returns the number of bytes written: 0 or a positive multiple of 188.
    /// `Ok(0)` indicates either an empty queue or `out.len() < 188`.
    pub fn pull(&mut self, out: &mut [u8]) -> Result<usize, MuxError> {
        if out.len() < 188 {
            return Ok(0);
        }
        let max_packets = (out.len() / 188).min(self.queue.len());
        for i in 0..max_packets {
            let pkt = self.queue.pop_front().expect("checked count");
            out[i * 188..(i + 1) * 188].copy_from_slice(&pkt);
        }
        Ok(max_packets * 188)
    }

    fn psi_due(&self, pts_90khz: i64) -> bool {
        match self.last_psi_emission_pts {
            None => true,
            Some(last_masked) => {
                let now_masked = Pts90khz(pts_90khz).masked_33bit();
                let delta = crate::mpegts::common::pts_diff_33bit(now_masked, last_masked);
                delta >= self.psi_interval_90khz
            }
        }
    }

    fn pcr_due(&self, pts_90khz: i64) -> bool {
        match self.last_pcr_emission_27mhz {
            None => true,
            Some(last) => {
                // PCR is at 27 MHz; the 33-bit base wraps at 2^33 base ticks.
                // Convert both to 33-bit base and use the same modular helper,
                // then compare in 90 kHz units.
                let now_base_masked = Pts90khz(pts_90khz).masked_33bit();
                let last_base_masked = (last / 300) & ((1u64 << 33) - 1);
                let delta_90khz = crate::mpegts::common::pts_diff_33bit(
                    now_base_masked,
                    last_base_masked,
                );
                let threshold_90khz = (self.pcr_interval_27mhz / 300) as i64;
                delta_90khz >= threshold_90khz
            }
        }
    }

    fn maybe_emit_psi(&mut self, pts_90khz: i64) {
        if !self.psi_due(pts_90khz) {
            return;
        }
        let mut pat = [0u8; 188];
        write_pat_packet(&mut pat, &mut self.counters);
        self.queue.push_back(pat);

        let mut pmt = [0u8; 188];
        let video_st = match self.config.video_codec {
            VideoCodec::H264 => StreamType::H264,
            VideoCodec::H265 => StreamType::H265,
        };
        let klv_st = match self.config.klv_stream_type {
            KlvStreamType::PrivateData => StreamType::KlvPrivate,
            KlvStreamType::SynchronousMetadata => StreamType::KlvSyncMetadata,
        };
        let entries = [
            PmtStreamEntry {
                stream_type: video_st,
                elementary_pid: self.config.video_pid,
                descriptors: &[],
            },
            PmtStreamEntry {
                stream_type: klv_st,
                elementary_pid: self.config.klv_pid,
                descriptors: KLVA_REGISTRATION_DESCRIPTOR,
            },
        ];
        write_pmt_packet(&mut pmt, self.pcr_pid, &entries, &mut self.counters);
        self.queue.push_back(pmt);

        self.last_psi_emission_pts = Some(Pts90khz(pts_90khz).masked_33bit());
    }
}

fn validate_annex_b(nal: &[u8]) -> Result<(), MuxError> {
    if nal.starts_with(&[0x00, 0x00, 0x00, 0x01]) || nal.starts_with(&[0x00, 0x00, 0x01]) {
        Ok(())
    } else {
        Err(MuxError::InvalidNal)
    }
}

/// Number of 188-byte TS packets needed to carry `payload_size` bytes of
/// PES (header + ES). 184 = 188 - 4 byte TS header. Adaptation field eats
/// further capacity but for sizing purposes the worst case is no AF (gives
/// the smallest packet count). The orchestrator may emit one more packet
/// than this if AF stuffing pushes a byte over; we allow a 1-packet slop
/// in the buffer reservation.
fn ts_packets_for(payload_size: usize) -> usize {
    payload_size.div_ceil(184).max(1) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_validates() {
        Config::default().validate().expect("default is valid");
    }

    #[test]
    fn rejects_video_pid_zero() {
        let cfg = Config {
            video_pid: 0x0000,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "video_pid must be in 0x0010..=0x1FFE"
            ))
        ));
    }

    #[test]
    fn rejects_klv_pid_null() {
        let cfg = Config {
            klv_pid: 0x1FFF,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "klv_pid must be in 0x0010..=0x1FFE"
            ))
        ));
    }

    #[test]
    fn rejects_pid_collision() {
        let cfg = Config {
            video_pid: 0x1234,
            klv_pid: 0x1234,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig("video_pid and klv_pid must differ"))
        ));
    }

    #[test]
    fn rejects_unrelated_pcr_pid() {
        let cfg = Config {
            pcr_pid: Some(0x0500),
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "pcr_pid must equal video_pid or klv_pid"
            ))
        ));
    }

    #[test]
    fn accepts_pcr_pid_on_klv() {
        let cfg = Config {
            pcr_pid: Some(0x1031), // = default klv_pid
            ..Default::default()
        };
        cfg.validate().expect("pcr_pid on klv is allowed");
    }

    #[test]
    fn rejects_pcr_interval_zero() {
        let cfg = Config {
            pcr_interval_ms: 0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_pcr_interval_over_100() {
        let cfg = Config {
            pcr_interval_ms: 150,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig(
                "pcr_interval_ms must be in 1..=100"
            ))
        ));
    }

    #[test]
    fn rejects_psi_interval_too_small() {
        let cfg = Config {
            psi_interval_ms: 5,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig("psi_interval_ms must be >= 10"))
        ));
    }

    #[test]
    fn rejects_buffer_too_small() {
        let cfg = Config {
            buffer_packets: 5,
            ..Default::default()
        };
        assert!(matches!(
            cfg.validate(),
            Err(MuxError::InvalidConfig("buffer_packets must be >= 10"))
        ));
    }

    #[test]
    fn rejects_sync_without_pts() {
        let cfg = Config {
            klv_stream_type: KlvStreamType::SynchronousMetadata,
            klv_carries_pts: false,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn accepts_async_with_pts_combo() {
        // 0x06 + PTS — the common-practice "sync KLV everyone recognizes"
        let cfg = Config {
            klv_stream_type: KlvStreamType::PrivateData,
            klv_carries_pts: true,
            ..Default::default()
        };
        cfg.validate().expect("0x06 + PTS is valid");
    }

    #[test]
    fn resolved_pcr_pid_default() {
        let cfg = Config::default();
        assert_eq!(cfg.resolved_pcr_pid(), cfg.video_pid);
    }

    #[test]
    fn resolved_pcr_pid_explicit() {
        let cfg = Config {
            pcr_pid: Some(0x1031),
            ..Default::default()
        };
        assert_eq!(cfg.resolved_pcr_pid(), 0x1031);
    }

    #[test]
    fn muxer_constructs_with_valid_config() {
        let mux = Muxer::new(Config::default());
        assert!(mux.is_ok());
    }

    #[test]
    fn muxer_rejects_invalid_config() {
        let cfg = Config {
            video_pid: 0,
            ..Default::default()
        };
        let res = Muxer::new(cfg);
        assert!(res.is_err());
    }

    #[test]
    fn pull_returns_zero_on_empty_queue() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let mut buf = [0u8; 1316];
        assert_eq!(mux.pull(&mut buf).unwrap(), 0);
    }

    #[test]
    fn pull_returns_zero_on_short_buffer() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x09, 0x10];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = [0u8; 100];
        assert_eq!(mux.pull(&mut buf).unwrap(), 0);
    }

    #[test]
    fn push_video_rejects_non_annex_b() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let bad = [0x12, 0x34, 0x56];
        assert!(matches!(
            mux.push_video(&bad, 0, false),
            Err(MuxError::InvalidNal)
        ));
    }

    #[test]
    fn push_video_accepts_3byte_start_code() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x01, 0x09, 0x10];
        assert!(mux.push_video(&nal, 0, true).is_ok());
    }

    #[test]
    fn first_pull_includes_pat_pmt() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = [0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x99];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = [0u8; 4096];
        let n = mux.pull(&mut buf).unwrap();
        assert!(n >= 188 * 3, "expected at least PAT + PMT + 1 video packet");
        // First packet should be PAT (PID 0)
        let pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(pid, 0x0000);
        // Second packet should be PMT (PID 0x1000 from psi.rs)
        let pid_2 = (((buf[188 + 1] as u16) & 0x1F) << 8) | buf[188 + 2] as u16;
        assert_eq!(pid_2, 0x1000);
    }

    #[test]
    fn buffer_full_returned_when_overcommitted() {
        let cfg = Config {
            buffer_packets: 10,
            ..Default::default()
        };
        let mut mux = Muxer::new(cfg).unwrap();
        // A 50KB IDR is much larger than 10 packets can hold.
        let big_nal = {
            let mut v = vec![0u8; 50_000];
            v[0] = 0;
            v[1] = 0;
            v[2] = 0;
            v[3] = 1;
            v[4] = 0x65; // IDR slice NAL type
            v
        };
        let res = mux.push_video(&big_nal, 0, true);
        assert!(matches!(
            res,
            Err(MuxError::BufferFull {
                capacity_packets: 10
            })
        ));
    }

    #[test]
    fn buffer_full_does_not_modify_state() {
        let cfg = Config {
            buffer_packets: 10,
            ..Default::default()
        };
        let mut mux = Muxer::new(cfg).unwrap();
        let nal = vec![0u8; 50_000];
        let nal = {
            let mut v = nal;
            v[..4].copy_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            v
        };
        let _ = mux.push_video(&nal, 0, true);
        // Queue should be empty (push didn't commit).
        let mut buf = [0u8; 1316];
        assert_eq!(mux.pull(&mut buf).unwrap(), 0);
    }

    #[test]
    fn psi_emission_survives_pts_rollover() {
        // Push a video AU just before 33-bit rollover, then another well past.
        // True modular delta is +9590 ticks (~106ms), greater than psi_interval
        // default of 9000 ticks (100ms), so PSI MUST re-emit. Buggy raw i64
        // subtraction yields a huge negative and wrongly suppresses PSI.
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        let just_before_wrap = (1i64 << 33) - 90;
        let well_past_wrap = 9_500;
        mux.push_video(&nal, just_before_wrap, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf).unwrap() > 0 {}
        mux.push_video(&nal, well_past_wrap, false).unwrap();
        let n = mux.pull(&mut buf).unwrap();
        assert!(n > 0);
        // First packet should be PAT (PID 0x0000) since PSI is due.
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(first_pid, 0x0000, "PSI suppressed across rollover; got first PID 0x{:04X}", first_pid);
    }

    #[test]
    fn psi_not_due_on_backward_pts() {
        // B-frame display-order: PTS may zigzag backward by a few frames. PSI
        // cadence must NOT trigger on a backward step (it would wrongly emit).
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        mux.push_video(&nal, 100_000, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf).unwrap() > 0 {}
        // Now push a backward PTS (display order earlier). Should NOT emit PSI.
        mux.push_video(&nal, 100_000 - 270, false).unwrap(); // -3ms
        let n = mux.pull(&mut buf).unwrap();
        assert!(n > 0);
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(first_pid, 0x1011, "PSI emitted on backward PTS, got first PID 0x{:04X}", first_pid);
    }

    #[test]
    fn psi_due_after_threshold_forward() {
        // Sanity: forward by exactly psi_interval triggers PSI.
        let mut mux = Muxer::new(Config::default()).unwrap();
        let nal = vec![0x00, 0x00, 0x00, 0x01, 0x65, 0x00];
        mux.push_video(&nal, 0, true).unwrap();
        let mut buf = vec![0u8; 188 * 64];
        while mux.pull(&mut buf).unwrap() > 0 {}
        // psi_interval default = 100ms = 9000 ticks at 90kHz.
        mux.push_video(&nal, 9_000, false).unwrap();
        let n = mux.pull(&mut buf).unwrap();
        assert!(n > 0);
        // First packet should be PAT (PID 0x0000) since PSI was due.
        let first_pid = (((buf[1] as u16) & 0x1F) << 8) | buf[2] as u16;
        assert_eq!(first_pid, 0x0000, "expected PAT, got 0x{:04X}", first_pid);
    }

    #[test]
    fn push_klv_rejects_oversized_blob() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        // PES_packet_length is u16; with PTS off, max KLV payload = 65535 - 3 = 65532.
        let too_big = vec![0u8; 65_533];
        let err = mux.push_klv(&too_big, 0).unwrap_err();
        match err {
            MuxError::KlvTooLarge { size, max } => {
                assert_eq!(size, 65_533);
                assert_eq!(max, 65_532);
            }
            other => panic!("expected MuxError::KlvTooLarge, got {:?}", other),
        }
    }

    #[test]
    fn push_klv_accepts_largest_legal_blob() {
        let mut mux = Muxer::new(Config::default()).unwrap();
        // 65532 with no PTS is the spec-imposed ceiling.
        let max_klv = vec![0xAB; 65_532];
        mux.push_klv(&max_klv, 0).expect("max-size KLV must succeed");
    }

    #[test]
    fn push_klv_with_pts_reduces_max() {
        // With klv_carries_pts=true, header_data_length=5, so max payload =
        // 65535 - 3 - 5 = 65527.
        let mut mux = Muxer::new(Config {
            klv_carries_pts: true,
            ..Config::default()
        })
        .unwrap();
        let too_big = vec![0u8; 65_528];
        let err = mux.push_klv(&too_big, 90_000).unwrap_err();
        match err {
            MuxError::KlvTooLarge { size, max } => {
                assert_eq!(size, 65_528);
                assert_eq!(max, 65_527);
            }
            other => panic!("expected MuxError::KlvTooLarge, got {:?}", other),
        }
    }
}
