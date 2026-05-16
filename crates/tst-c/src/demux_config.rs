//! `tst_demux_config_t` opaque builder + supporting C-ABI enums.
//!
//! Mirrors the `tst_mux_config_t` shape from plan #14: heap-allocated
//! opaque builder, mutating setters returning `i32` codes, `_free`
//! releases. The receiver clones what it needs at `_open_with_config`
//! time; the caller still owns the builder and must `_free` it.

use libc::c_int;

/// `repr(i32)` mirror of `tst_core::mpegts::demux::StrictMode`.
///
/// Four-valued enum: `Off=0` (default, lenient), `TimingOnly=1`,
/// `DescriptorsOnly=2`, `Full=3`. cbindgen emits parallel `#define
/// TST_STRICT_MODE_*` blocks for C callers.
///
/// NOTE: this differs from the receiver-surface design doc §7.1, which
/// originally specified `0=Off, 1=KlvOnly, 2=All`. The actual Rust enum
/// is 4-valued; this mapping is the truth.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TstStrictMode {
    Off = 0,
    TimingOnly = 1,
    DescriptorsOnly = 2,
    Full = 3,
}

impl TstStrictMode {
    pub(crate) fn from_c_int(v: c_int) -> Option<Self> {
        match v {
            0 => Some(Self::Off),
            1 => Some(Self::TimingOnly),
            2 => Some(Self::DescriptorsOnly),
            3 => Some(Self::Full),
            _ => None,
        }
    }

    pub(crate) fn to_rust(self) -> tst_core::mpegts::demux::StrictMode {
        use tst_core::mpegts::demux::StrictMode;
        match self {
            Self::Off => StrictMode::Off,
            Self::TimingOnly => StrictMode::TimingOnly,
            Self::DescriptorsOnly => StrictMode::DescriptorsOnly,
            Self::Full => StrictMode::Full,
        }
    }
}

// ------------------------------------------------------------------
// TstDemuxConfig opaque builder
// ------------------------------------------------------------------

use libc::size_t;
use std::collections::HashMap;
use tst_core::mpegts::demux::{DemuxerOptions, StrictMode, StreamKind};

/// Opaque demux-config builder. Heap-allocated via `_new`, mutated
/// in place via setters, released via `_free`. The receiver clones
/// what it needs at `_open_with_config` time; the caller still owns
/// the builder.
///
/// Lifecycle mirrors `tst_mux_config_t` from plan #14 exactly.
pub struct TstDemuxConfig {
    strict: StrictMode,
    pes_cap_per_pid: Option<usize>,
    pes_cap_total: Option<usize>,
    klv_link_overrides: Vec<(u16, u16)>,
    stream_kind_overrides: HashMap<u16, StreamKind>,
}

impl TstDemuxConfig {
    #[allow(dead_code)] // used in Task 10
    pub(crate) fn build_options(&self) -> DemuxerOptions {
        DemuxerOptions {
            strict: self.strict,
            pes_cap_per_pid: self.pes_cap_per_pid,
            pes_cap_total: self.pes_cap_total,
            klv_link_overrides: self.klv_link_overrides.clone(),
            stream_kind_overrides: self.stream_kind_overrides.clone(),
            lenient_psi_reassembly: false,
        }
    }
}

/// Allocate a new `tst_demux_config_t` with default values
/// (strict mode = Off, no overrides, default PES caps).
///
/// Returns `NULL` on allocation failure or internal panic.
/// Free with `tst_demux_config_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_new() -> *mut TstDemuxConfig {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(TstDemuxConfig {
            strict: StrictMode::Off,
            pes_cap_per_pid: None,
            pes_cap_total: None,
            klv_link_overrides: Vec::new(),
            stream_kind_overrides: HashMap::new(),
        }))
    })
}

/// Release a `tst_demux_config_t`. Safe to call with NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_free(cfg: *mut TstDemuxConfig) {
    if cfg.is_null() {
        return;
    }
    drop(unsafe { Box::from_raw(cfg) });
}

