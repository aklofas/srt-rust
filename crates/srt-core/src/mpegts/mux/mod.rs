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
    #[allow(dead_code)] // Used in Task 8.
    pub(crate) fn resolved_pcr_pid(&self) -> u16 {
        self.pcr_pid.unwrap_or(self.video_pid)
    }
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
}
