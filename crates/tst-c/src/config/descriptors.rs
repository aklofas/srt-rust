//! PMT descriptor configuration entry points.
//!
//! Houses the program- and per-stream descriptor setters/adders. The two
//! private helpers `desc_to_tlv_blob` (single-descriptor copy) and
//! `parse_tlv_list` (multi-descriptor byte-stream parse) live here next to
//! their callers. PMT descriptors flow from the caller's `tst_descriptor_t`
//! values into per-stream `Vec<Vec<u8>>` slots on the inner
//! `MuxerProgramConfig`; the muxer emits them in PMT order at open time.

use super::{TstMuxConfig, TstProgramHandle};
use crate::error::{TstError, set_last_error};
use crate::handle::{
    TstAudioStreamHandle, TstKlvStreamHandle, TstSubtitleStreamHandle, TstVideoStreamHandle,
};
use crate::panic::ffi_catch;
use tst_core::mpegts::mux::{KlvStreamHandle, StreamSpec, VideoStreamHandle};

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
    ffi_catch(TstError::Internal as i32, || {
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
    })
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
    ffi_catch(TstError::Internal as i32, || {
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
    })
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
    ffi_catch(TstError::Internal as i32, || {
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
    })
}

// Internal helper: copy one `TstDescriptor` into a full TLV blob
// `[tag, body_len, body...]` ready to push onto a `stream_descriptors[i]` slot.
//
// Returns `Err(TST_E_*)` on null data with non-zero length or body > 255 bytes.
//
// # Safety
// Caller must validate that `desc` is non-null before calling.
unsafe fn desc_to_tlv_blob(desc: &crate::event::TstDescriptor) -> Result<Vec<u8>, libc::c_int> {
    if desc.data_len > 255 {
        set_last_error(
            TstError::InvalidConfig,
            "descriptor body length exceeds 255 (MPEG-TS descriptor limit)",
        );
        return Err(TstError::InvalidConfig as i32);
    }
    if desc.data.is_null() && desc.data_len != 0 {
        set_last_error(
            TstError::InvalidConfig,
            "descriptor data pointer is null with non-zero data_len",
        );
        return Err(TstError::InvalidConfig as i32);
    }
    let body: &[u8] = if desc.data_len == 0 {
        &[]
    } else {
        // SAFETY: caller validated desc.data non-null and desc.data_len > 0;
        // bytes are copied into the Vec so the caller's buffer can be freed.
        unsafe { std::slice::from_raw_parts(desc.data, desc.data_len) }
    };
    let mut tlv = Vec::with_capacity(2 + body.len());
    tlv.push(desc.tag);
    tlv.push(desc.data_len as u8);
    tlv.extend_from_slice(body);
    Ok(tlv)
}

/// Append one PMT descriptor to a video stream's per-PID descriptor list.
///
/// `stream` is the handle returned by `tst_mux_config_add_video_stream`.
/// `desc` must be non-null with `desc.data` pointing to `desc.data_len`
/// bytes (stripped length — does not include the tag/length header bytes).
/// Bytes are copied; the caller's buffer is not retained after this call.
/// Multiple calls accumulate; descriptors appear in the PMT in add-order.
///
/// Returns 0 on success, or a negative `TST_E_*` code on: null `cfg` or
/// `desc`, stale handle, null `desc.data` with non-zero `desc.data_len`,
/// or `desc.data_len > 255` (MPEG-TS descriptor body limit).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_video_descriptor(
    cfg: *mut TstMuxConfig,
    stream: TstVideoStreamHandle,
    desc: *const crate::event::TstDescriptor,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        let Some(desc) = (unsafe { desc.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null descriptor pointer");
            return TstError::InvalidConfig as i32;
        };
        let tlv = unsafe {
            match desc_to_tlv_blob(desc) {
                Ok(b) => b,
                Err(rc) => return rc,
            }
        };
        // Unpack (prog_idx, within_idx) from the packed handle — same bit layout
        // as VideoStreamHandle::pack: bits 4..7 = program, bits 0..3 = within.
        let prog_idx = (stream >> 4) as usize;
        let video_within_idx = (stream & 0xF) as usize;
        if prog_idx >= cfg.programs.len() {
            set_last_error(
                TstError::InvalidUsage,
                "invalid video stream handle (program out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
        let prog = &mut cfg.programs[prog_idx];
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
        prog.stream_descriptors[stream_idx].push(tlv);
        0
    })
}

