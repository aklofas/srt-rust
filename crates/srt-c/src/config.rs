//! Opaque builder handles for muxer / sender / reconnect configuration.
//!
//! Each builder is a Box<T>. `_open` clones the inner before consuming it,
//! so the caller may free immediately after a successful open.

use crate::error::{SrtcError, set_last_error};
use srt_core::mpegts::mux::{ConfigBuilder, KlvStreamType, VideoCodec};
use srt_core::pipeline::{
    BackoffStrategy, OverflowPolicy, RawSenderConfig, ReconnectPolicy, TsFramingMode,
    TsSenderConfig,
};
use std::time::Duration;

// ------------------------------------------------------------------
// srtc_mux_config_t
// ------------------------------------------------------------------

/// Opaque mux-config builder. Constructed via `srtc_mux_config_new`,
/// populated with setters, consumed by `srtc_*_open` (which clones the
/// inner). Caller is responsible for calling `srtc_mux_config_free`.
pub struct SrtcMuxConfig {
    pub(crate) builder: ConfigBuilder,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_new() -> *mut SrtcMuxConfig {
    Box::into_raw(Box::new(SrtcMuxConfig {
        builder: ConfigBuilder::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_free(p: *mut SrtcMuxConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_add_video(
    p: *mut SrtcMuxConfig,
    pid: u16,
    codec: SrtcVideoCodec,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let codec = match codec {
        SrtcVideoCodec::H264 => VideoCodec::H264,
        SrtcVideoCodec::H265 => VideoCodec::H265,
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.add_video(pid, codec);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_add_klv(
    p: *mut SrtcMuxConfig,
    pid: u16,
    stream_type: SrtcKlvStreamType,
    carries_pts: bool,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let stream_type = match stream_type {
        SrtcKlvStreamType::PrivateData => KlvStreamType::PrivateData,
        SrtcKlvStreamType::SynchronousMetadata => KlvStreamType::SynchronousMetadata,
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.add_klv(pid, stream_type, carries_pts);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_set_pcr_pid(
    p: *mut SrtcMuxConfig,
    pid: u16,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.pcr_pid(pid);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_set_pcr_interval_ms(
    p: *mut SrtcMuxConfig,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.pcr_interval_ms(ms);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_set_psi_interval_ms(
    p: *mut SrtcMuxConfig,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.psi_interval_ms(ms);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_mux_config_set_buffer_packets(
    p: *mut SrtcMuxConfig,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    let taken = std::mem::take(&mut cfg.builder);
    cfg.builder = taken.buffer_packets(n);
    0
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum SrtcVideoCodec {
    H264 = 0,
    H265 = 1,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum SrtcKlvStreamType {
    PrivateData = 0,
    SynchronousMetadata = 1,
}

// ------------------------------------------------------------------
// srtc_ts_sender_config_t
// ------------------------------------------------------------------

pub struct SrtcTsSenderConfig {
    pub(crate) inner: TsSenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_config_new() -> *mut SrtcTsSenderConfig {
    Box::into_raw(Box::new(SrtcTsSenderConfig {
        inner: TsSenderConfig::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_config_free(p: *mut SrtcTsSenderConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum SrtcTsFramingMode {
    Recover = 0,
    Strict = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_config_set_framing_mode(
    p: *mut SrtcTsSenderConfig,
    mode: SrtcTsFramingMode,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.framing_mode = match mode {
        SrtcTsFramingMode::Recover => TsFramingMode::Recover,
        SrtcTsFramingMode::Strict => TsFramingMode::Strict,
    };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_ts_sender_config_set_max_unsynced_bytes(
    p: *mut SrtcTsSenderConfig,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.max_unsynced_bytes = n;
    0
}

// ------------------------------------------------------------------
// srtc_raw_sender_config_t (empty in v0; reserved for future setters)
// ------------------------------------------------------------------

pub struct SrtcRawSenderConfig {
    #[allow(dead_code)] // used in Task 9 (srtc_raw_sender_t)
    pub(crate) inner: RawSenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_config_new() -> *mut SrtcRawSenderConfig {
    Box::into_raw(Box::new(SrtcRawSenderConfig {
        inner: RawSenderConfig::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_raw_sender_config_free(p: *mut SrtcRawSenderConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

// ------------------------------------------------------------------
// srtc_reconnect_policy_t
// ------------------------------------------------------------------

pub struct SrtcReconnectPolicy {
    pub(crate) inner: ReconnectPolicy,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_new() -> *mut SrtcReconnectPolicy {
    Box::into_raw(Box::new(SrtcReconnectPolicy {
        inner: ReconnectPolicy::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_free(p: *mut SrtcReconnectPolicy) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

/// Set max reconnect attempts. `n < 0` means retry forever.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_set_max_attempts(
    p: *mut SrtcReconnectPolicy,
    n: i32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.max_attempts = if n < 0 { None } else { Some(n as u32) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_set_backoff_constant_ms(
    p: *mut SrtcReconnectPolicy,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.backoff = BackoffStrategy::Constant(Duration::from_millis(ms as u64));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_set_backoff_exponential_ms(
    p: *mut SrtcReconnectPolicy,
    base_ms: u32,
    max_ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.backoff = BackoffStrategy::Exponential {
        base: Duration::from_millis(base_ms as u64),
        max: Duration::from_millis(max_ms as u64),
    };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_set_gap_buffer_capacity(
    p: *mut SrtcReconnectPolicy,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.gap_buffer_capacity = n;
    0
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum SrtcOverflowPolicy {
    DropOldest = 0,
    Reject = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn srtc_reconnect_policy_set_overflow_policy(
    p: *mut SrtcReconnectPolicy,
    policy: SrtcOverflowPolicy,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(SrtcError::InvalidConfig, "null config pointer");
        return SrtcError::InvalidConfig as i32;
    };
    cfg.inner.overflow_policy = match policy {
        SrtcOverflowPolicy::DropOldest => OverflowPolicy::DropOldest,
        SrtcOverflowPolicy::Reject => OverflowPolicy::Reject,
    };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mux_config_lifecycle() {
        unsafe {
            let p = srtc_mux_config_new();
            assert!(!p.is_null());
            assert_eq!(
                srtc_mux_config_add_video(p, 0x1011, SrtcVideoCodec::H264),
                0
            );
            assert_eq!(
                srtc_mux_config_add_klv(p, 0x1031, SrtcKlvStreamType::PrivateData, false),
                0,
            );
            assert_eq!(srtc_mux_config_set_pcr_interval_ms(p, 30), 0);
            srtc_mux_config_free(p);
        }
    }

    #[test]
    fn ts_sender_config_lifecycle() {
        unsafe {
            let p = srtc_ts_sender_config_new();
            assert_eq!(
                srtc_ts_sender_config_set_framing_mode(p, SrtcTsFramingMode::Strict),
                0,
            );
            assert_eq!(srtc_ts_sender_config_set_max_unsynced_bytes(p, 1024), 0);
            srtc_ts_sender_config_free(p);
        }
    }

    #[test]
    fn reconnect_policy_lifecycle() {
        unsafe {
            let p = srtc_reconnect_policy_new();
            assert_eq!(srtc_reconnect_policy_set_max_attempts(p, -1), 0); // forever
            assert_eq!(
                srtc_reconnect_policy_set_backoff_exponential_ms(p, 100, 5_000),
                0,
            );
            assert_eq!(srtc_reconnect_policy_set_gap_buffer_capacity(p, 128), 0);
            assert_eq!(
                srtc_reconnect_policy_set_overflow_policy(p, SrtcOverflowPolicy::Reject),
                0,
            );
            srtc_reconnect_policy_free(p);
        }
    }

    #[test]
    fn null_pointer_setters_return_invalid_config() {
        unsafe {
            assert_eq!(
                srtc_mux_config_add_video(std::ptr::null_mut(), 0, SrtcVideoCodec::H264),
                SrtcError::InvalidConfig as i32,
            );
        }
    }
}
