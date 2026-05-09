//! Opaque builder handles for muxer / sender / reconnect configuration.
//!
//! Each builder is a Box<T>. `_open` clones the inner before consuming it,
//! so the caller may free immediately after a successful open.

use crate::error::{TstError, set_last_error};
use crate::handle::{TST_INVALID_STREAM_HANDLE, TstKlvStreamHandle, TstVideoStreamHandle};
use std::time::Duration;
use tst_core::error::MuxError;
use tst_core::mpegts::mux::{
    KlvStreamHandle, KlvStreamType, MuxerConfig, MuxerProgramConfig, StreamSpec, VideoCodec,
    VideoStreamHandle,
};
use tst_pipeline::{
    BackoffStrategy, OverflowPolicy, RawSenderConfig, ReconnectPolicy, SenderConfig, TsFramingMode,
};

// ------------------------------------------------------------------
// tst_program_handle_t
// ------------------------------------------------------------------

/// Opaque handle returned by `tst_mux_config_add_program`. Used as an
/// argument to subsequent stream-add and config-set entry points to
/// disambiguate which program's streams or descriptors are being modified.
///
/// The value is a zero-based index into the program list. Programs are
/// numbered in the order they were added. Handles are stable across the
/// config→open boundary — the same index applies after `tst_muxer_open`.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TstProgramHandle(pub u32);

/// Sentinel returned by `tst_mux_config_add_program` on failure (null cfg
/// or other hard error). Check `tst_get_last_error()` for the reason.
pub const TST_INVALID_PROGRAM_HANDLE: TstProgramHandle = TstProgramHandle(u32::MAX);

// ------------------------------------------------------------------
// tst_mux_config_t
// ------------------------------------------------------------------

/// Opaque mux-config builder. Constructed via `tst_mux_config_new`,
/// populated via `tst_mux_config_add_program` + stream-add / descriptor-set
/// entry points, then consumed by `tst_*_open` (which clones the inner).
/// Caller is responsible for calling `tst_mux_config_free`.
///
/// Unlike the pre-multi-program API, the config starts **empty** — no program
/// is pre-created. The caller must call `tst_mux_config_add_program` at
/// least once before opening a muxer or sender.
pub struct TstMuxConfig {
    /// Programs accumulated so far. Each `tst_mux_config_add_program` call
    /// pushes one entry; subsequent stream-add / descriptor-set calls index
    /// into this vec by the returned `TstProgramHandle` ordinal.
    pub(crate) programs: Vec<MuxerProgramConfig>,
    /// Per-call interval overrides forwarded to `MuxerConfig` at build time.
    pub(crate) pcr_interval_ms: Option<u32>,
    pub(crate) psi_interval_ms: Option<u32>,
    pub(crate) buffer_packets: Option<usize>,
}

impl TstMuxConfig {
    /// Finish building and return a validated `MuxerConfig`.
    ///
    /// Assembles a `MuxerConfig` from the accumulated programs and any
    /// interval / buffer overrides. The `programs` vec is cloned so the
    /// config may be opened multiple times (the C API allows `_free` after
    /// `_open`, but tests call `_open` more than once in practice).
    pub(crate) fn build_config(&self) -> Result<MuxerConfig, MuxError> {
        let mut cfg = MuxerConfig {
            programs: self.programs.clone(),
            pcr_interval_ms: 40,
            psi_interval_ms: 100,
            buffer_packets: 10_000,
        };
        if let Some(ms) = self.pcr_interval_ms {
            cfg.pcr_interval_ms = ms;
        }
        if let Some(ms) = self.psi_interval_ms {
            cfg.psi_interval_ms = ms;
        }
        if let Some(n) = self.buffer_packets {
            cfg.buffer_packets = n;
        }
        cfg.validate()?;
        Ok(cfg)
    }
}