/// Append one PMT descriptor to a KLV stream's per-PID descriptor list.
/// Same contract as `tst_mux_config_add_video_descriptor`.
///
/// `stream` is the handle returned by `tst_mux_config_add_klv_stream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_klv_descriptor(
    cfg: *mut TstMuxConfig,
    stream: TstKlvStreamHandle,
    desc: *const crate::event::TstDescriptor,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        let Some(desc) = (unsafe { desc.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null descriptor pointer");
            return TstError::InvalidConfig as i32;
        };
        let tlv = unsafe {
            match desc_to_tlv_blob(desc) {
                Ok(b) => b,
                Err(rc) => return rc,
            }
        };
        let prog_idx = (stream >> 4) as usize;
        let klv_within_idx = (stream & 0xF) as usize;
        if prog_idx >= cfg.programs.len() {
            set_last_error(
                TstError::InvalidUsage,
                "invalid klv stream handle (program out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
        let prog = &mut cfg.programs[prog_idx];
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
        prog.stream_descriptors[stream_idx].push(tlv);
        0
    })
}

/// Append one PMT descriptor to an audio stream's per-PID descriptor list.
/// Same contract as `tst_mux_config_add_video_descriptor`.
///
/// `stream` is the handle returned by `tst_mux_config_add_audio_stream`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_audio_descriptor(
    cfg: *mut TstMuxConfig,
    stream: TstAudioStreamHandle,
    desc: *const crate::event::TstDescriptor,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        let Some(desc) = (unsafe { desc.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null descriptor pointer");
            return TstError::InvalidConfig as i32;
        };
        let tlv = unsafe {
            match desc_to_tlv_blob(desc) {
                Ok(b) => b,
                Err(rc) => return rc,
            }
        };
        let prog_idx = (stream >> 4) as usize;
        let audio_within_idx = (stream & 0xF) as usize;
        if prog_idx >= cfg.programs.len() {
            set_last_error(
                TstError::InvalidUsage,
                "invalid audio stream handle (program out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
        let prog = &mut cfg.programs[prog_idx];
        let stream_idx = match prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Audio { .. }))
            .nth(audio_within_idx)
            .map(|(i, _)| i)
        {
            Some(i) => i,
            None => {
                set_last_error(
                    TstError::InvalidUsage,
                    "invalid audio stream handle (stream out of range)",
                );
                return TstError::InvalidUsage as i32;
            }
        };
        prog.stream_descriptors[stream_idx].push(tlv);
        0
    })
}

/// Append one PMT descriptor to a subtitle stream's per-PID descriptor list.
/// Same contract as `tst_mux_config_add_video_descriptor`.
///
/// `stream` is the handle returned by one of
/// `tst_mux_config_add_subtitle_stream_dvb_subtitling`,
/// `tst_mux_config_add_subtitle_stream_dvb_teletext`,
/// `tst_mux_config_add_subtitle_stream_cea708`, or
/// `tst_mux_config_add_subtitle_stream_webvtt`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_mux_config_add_subtitle_descriptor(
    cfg: *mut TstMuxConfig,
    stream: TstSubtitleStreamHandle,
    desc: *const crate::event::TstDescriptor,
) -> libc::c_int {
    ffi_catch(TstError::Internal as i32, || {
        let Some(cfg) = (unsafe { cfg.as_mut() }) else {
            set_last_error(TstError::InvalidConfig, "null config pointer");
            return TstError::InvalidConfig as i32;
        };
        let Some(desc) = (unsafe { desc.as_ref() }) else {
            set_last_error(TstError::InvalidConfig, "null descriptor pointer");
            return TstError::InvalidConfig as i32;
        };
        let tlv = unsafe {
            match desc_to_tlv_blob(desc) {
                Ok(b) => b,
                Err(rc) => return rc,
            }
        };
        let prog_idx = (stream >> 4) as usize;
        let subtitle_within_idx = (stream & 0xF) as usize;
        if prog_idx >= cfg.programs.len() {
            set_last_error(
                TstError::InvalidUsage,
                "invalid subtitle stream handle (program out of range)",
            );
            return TstError::InvalidUsage as i32;
        }
        let prog = &mut cfg.programs[prog_idx];
        let stream_idx = match prog
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, StreamSpec::Subtitle { .. }))
            .nth(subtitle_within_idx)
            .map(|(i, _)| i)
        {
            Some(i) => i,
            None => {
                set_last_error(
                    TstError::InvalidUsage,
                    "invalid subtitle stream handle (stream out of range)",
                );
                return TstError::InvalidUsage as i32;
            }
        };
        prog.stream_descriptors[stream_idx].push(tlv);
        0
    })
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