/// Set the demuxer's strict mode. `mode` is one of
/// `TST_STRICT_MODE_OFF` (0, default), `TST_STRICT_MODE_TIMING_ONLY` (1),
/// `TST_STRICT_MODE_DESCRIPTORS_ONLY` (2), or `TST_STRICT_MODE_FULL` (3).
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on null `cfg` or
/// unrecognized `mode`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_set_strict_mode(
    cfg: *mut TstDemuxConfig,
    mode: c_int,
) -> c_int {
    crate::panic::ffi_catch(crate::error::TstError::PanicCaught as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            crate::error::set_last_error(
                crate::error::TstError::InvalidConfig,
                "null config pointer",
            );
            return crate::error::TstError::InvalidConfig as i32;
        };
        let Some(parsed) = TstStrictMode::from_c_int(mode) else {
            crate::error::set_last_error(
                crate::error::TstError::InvalidConfig,
                "unrecognized strict mode (valid: 0..=3)",
            );
            return crate::error::TstError::InvalidConfig as i32;
        };
        cfg.strict = parsed.to_rust();
        0
    })
}

/// Add a `klv_pid` → `video_pid` KLV-link override. Bypasses PMT-descriptor
/// inference. Returns 0 on success, `TST_E_INVALID_CONFIG` on null `cfg`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_add_link_klv(
    cfg: *mut TstDemuxConfig,
    klv_pid: u16,
    video_pid: u16,
) -> c_int {
    crate::panic::ffi_catch(crate::error::TstError::PanicCaught as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            crate::error::set_last_error(
                crate::error::TstError::InvalidConfig,
                "null config pointer",
            );
            return crate::error::TstError::InvalidConfig as i32;
        };
        cfg.klv_link_overrides.push((klv_pid, video_pid));
        0
    })
}

/// Force a PID's stream-kind classification. `stream_kind` is one of
/// `TST_STREAM_KIND_VIDEO_H264`, `TST_STREAM_KIND_VIDEO_H265`,
/// `TST_STREAM_KIND_AUDIO_MP2`, ... — see the header for the full table.
///
/// NOTE: The C-side mapping flattens (TstStreamKindTag × codec) pairs
/// into single integers. The implementation table below covers the
/// common cases — extend if a consumer asks for a kind not listed.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on null `cfg` or
/// unrecognized `stream_kind`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_add_treat_as(
    cfg: *mut TstDemuxConfig,
    pid: u16,
    stream_kind: c_int,
) -> c_int {
    crate::panic::ffi_catch(crate::error::TstError::PanicCaught as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            crate::error::set_last_error(
                crate::error::TstError::InvalidConfig,
                "null config pointer",
            );
            return crate::error::TstError::InvalidConfig as i32;
        };
        use tst_core::mpegts::demux::{AudioCodec, StreamKind, SubtitleCodec, VideoCodec};
        // Flat mapping: 100..=103 = Video H264/H265/H266/AV1;
        //               200..=203 = Audio MP2/AAC/AacLatm/AC3;
        //               300..=303 = Subtitle DvbSub/DvbTeletext/Cea708/WebVtt;
        //               400 = KlvSync (declared_link=None);
        //               401 = KlvAsync.
        let kind = match stream_kind {
            100 => StreamKind::Video(VideoCodec::H264),
            101 => StreamKind::Video(VideoCodec::H265),
            102 => StreamKind::Video(VideoCodec::H266),
            103 => StreamKind::Video(VideoCodec::Av1),
            200 => StreamKind::Audio(AudioCodec::Mp2),
            201 => StreamKind::Audio(AudioCodec::Aac),
            202 => StreamKind::Audio(AudioCodec::AacLatm),
            203 => StreamKind::Audio(AudioCodec::Ac3),
            300 => StreamKind::Subtitle(SubtitleCodec::DvbSubtitling),
            301 => StreamKind::Subtitle(SubtitleCodec::DvbTeletext),
            302 => StreamKind::Subtitle(SubtitleCodec::Cea708Standalone),
            303 => StreamKind::Subtitle(SubtitleCodec::WebVttInTs),
            400 => StreamKind::KlvSync { declared_link: None },
            401 => StreamKind::KlvAsync,
            _ => {
                crate::error::set_last_error(
                    crate::error::TstError::InvalidConfig,
                    "unrecognized stream_kind integer",
                );
                return crate::error::TstError::InvalidConfig as i32;
            }
        };
        cfg.stream_kind_overrides.insert(pid, kind);
        0
    })
}