/// Create a new, empty mux config. No programs are added — call
/// `tst_mux_config_add_program` before using this config to open a muxer
/// or sender. Returns NULL only on allocation failure (OOM).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_new() -> *mut TstMuxConfig {
    Box::into_raw(Box::new(TstMuxConfig {
        programs: Vec::new(),
        pcr_interval_ms: None,
        psi_interval_ms: None,
        buffer_packets: None,
    }))
}

/// Free a mux config previously returned by `tst_mux_config_new`. No-op on
/// NULL. The config must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_free(p: *mut TstMuxConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

/// Begin a new program in this multiplex. Returns a handle used as the
/// `program` argument to subsequent stream-add and descriptor-set entry
/// points. Programs are numbered in insertion order starting at 0.
///
/// `program_number` is the PAT program_number field (must be > 0 and unique
/// within the config). `pmt_pid` is the PID on which this program's PMT will
/// be carried (must be unique within the config and not collide with any
/// stream PID).
///
/// Returns `TST_INVALID_PROGRAM_HANDLE` and sets last-error on null `cfg`.
/// Validation (duplicate program_number, colliding PMT PID, etc.) is deferred
/// to `tst_muxer_open` / `tst_*_sender_open` time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_program(
    cfg: *mut TstMuxConfig,
    program_number: u16,
    pmt_pid: u16,
) -> TstProgramHandle {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TST_INVALID_PROGRAM_HANDLE;
    };
    cfg.programs.push(MuxerProgramConfig {
        program_number,
        pmt_pid,
        streams: Vec::new(),
        pcr_pid: None,
        program_descriptors: Vec::new(),
        stream_descriptors: Vec::new(),
    });
    TstProgramHandle((cfg.programs.len() - 1) as u32)
}

/// Add a video elementary stream to the specified program and return its
/// handle.
///
/// The returned `tst_video_stream_handle_t` is stable across the config→open
/// boundary and across managed-sender reconnects. Pass it to
/// `tst_muxer_push_video_to` / `tst_mux_sender_send_video_to` /
/// `tst_managed_mux_sender_send_video_to` to fan out to this specific stream.
///
/// Returns `TST_INVALID_STREAM_HANDLE` and sets last-error on: null `cfg`,
/// invalid `program` handle, or per-program stream cap exceeded (>16 streams
/// of any kind per program). Hard validation errors surface at `_open` time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_video_stream(
    cfg: *mut TstMuxConfig,
    program: TstProgramHandle,
    pid: u16,
    codec: TstVideoCodec,
) -> TstVideoStreamHandle {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TST_INVALID_STREAM_HANDLE;
    };
    let prog_idx = program.0 as usize;
    if prog_idx >= cfg.programs.len() {
        set_last_error(TstError::InvalidUsage, "invalid program handle");
        return TST_INVALID_STREAM_HANDLE;
    }
    let prog = &mut cfg.programs[prog_idx];
    // within_idx for video handles is the index among video streams only
    // (Muxer builds video_streams[prog] as a filtered subset of streams).
    let within_idx = prog
        .streams
        .iter()
        .filter(|s| matches!(s, StreamSpec::Video { .. }))
        .count();
    if within_idx >= 16 {
        // VideoStreamHandle::pack() debug_asserts within_index < 16; reject
        // before that fires so the C caller gets a defined error.
        set_last_error(
            TstError::InvalidUsage,
            "per-program video stream cap (16) exceeded",
        );
        return TST_INVALID_STREAM_HANDLE;
    }
    let rust_codec = match codec {
        TstVideoCodec::H264 => VideoCodec::H264,
        TstVideoCodec::H265 => VideoCodec::H265,
    };
    prog.streams.push(StreamSpec::Video {
        pid,
        codec: rust_codec,
    });
    prog.stream_descriptors.push(Vec::new());
    VideoStreamHandle::pack(prog_idx, within_idx).raw()
}

