//! Per-stream-class state structs cached by `Muxer` at construction
//! time, plus free helper functions used during muxer setup and per-push
//! validation.
//!
//! All items here are `pub(super)` — visible to `mpegts::mux` and its
//! siblings (`push_video`, `push_klv`, `push_audio`, `push_subtitle`,
//! `emit`, `scheduling`, `stats_accounting`) but not outside
//! the parent module.

use super::{AudioCodec, KlvStreamType, SubtitleCodec, VideoCodec};
use crate::error::MuxError;

/// Per-video-stream cached state. Built once at `Muxer::new` time.
pub(super) struct VideoStreamState {
    pub(super) pid: u16,
    pub(super) codec: VideoCodec,
}

/// Per-KLV-stream cached state.
pub(super) struct KlvStreamState {
    pub(super) pid: u16,
    pub(super) stream_type: KlvStreamType,
    pub(super) carries_pts: bool,
    /// For `SynchronousMetadata` streams: incrementing AU cell `sequence_number`,
    /// wraps modulo 256 per H.222.0 §2.12.4.2 Table 2-156 semantics. Unused
    /// for `PrivateData` streams.
    pub(super) au_cell_sequence_number: u8,
}

/// Per-audio-stream cached state.
pub(super) struct AudioStreamState {
    pub(super) pid: u16,
    pub(super) codec: AudioCodec,
}

/// Per-subtitle-stream cached state. `codec` is `Clone` (not `Copy`) so we
/// store it owned per-stream — same shape as `SubtitleCodec` itself.
pub(super) struct SubtitleStreamState {
    pub(super) pid: u16,
    pub(super) codec: SubtitleCodec,
}

pub(super) fn validate_annex_b(nal: &[u8]) -> Result<(), MuxError> {
    if nal.starts_with(&[0x00, 0x00, 0x00, 0x01]) || nal.starts_with(&[0x00, 0x00, 0x01]) {
        Ok(())
    } else {
        Err(MuxError::InvalidNal)
    }
}

/// True iff `caller_descs` contains any descriptor that the receiver-side
/// subtitle classifier recognizes as a codec marker. Mirrors the demux-side
/// `mpegts::demux::pmt_classify::has_recognized_subtitle_descriptor` predicate
/// but operates on raw TLV bytes (the form held in `prog.stream_descriptors`)
/// rather than on parsed `RawDescriptor`.
///
/// Used to suppress the subtitle auto-emit when the caller has already
/// supplied one of:
///   * `subtitling_descriptor`   (tag 0x59)
///   * `teletext_descriptor`     (tag 0x56)
///   * `VBI_teletext_descriptor` (tag 0x46)
///   * `registration_descriptor` (tag 0x05) with format_identifier
///     `"VTTC"` or `"GA94"`
///
/// Mirrors the KLV/AV1 caller-supplied-Registration suppression rule so
/// receivers don't see two competing codec markers on the same PID.
pub(super) fn caller_has_recognized_subtitle_descriptor(caller_descs: &[Vec<u8>]) -> bool {
    for tlv in caller_descs {
        if tlv.is_empty() {
            continue;
        }
        let tag = tlv[0];
        if tag == 0x59 || tag == 0x56 || tag == 0x46 {
            return true;
        }
        // registration_descriptor TLV layout: tag(1) + length(1) + body(length).
        // format_identifier is the first 4 body bytes.
        if tag == 0x05 && tlv.len() >= 6 {
            let fid = &tlv[2..6];
            if fid == b"VTTC" || fid == b"GA94" {
                return true;
            }
        }
    }
    false
}

/// Number of 188-byte TS packets needed to carry `payload_size` bytes of
/// PES (header + ES). 184 = 188 - 4 byte TS header. Adaptation field eats
/// further capacity but for sizing purposes the worst case is no AF (gives
/// the smallest packet count). The orchestrator may emit one more packet
/// than this if AF stuffing pushes a byte over; we allow a 1-packet slop
/// in the buffer reservation.
pub(super) fn ts_packets_for(payload_size: usize) -> usize {
    payload_size.div_ceil(184).max(1) + 1
}