/// Set PES reassembly caps. `0` means use the Rust-side default.
///
/// Returns 0 on success, `TST_E_INVALID_CONFIG` on null `cfg`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_demux_config_set_pes_cap(
    cfg: *mut TstDemuxConfig,
    per_pid: size_t,
    total: size_t,
) -> c_int {
    crate::panic::ffi_catch(crate::error::TstError::PanicCaught as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            crate::error::set_last_error(
                crate::error::TstError::InvalidConfig,
                "null config pointer",
            );
            return crate::error::TstError::InvalidConfig as i32;
        };
        cfg.pes_cap_per_pid = if per_pid == 0 { None } else { Some(per_pid) };
        cfg.pes_cap_total = if total == 0 { None } else { Some(total) };
        0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_mode_round_trip() {
        for v in 0..=3 {
            let m = TstStrictMode::from_c_int(v).expect("recognized");
            assert_eq!(m as i32, v);
        }
        assert!(TstStrictMode::from_c_int(-1).is_none());
        assert!(TstStrictMode::from_c_int(4).is_none());
    }

    #[test]
    fn new_then_free_smoke() {
        unsafe {
            let cfg = tst_demux_config_new();
            assert!(!cfg.is_null());
            tst_demux_config_free(cfg);
        }
    }

    #[test]
    fn double_free_safe() {
        unsafe {
            let cfg = tst_demux_config_new();
            tst_demux_config_free(cfg);
            // Calling _free again on the same pointer is documented as
            // undefined behavior; this test only validates _free(NULL).
            tst_demux_config_free(std::ptr::null_mut());
        }
    }

    #[test]
    fn set_strict_mode_null_cfg_returns_invalid_config() {
        let rc = unsafe { tst_demux_config_set_strict_mode(std::ptr::null_mut(), 0) };
        assert_eq!(rc, crate::error::TstError::InvalidConfig as i32);
    }

    #[test]
    fn set_strict_mode_invalid_value_returns_invalid_config() {
        unsafe {
            let cfg = tst_demux_config_new();
            let rc = tst_demux_config_set_strict_mode(cfg, 999);
            assert_eq!(rc, crate::error::TstError::InvalidConfig as i32);
            tst_demux_config_free(cfg);
        }
    }

    #[test]
    fn add_link_klv_null_cfg_returns_invalid_config() {
        let rc = unsafe { tst_demux_config_add_link_klv(std::ptr::null_mut(), 0x101, 0x102) };
        assert_eq!(rc, crate::error::TstError::InvalidConfig as i32);
    }

    #[test]
    fn add_treat_as_unrecognized_kind_returns_invalid_config() {
        unsafe {
            let cfg = tst_demux_config_new();
            let rc = tst_demux_config_add_treat_as(cfg, 0x101, 9999);
            assert_eq!(rc, crate::error::TstError::InvalidConfig as i32);
            tst_demux_config_free(cfg);
        }
    }

    #[test]
    fn set_pes_cap_null_cfg_returns_invalid_config() {
        let rc = unsafe { tst_demux_config_set_pes_cap(std::ptr::null_mut(), 1024, 65536) };
        assert_eq!(rc, crate::error::TstError::InvalidConfig as i32);
    }
}