/// Add a KLV elementary stream to the specified program and return its handle.
///
/// `stream_type`: `TST_KLV_STREAM_TYPE_PRIVATE_DATA` (0x06, async — no AU
/// cell wrapping) or `TST_KLV_STREAM_TYPE_SYNCHRONOUS_METADATA` (0x15 —
/// the muxer auto-wraps each push in a 5-byte `Metadata_AU_cell` header
/// per ITU-T H.222.0 V9 § 2.12.4.2 before TS-framing; pass raw KLV LS
/// bytes to `push_klv_to`).
///
/// `carries_pts`: set `true` for synchronous KLV (PTS carried in PES header),
/// `false` for async KLV.
///
/// Returns `TST_INVALID_STREAM_HANDLE` on error (same conditions as
/// `tst_mux_config_add_video_stream`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_klv_stream(
    cfg: *mut TstMuxConfig,
    program: TstProgramHandle,
    pid: u16,
    stream_type: TstKlvStreamType,
    carries_pts: bool,
) -> TstKlvStreamHandle {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TST_INVALID_STREAM_HANDLE;
    };
    let prog_idx = program.0 as usize;
    if prog_idx >= cfg.programs.len() {
        set_last_error(TstError::InvalidUsage, "invalid program handle");
        return TST_INVALID_STREAM_HANDLE;
    }
    let prog = &mut cfg.programs[prog_idx];
    // within_idx for klv handles is the index among klv streams only
    // (Muxer builds klv_streams[prog] as a filtered subset of streams).
    let within_idx = prog
        .streams
        .iter()
        .filter(|s| matches!(s, StreamSpec::Klv { .. }))
        .count();
    if within_idx >= 16 {
        set_last_error(
            TstError::InvalidUsage,
            "per-program klv stream cap (16) exceeded",
        );
        return TST_INVALID_STREAM_HANDLE;
    }
    let rust_stream_type = match stream_type {
        TstKlvStreamType::PrivateData => KlvStreamType::PrivateData,
        TstKlvStreamType::SynchronousMetadata => KlvStreamType::SynchronousMetadata,
    };
    prog.streams.push(StreamSpec::Klv {
        pid,
        stream_type: rust_stream_type,
        carries_pts,
    });
    prog.stream_descriptors.push(Vec::new());
    KlvStreamHandle::pack(prog_idx, within_idx).raw()
}

/// Pin the PCR PID for the specified program. By default the muxer uses the
/// first video stream's PID (or first KLV PID for KLV-only programs).
///
/// Returns 0 on success, or a negative `TST_E_*` code on null pointer or
/// invalid program handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_pcr_pid(
    cfg: *mut TstMuxConfig,
    program: TstProgramHandle,
    pid: u16,
) -> libc::c_int {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    let prog_idx = program.0 as usize;
    if prog_idx >= cfg.programs.len() {
        set_last_error(TstError::InvalidUsage, "invalid program handle");
        return TstError::InvalidUsage as i32;
    }
    cfg.programs[prog_idx].pcr_pid = Some(pid);
    0
}

/// Set the PCR re-emission interval for this mux config (applies to all
/// programs). Default is 40 ms. Must be in range 1..=100.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_pcr_interval_ms(
    p: *mut TstMuxConfig,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.pcr_interval_ms = Some(ms);
    0
}

/// Set the PAT/PMT re-emission interval for this mux config. Default 100 ms.
/// Must be >= 10.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_psi_interval_ms(
    p: *mut TstMuxConfig,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.psi_interval_ms = Some(ms);
    0
}

/// Set the TS-packet output buffer capacity. Default 10000 (~1.88 MB).
/// Must be >= 10.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_buffer_packets(
    p: *mut TstMuxConfig,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.buffer_packets = Some(n);
    0
}

/// Set program-level PMT descriptors for the specified program.
///
/// `tlv_bytes` points to the concatenation of `tlv_count` TLV triples,
/// totalling `tlv_total_len` bytes. Each TLV has the layout:
///   byte 0: tag
///   byte 1: length of body (N)
///   bytes 2..2+N: body
///
/// Calling with `tlv_total_len == 0` or `tlv_count == 0` clears any
/// previously set program descriptors for this program.
///
/// Returns 0 on success or a negative `TST_E_*` code on: null `cfg`,
/// null `tlv_bytes` with non-zero count, invalid program handle, or a
/// malformed TLV byte stream (truncated length or body).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_program_descriptors(
    cfg: *mut TstMuxConfig,
    program: TstProgramHandle,
    tlv_bytes: *const u8,
    tlv_total_len: usize,
    tlv_count: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    let prog_idx = program.0 as usize;
    if prog_idx >= cfg.programs.len() {
        set_last_error(TstError::InvalidUsage, "invalid program handle");
        return TstError::InvalidUsage as i32;
    }
    if tlv_total_len == 0 || tlv_count == 0 {
        cfg.programs[prog_idx].program_descriptors = Vec::new();
        return 0;
    }
    let descs = unsafe {
        match parse_tlv_list(tlv_bytes, tlv_total_len, tlv_count) {
            Ok(d) => d,
            Err(rc) => return rc,
        }
    };
    cfg.programs[prog_idx].program_descriptors = descs;
    0
}

/// Set per-stream PMT descriptors for the specified video stream.
///
/// `video` is a handle previously returned by `tst_mux_config_add_video_stream`.
/// The TLV byte format is the same as `tst_mux_config_set_program_descriptors`.
///
/// Calling with `tlv_total_len == 0` or `tlv_count == 0` clears any
/// previously set stream descriptors for this stream.
///
/// Returns 0 on success or a negative `TST_E_*` code on: null `cfg`,
/// invalid handle, null `tlv_bytes` with non-zero count, or malformed TLV.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_stream_descriptors_for_video(
    cfg: *mut TstMuxConfig,
    video: TstVideoStreamHandle,
    tlv_bytes: *const u8,
    tlv_total_len: usize,
    tlv_count: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    let (prog_idx, video_within_idx) = VideoStreamHandle::from_raw(video).unpack();
    if prog_idx >= cfg.programs.len() {
        set_last_error(
            TstError::InvalidUsage,
            "invalid video stream handle (program out of range)",
        );
        return TstError::InvalidUsage as i32;
    }
    let prog = &mut cfg.programs[prog_idx];
    // video_within_idx is the index among video streams only; find the parallel
    // position in prog.streams (which holds all stream kinds interleaved).
    let stream_idx = match prog
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, StreamSpec::Video { .. }))
        .nth(video_within_idx)
        .map(|(i, _)| i)
    {
        Some(i) => i,
        None => {
            set_last_error(
                TstError::InvalidUsage,
                "invalid video stream handle (stream out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
    };
    let descs = unsafe {
        match parse_tlv_list(tlv_bytes, tlv_total_len, tlv_count) {
            Ok(d) => d,
            Err(rc) => return rc,
        }
    };
    prog.stream_descriptors[stream_idx] = descs;
    0
}

/// Set per-stream PMT descriptors for the specified KLV stream.
///
/// `klv` is a handle previously returned by `tst_mux_config_add_klv_stream`.
/// The TLV byte format is the same as `tst_mux_config_set_program_descriptors`.
///
/// Returns 0 on success or a negative `TST_E_*` code on the same conditions
/// as `tst_mux_config_set_stream_descriptors_for_video`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_set_stream_descriptors_for_klv(
    cfg: *mut TstMuxConfig,
    klv: TstKlvStreamHandle,
    tlv_bytes: *const u8,
    tlv_total_len: usize,
    tlv_count: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { cfg.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    let (prog_idx, klv_within_idx) = KlvStreamHandle::from_raw(klv).unpack();
    if prog_idx >= cfg.programs.len() {
        set_last_error(
            TstError::InvalidUsage,
            "invalid klv stream handle (program out of range)",
        );
        return TstError::InvalidUsage as i32;
    }
    let prog = &mut cfg.programs[prog_idx];
    // klv_within_idx is the index among KLV streams only; find the parallel
    // position in prog.streams (which holds all stream kinds interleaved).
    let stream_idx = match prog
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| matches!(s, StreamSpec::Klv { .. }))
        .nth(klv_within_idx)
        .map(|(i, _)| i)
    {
        Some(i) => i,
        None => {
            set_last_error(
                TstError::InvalidUsage,
                "invalid klv stream handle (stream out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
    };
    let descs = unsafe {
        match parse_tlv_list(tlv_bytes, tlv_total_len, tlv_count) {
            Ok(d) => d,
            Err(rc) => return rc,
        }
    };
    prog.stream_descriptors[stream_idx] = descs;
    0
}

// Internal helper: parse a concatenated TLV byte stream (tag + length + body
// per descriptor) into a Vec<Vec<u8>>. Returns Err(TST_E_*) on malformed input.
//
// # Safety
// Caller must ensure `tlv_bytes` is valid for `tlv_total_len` bytes when
// `tlv_total_len > 0` and `tlv_count > 0`.
unsafe fn parse_tlv_list(
    tlv_bytes: *const u8,
    tlv_total_len: usize,
    tlv_count: usize,
) -> Result<Vec<Vec<u8>>, libc::c_int> {
    if tlv_total_len == 0 || tlv_count == 0 {
        return Ok(Vec::new());
    }
    if tlv_bytes.is_null() {
        set_last_error(
            TstError::InvalidUsage,
            "null tlv_bytes pointer with non-zero count",
        );
        return Err(TstError::InvalidUsage as i32);
    }
    let bytes = unsafe { std::slice::from_raw_parts(tlv_bytes, tlv_total_len) };
    let mut descs = Vec::with_capacity(tlv_count);
    let mut offset = 0usize;
    for _ in 0..tlv_count {
        if offset + 2 > bytes.len() {
            set_last_error(
                TstError::InvalidConfig,
                "TLV byte stream truncated (no tag+length)",
            );
            return Err(TstError::InvalidConfig as i32);
        }
        let body_len = bytes[offset + 1] as usize;
        let total = 2 + body_len;
        if offset + total > bytes.len() {
            set_last_error(
                TstError::InvalidConfig,
                "TLV byte stream truncated (body too short)",
            );
            return Err(TstError::InvalidConfig as i32);
        }
        descs.push(bytes[offset..offset + total].to_vec());
        offset += total;
    }
    Ok(descs)
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstVideoCodec {
    H264 = 0,
    H265 = 1,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstKlvStreamType {
    PrivateData = 0,
    SynchronousMetadata = 1,
}

// ------------------------------------------------------------------
// tst_sender_config_t
// ------------------------------------------------------------------

pub struct TstSenderConfig {
    pub(crate) inner: SenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_new() -> *mut TstSenderConfig {
    Box::into_raw(Box::new(TstSenderConfig {
        inner: SenderConfig::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_free(p: *mut TstSenderConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstTsFramingMode {
    Recover = 0,
    Strict = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_set_framing_mode(
    p: *mut TstSenderConfig,
    mode: TstTsFramingMode,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.framing_mode = match mode {
        TstTsFramingMode::Recover => TsFramingMode::Recover,
        TstTsFramingMode::Strict => TsFramingMode::Strict,
    };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_sender_config_set_max_unsynced_bytes(
    p: *mut TstSenderConfig,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.max_unsynced_bytes = n;
    0
}

// ------------------------------------------------------------------
// tst_raw_sender_config_t (empty today; reserved for future setters)
// ------------------------------------------------------------------

pub struct TstRawSenderConfig {
    pub(crate) inner: RawSenderConfig,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_config_new() -> *mut TstRawSenderConfig {
    Box::into_raw(Box::new(TstRawSenderConfig {
        inner: RawSenderConfig::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_raw_sender_config_free(p: *mut TstRawSenderConfig) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

// ------------------------------------------------------------------
// tst_reconnect_policy_t
// ------------------------------------------------------------------

pub struct TstReconnectPolicy {
    pub(crate) inner: ReconnectPolicy,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_new() -> *mut TstReconnectPolicy {
    Box::into_raw(Box::new(TstReconnectPolicy {
        inner: ReconnectPolicy::default(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_free(p: *mut TstReconnectPolicy) {
    if !p.is_null() {
        unsafe { drop(Box::from_raw(p)) };
    }
}

/// Set max reconnect attempts. `n < 0` means retry forever.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_max_attempts(
    p: *mut TstReconnectPolicy,
    n: i32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.max_attempts = if n < 0 { None } else { Some(n as u32) };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_backoff_constant_ms(
    p: *mut TstReconnectPolicy,
    ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.backoff = BackoffStrategy::Constant(Duration::from_millis(ms as u64));
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_backoff_exponential_ms(
    p: *mut TstReconnectPolicy,
    base_ms: u32,
    max_ms: u32,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.backoff = BackoffStrategy::Exponential {
        base: Duration::from_millis(base_ms as u64),
        max: Duration::from_millis(max_ms as u64),
    };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_gap_buffer_capacity(
    p: *mut TstReconnectPolicy,
    n: usize,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.gap_buffer_capacity = n;
    0
}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum TstOverflowPolicy {
    DropOldest = 0,
    Reject = 1,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_reconnect_policy_set_overflow_policy(
    p: *mut TstReconnectPolicy,
    policy: TstOverflowPolicy,
) -> libc::c_int {
    let Some(cfg) = (unsafe { p.as_mut() }) else {
        set_last_error(TstError::InvalidConfig, "null config pointer");
        return TstError::InvalidConfig as i32;
    };
    cfg.inner.overflow_policy = match policy {
        TstOverflowPolicy::DropOldest => OverflowPolicy::DropOldest,
        TstOverflowPolicy::Reject => OverflowPolicy::Reject,
    };
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mux_config_lifecycle() {
        unsafe {
            let p = tst_mux_config_new();
            assert!(!p.is_null());
            let prog = tst_mux_config_add_program(p, 1, 0x1000);
            assert_ne!(prog, TST_INVALID_PROGRAM_HANDLE);
            assert_eq!(
                tst_mux_config_add_video_stream(p, prog, 0x1011, TstVideoCodec::H264),
                // VideoStreamHandle::pack(0, 0) = 0
                0,
            );
            assert_ne!(
                tst_mux_config_add_klv_stream(
                    p,
                    prog,
                    0x1031,
                    TstKlvStreamType::PrivateData,
                    false
                ),
                TST_INVALID_STREAM_HANDLE,
            );
            assert_eq!(tst_mux_config_set_pcr_interval_ms(p, 30), 0);
            tst_mux_config_free(p);
        }
    }

    #[test]
    fn ts_sender_config_lifecycle() {
        unsafe {
            let p = tst_sender_config_new();
            assert_eq!(
                tst_sender_config_set_framing_mode(p, TstTsFramingMode::Strict),
                0,
            );
            assert_eq!(tst_sender_config_set_max_unsynced_bytes(p, 1024), 0);
            tst_sender_config_free(p);
        }
    }

    #[test]
    fn reconnect_policy_lifecycle() {
        unsafe {
            let p = tst_reconnect_policy_new();
            assert_eq!(tst_reconnect_policy_set_max_attempts(p, -1), 0); // forever
            assert_eq!(
                tst_reconnect_policy_set_backoff_exponential_ms(p, 100, 5_000),
                0,
            );
            assert_eq!(tst_reconnect_policy_set_gap_buffer_capacity(p, 128), 0);
            assert_eq!(
                tst_reconnect_policy_set_overflow_policy(p, TstOverflowPolicy::Reject),
                0,
            );
            tst_reconnect_policy_free(p);
        }
    }

    #[test]
    fn null_pointer_setters_return_invalid_config() {
        unsafe {
            assert_ne!(
                tst_mux_config_add_program(std::ptr::null_mut(), 1, 0x1000),
                TstProgramHandle(0),
            );
        }
    }

    #[test]
    fn add_video_stream_returns_packed_handles() {
        unsafe {
            let p = tst_mux_config_new();
            let prog = tst_mux_config_add_program(p, 1, 0x1000);
            // VideoStreamHandle::pack(0, 0) = (0<<4)|0 = 0
            let h0 = tst_mux_config_add_video_stream(p, prog, 0x1011, TstVideoCodec::H264);
            // VideoStreamHandle::pack(0, 1) = (0<<4)|1 = 1
            let h1 = tst_mux_config_add_video_stream(p, prog, 0x1012, TstVideoCodec::H265);
            assert_eq!(h0, 0);
            assert_eq!(h1, 1);
            tst_mux_config_free(p);
        }
    }

    #[test]
    fn add_klv_stream_returns_packed_handles() {
        unsafe {
            let p = tst_mux_config_new();
            let prog = tst_mux_config_add_program(p, 1, 0x1000);
            // KlvStreamHandle::pack(0, 0) = 0
            let h0 = tst_mux_config_add_klv_stream(
                p,
                prog,
                0x1031,
                TstKlvStreamType::PrivateData,
                false,
            );
            // KlvStreamHandle::pack(0, 1) = 1
            let h1 = tst_mux_config_add_klv_stream(
                p,
                prog,
                0x1032,
                TstKlvStreamType::SynchronousMetadata,
                true,
            );
            assert_eq!(h0, 0);
            assert_eq!(h1, 1);
            tst_mux_config_free(p);
        }
    }

    #[test]
    fn add_video_stream_null_returns_sentinel() {
        use crate::handle::TST_INVALID_STREAM_HANDLE;
        unsafe {
            // Null cfg: no program handle needed
            let h = tst_mux_config_add_video_stream(
                std::ptr::null_mut(),
                TstProgramHandle(0),
                0,
                TstVideoCodec::H264,
            );
            assert_eq!(h, TST_INVALID_STREAM_HANDLE);
        }
    }

    #[test]
    fn add_video_stream_invalid_program_returns_sentinel() {
        unsafe {
            let p = tst_mux_config_new();
            // No programs added yet; any handle is invalid.
            let h = tst_mux_config_add_video_stream(
                p,
                TstProgramHandle(0),
                0x1011,
                TstVideoCodec::H264,
            );
            assert_eq!(h, TST_INVALID_STREAM_HANDLE);
            tst_mux_config_free(p);
        }
    }

    #[test]
    fn multi_program_handles_are_distinct() {
        unsafe {
            let p = tst_mux_config_new();
            let prog1 = tst_mux_config_add_program(p, 1, 0x1000);
            let prog2 = tst_mux_config_add_program(p, 2, 0x1100);
            assert_eq!(prog1, TstProgramHandle(0));
            assert_eq!(prog2, TstProgramHandle(1));

            // VideoStreamHandle::pack(0, 0) = 0, pack(1, 0) = 0x10
            let v1 = tst_mux_config_add_video_stream(p, prog1, 0x1011, TstVideoCodec::H264);
            let v2 = tst_mux_config_add_video_stream(p, prog2, 0x1111, TstVideoCodec::H265);
            assert_ne!(v1, v2);
            assert_ne!(v1, TST_INVALID_STREAM_HANDLE);
            assert_ne!(v2, TST_INVALID_STREAM_HANDLE);
            tst_mux_config_free(p);
        }
    }

    #[test]
    fn add_program_null_returns_sentinel() {
        unsafe {
            let h = tst_mux_config_add_program(std::ptr::null_mut(), 1, 0x1000);
            assert_eq!(h, TST_INVALID_PROGRAM_HANDLE);
        }
    }
}
