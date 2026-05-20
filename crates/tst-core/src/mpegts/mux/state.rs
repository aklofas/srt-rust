//! Per-stream-class state structs cached by `Muxer` at construction
//! time, plus free helper functions used during muxer setup and per-push
//! validation.
//!
//! All items here are `pub(super)` — visible to `mpegts::mux` and its
//! siblings (`push_video`, `push_klv`, `push_audio`, `push_subtitle`,
//! `emit`, `scheduling`, `stats_accounting`) but not outside
//! the parent module.

use super::psi::KLVA_REGISTRATION_DESCRIPTOR;
use super::{AudioCodec, KlvStreamType, MuxerProgramConfig, StreamSpec, SubtitleCodec, VideoCodec};
use crate::error::MuxError;
use crate::mpegts::common::{StreamType, StreamTypeCode};
use crate::mpegts::stats::StreamStats;
use std::collections::BTreeMap;

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

/// Structural Annex-B access-unit validator per H.264 / H.265 / H.266
/// byte-stream syntax (ISO/IEC 14496-10 Annex B; ITU-T H.265 Annex B;
/// ITU-T H.266 Annex B).
///
/// Validate-1 C13: the prior implementation only checked the AU's leading
/// prefix. That accepted malformed inputs whose first 3-4 bytes happened
/// to be a start code but whose interior contained no real NAL units, or
/// trailed off mid-start-code. This walker scans every start code in the
/// buffer and rejects:
///
/// - inputs that do not start with `00 00 01` or `00 00 00 01`,
/// - inputs that contain only start codes with no NAL body byte between
///   them (zero-length NAL unit),
/// - inputs whose final NAL unit is empty (start code at the very tail),
/// - inputs that end mid-start-code (trailing `00 00` or `00` is
///   ambiguous in Annex-B framing).
///
/// Spec note: H.264/H.265/H.266 forbid empty NAL units — a `nal_unit()`
/// element must contain at least a 1-byte (H.264) or 2-byte (H.265/H.266)
/// `nal_unit_header()`. Validating "non-empty NAL" without parsing the
/// header is the minimum compatible check; we don't enforce the
/// codec-specific header size here (codec-specific parsers in
/// `crate::codec` do that downstream).
pub(super) fn validate_annex_b(nal: &[u8]) -> Result<(), MuxError> {
    // Must start with a recognised Annex-B start code.
    let leading_len = if nal.starts_with(&[0x00, 0x00, 0x00, 0x01]) {
        4
    } else if nal.starts_with(&[0x00, 0x00, 0x01]) {
        3
    } else {
        return Err(MuxError::InvalidNal);
    };

    // Walk the buffer counting non-empty NAL units (bytes between
    // consecutive start codes, and between the last start code and EOF).
    let mut nal_count: usize = 0;
    // Position immediately after the current start code (i.e., start of
    // the current NAL unit's body).
    let mut nal_body_start: usize = leading_len;
    let mut i: usize = leading_len;
    while i < nal.len() {
        // Look for the next start code: `00 00 01` or `00 00 00 01`.
        // The byte preceding `00 00 01` may be a `00` (4-byte form) or
        // any other byte (3-byte form); either way the NAL body ends
        // before the leading zero pair.
        if i + 3 <= nal.len() && nal[i] == 0 && nal[i + 1] == 0 && nal[i + 2] == 1 {
            // 3-byte start code — close the current NAL.
            if i <= nal_body_start {
                // Zero-length NAL between two adjacent start codes (no
                // body bytes), e.g., `00 00 01 00 00 01`. Forbidden.
                return Err(MuxError::InvalidNal);
            }
            nal_count += 1;
            nal_body_start = i + 3;
            i += 3;
            continue;
        }
        if i + 4 <= nal.len()
            && nal[i] == 0
            && nal[i + 1] == 0
            && nal[i + 2] == 0
            && nal[i + 3] == 1
        {
            // 4-byte start code — close the current NAL.
            if i <= nal_body_start {
                return Err(MuxError::InvalidNal);
            }
            nal_count += 1;
            nal_body_start = i + 4;
            i += 4;
            continue;
        }
        i += 1;
    }
    // Reject buffers that end mid-start-code (trailing `00` or `00 00`
    // with no terminating `01` byte). The Annex-B framing makes such a
    // tail ambiguous — receivers could read it as a start-code prefix
    // and expect more data. Allow benign trailing zero-byte padding
    // only if the last closed NAL had a body (caller-supplied
    // emulation-prevention-style tails).
    //
    // We only reject if the *very last* bytes look like an unterminated
    // start code AND no NAL body has been emitted after it. This avoids
    // false positives for legitimate emulation prevention bytes inside
    // a NAL body.
    if nal.len() >= 2 && nal[nal.len() - 1] == 0 && nal[nal.len() - 2] == 0 {
        // Trailing `... 00 00` with no terminating `01` — only a problem
        // if those zeros are not part of the last NAL body. Cheap proxy:
        // when nal_body_start sits more than 2 bytes from EOF, the zeros
        // are inside a NAL body (fine). When they ARE the start of a
        // would-be new start code (i.e., at the boundary of the last NAL),
        // reject.
        if nal.len() <= nal_body_start + 2 && nal.len() > nal_body_start {
            return Err(MuxError::InvalidNal);
        }
    }

    // Close out the final NAL (from `nal_body_start` to EOF).
    if nal.len() <= nal_body_start {
        // No bytes after the last start code — final NAL is empty.
        return Err(MuxError::InvalidNal);
    }
    nal_count += 1;

    if nal_count == 0 {
        // Defensive: leading start code guarantees ≥1 NAL closes above,
        // so this branch is unreachable in practice. Keep as a safety
        // net so the function returns Err on any zero-NAL slip.
        return Err(MuxError::InvalidNal);
    }

    Ok(())
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

/// Collect per-stream-class state vectors for one program. Single-pass over
/// `prog.streams`. Matches the `filter_map` collections that previously lived
/// in `Muxer::new`.
pub(super) fn collect_stream_states(
    prog: &MuxerProgramConfig,
) -> (
    Vec<VideoStreamState>,
    Vec<KlvStreamState>,
    Vec<AudioStreamState>,
    Vec<SubtitleStreamState>,
) {
    let video: Vec<VideoStreamState> = prog
        .streams
        .iter()
        .filter_map(|s| match s {
            StreamSpec::Video { pid, codec } => Some(VideoStreamState {
                pid: *pid,
                codec: *codec,
            }),
            _ => None,
        })
        .collect();
    let klv: Vec<KlvStreamState> = prog
        .streams
        .iter()
        .filter_map(|s| match s {
            StreamSpec::Klv {
                pid,
                stream_type,
                carries_pts,
            } => Some(KlvStreamState {
                pid: *pid,
                stream_type: *stream_type,
                carries_pts: *carries_pts,
                au_cell_sequence_number: 0,
            }),
            _ => None,
        })
        .collect();
    let audio: Vec<AudioStreamState> = prog
        .streams
        .iter()
        .filter_map(|s| match s {
            StreamSpec::Audio { pid, codec, .. } => Some(AudioStreamState {
                pid: *pid,
                codec: *codec,
            }),
            _ => None,
        })
        .collect();
    let subtitle: Vec<SubtitleStreamState> = prog
        .streams
        .iter()
        .filter_map(|s| match s {
            StreamSpec::Subtitle { pid, codec } => Some(SubtitleStreamState {
                pid: *pid,
                codec: codec.clone(),
            }),
            _ => None,
        })
        .collect();
    (video, klv, audio, subtitle)
}

/// Resolve the PCR-carrying PID for a program. Priority order: caller-pinned >
/// first video > first KLV > first audio. `validate()` guarantees ≥1 stream per
/// program, so the unwrap below cannot panic in well-formed configs.
pub(super) fn resolve_pcr_pid(prog: &MuxerProgramConfig) -> u16 {
    prog.pcr_pid.unwrap_or_else(|| {
        prog.first_video_pid()
            .or_else(|| prog.first_klv_pid())
            .or_else(|| prog.first_audio_pid())
            .expect("validate() guarantees ≥1 stream per program")
    })
}

/// Build the per-stream PMT descriptor cache for one program. Each entry is
/// the concatenated descriptor bytes for the corresponding `StreamSpec`,
/// composed of (a) any auto-emitted descriptor (KLVA / AV01 / AC-3 / ISO 639 /
/// subtitle disambiguator) followed by (b) the caller-supplied descriptors
/// from `prog.stream_descriptors[i]`. The auto-emit on (a) is suppressed when
/// the caller has already supplied an equivalent descriptor.
///
/// Mechanically extracted from `Muxer::new`; zero semantic change vs the
/// pre-refactor inline version. See history pre-Wave 6 for the per-codec
/// suppression rationale (KLVA / AV01 / AC-3 mirror each other; subtitle
/// suppression matches the demux-side classifier).
pub(super) fn build_pmt_descriptor_cache(prog: &MuxerProgramConfig) -> Vec<Vec<u8>> {
    let mut cache: Vec<Vec<u8>> = Vec::with_capacity(prog.streams.len());
    for (i, spec) in prog.streams.iter().enumerate() {
        let caller_descs = &prog.stream_descriptors[i];
        let caller_has_registration = caller_descs
            .iter()
            .any(|tlv| !tlv.is_empty() && tlv[0] == 0x05);

        if matches!(spec, StreamSpec::Klv { .. }) {
            for tlv in caller_descs {
                if tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"KLVA" {
                    tracing::warn!(
                        "caller-supplied Registration descriptor on KLV PID has \
                         non-KLVA format_identifier ({:?}); receivers may not \
                         recognize the stream as KLV",
                        std::str::from_utf8(&tlv[2..6]).unwrap_or("?")
                    );
                }
            }
        }

        let mut bytes = Vec::new();
        // KLVA Registration auto-emit on KLV streams (both
        // PrivateData=0x06 and SynchronousMetadata=0x15). ffmpeg
        // mpegtsenc.c:817-818 emits KLVA on the metadata
        // stream_type path too — receivers gate KLV
        // classification on the descriptor regardless of
        // stream_type. Sync KLV with metadata_descriptor
        // (tag 0x26) doesn't *replace* KLVA — TSDuck + ffmpeg
        // consume both side-by-side.
        if matches!(
            spec,
            StreamSpec::Klv {
                stream_type: KlvStreamType::PrivateData | KlvStreamType::SynchronousMetadata,
                ..
            }
        ) && !caller_has_registration
        {
            bytes.extend_from_slice(KLVA_REGISTRATION_DESCRIPTOR);
        }
        // AV1 auto-emit: AV01 registration_descriptor (binding §2.1).
        // MUST be the FIRST descriptor in the per-stream PMT loop —
        // receivers gate AV1 classification on stream_type 0x06 +
        // first-position AV01 Registration. Suppress when the caller
        // has already supplied an AV01 Registration (mirrors KLVA
        // suppression). If the caller supplied a Registration with a
        // non-AV01 format_identifier, log warn but still auto-emit so
        // the stream stays classifiable as AV1 — we don't silently
        // override caller intent, but we don't let a stray non-AV01
        // Registration silently break receiver classification either.
        if let StreamSpec::Video {
            codec: VideoCodec::Av1,
            ..
        } = spec
        {
            let caller_has_av01 = caller_descs
                .iter()
                .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] == b"AV01");
            let caller_has_other_registration = caller_descs
                .iter()
                .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"AV01");
            if caller_has_other_registration && !caller_has_av01 {
                tracing::warn!(
                    "caller-supplied Registration descriptor on AV1 PID has \
                     non-AV01 format_identifier; receivers may not recognize \
                     the stream as AV1"
                );
            }
            if !caller_has_av01 {
                bytes.extend_from_slice(&crate::mpegts::descriptors::format_identifier_av01());
            }
        }
        // AC-3 auto-emit: Registration descriptor with format_identifier
        // "AC-3" per ATSC A/52 §A.2.3. Receivers use this to distinguish
        // AC-3 from other private-stream-1 (PES stream_id 0xBD) audio.
        // Suppression mirrors the KLVA / AV01 rules: suppress when the
        // caller has already supplied an AC-3 Registration (tag 0x05 with
        // format_identifier == b"AC-3"). If the caller supplied a
        // Registration with a different format_identifier, log warn but
        // do NOT auto-emit — caller intent takes precedence and we don't
        // silently override it.
        if let StreamSpec::Audio {
            codec: AudioCodec::Ac3,
            ..
        } = spec
        {
            let caller_has_ac3 = caller_descs
                .iter()
                .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] == b"AC-3");
            let caller_has_other_registration = caller_descs
                .iter()
                .any(|tlv| tlv.len() >= 6 && tlv[0] == 0x05 && &tlv[2..6] != b"AC-3");
            if caller_has_other_registration && !caller_has_ac3 {
                tracing::warn!(
                    "caller-supplied Registration descriptor on AC-3 PID has \
                     non-AC-3 format_identifier; receivers may not recognize \
                     the stream as AC-3"
                );
            }
            if !caller_has_ac3 {
                bytes.extend_from_slice(&crate::mpegts::descriptors::format_identifier_ac3());
            }
        }
        // ISO 639 language descriptor auto-emit on Audio when
        // StreamSpec::Audio.language is Some. Per ISO/IEC 13818-1
        // §2.6.18-19 (tag 0x0A, length 4: 3 lang bytes + 1
        // audio_type byte). audio_type=0x00 (undefined / clean
        // main) is the spec default; richer values come from
        // caller-supplied stream_descriptors_for_audio. Suppress
        // when caller already supplied any tag-0x0A descriptor —
        // caller intent wins (their language code may differ).
        if let StreamSpec::Audio {
            language: Some(lang),
            ..
        } = spec
        {
            let caller_has_lang = caller_descs
                .iter()
                .any(|tlv| !tlv.is_empty() && tlv[0] == 0x0A);
            if !caller_has_lang {
                bytes.extend_from_slice(&crate::mpegts::descriptors::iso_639_language(*lang, 0x00));
            }
        }
        // Subtitle auto-emit: codec-disambiguating per-stream descriptor.
        // All four SubtitleCodec variants ride PMT stream_type 0x06; the
        // descriptor here is what tells receivers which codec rides on
        // this PID. Mirrors the KLV/AV1 caller-supplied-Registration
        // suppression rule: when the caller has already supplied any
        // descriptor that the receiver-side classifier recognizes as a
        // subtitle codec marker (subtitling 0x59 / teletext 0x56 /
        // VBI teletext 0x46 / Registration with VTTC or GA94
        // format_identifier), the auto-emit is suppressed — caller's
        // takes precedence and we don't double-emit.
        if let StreamSpec::Subtitle { codec, .. } = spec {
            if !caller_has_recognized_subtitle_descriptor(caller_descs) {
                let auto = match codec {
                    SubtitleCodec::DvbSubtitling {
                        language,
                        subtitling_type,
                        composition_page_id,
                        ancillary_page_id,
                    } => crate::mpegts::descriptors::subtitling_descriptor(
                        *language,
                        *subtitling_type,
                        *composition_page_id,
                        *ancillary_page_id,
                    ),
                    SubtitleCodec::DvbTeletext {
                        language,
                        teletext_type,
                        magazine_number,
                        page_number,
                    } => crate::mpegts::descriptors::teletext_descriptor(
                        *language,
                        *teletext_type,
                        *magazine_number,
                        *page_number,
                    ),
                    SubtitleCodec::Cea708Standalone => {
                        crate::mpegts::descriptors::format_identifier_ga94()
                    }
                    SubtitleCodec::WebVttInTs => {
                        crate::mpegts::descriptors::format_identifier_vttc()
                    }
                };
                bytes.extend_from_slice(&auto);
            }
        }
        for tlv in caller_descs {
            bytes.extend_from_slice(tlv);
        }
        cache.push(bytes);
    }
    cache
}

/// Initialize per-stream stats entries for one program. Mutates `into` rather
/// than returning a fresh BTreeMap so the caller can accumulate across
/// programs in one shared map without an extra allocation+merge pass.
pub(super) fn initialize_stats(
    prog: &MuxerProgramConfig,
    video: &[VideoStreamState],
    klv: &[KlvStreamState],
    audio: &[AudioStreamState],
    subtitle: &[SubtitleStreamState],
    into: &mut BTreeMap<u16, StreamStats>,
) {
    for v in video {
        let stream_type_byte = match v.codec {
            VideoCodec::H264 => StreamType::H264.as_u8(),
            VideoCodec::H265 => StreamType::H265.as_u8(),
            VideoCodec::H266 => StreamType::H266.as_u8(),
            // AV1 rides PMT stream_type 0x06; the AV01
            // registration_descriptor disambiguates on the receiver
            // (auto-emitted in the per-stream descriptor cache).
            VideoCodec::Av1 => StreamType::KlvPrivate.as_u8(),
        };
        into.insert(
            v.pid,
            StreamStats {
                pid: v.pid,
                stream_type: StreamTypeCode::from_byte(stream_type_byte),
                program_number: prog.program_number,
                ..Default::default()
            },
        );
    }
    for k in klv {
        let stream_type_byte = match k.stream_type {
            KlvStreamType::PrivateData => StreamType::KlvPrivate.as_u8(),
            KlvStreamType::SynchronousMetadata => StreamType::KlvSyncMetadata.as_u8(),
        };
        into.insert(
            k.pid,
            StreamStats {
                pid: k.pid,
                stream_type: StreamTypeCode::from_byte(stream_type_byte),
                program_number: prog.program_number,
                ..Default::default()
            },
        );
    }
    for a in audio {
        let stream_type_byte = match a.codec {
            AudioCodec::Mp2 => StreamType::AudioMp2.as_u8(),
            AudioCodec::Aac => StreamType::AudioAac.as_u8(),
            AudioCodec::AacLatm => StreamType::AudioAacLatm.as_u8(),
            AudioCodec::Ac3 => StreamType::AudioAc3.as_u8(),
        };
        into.insert(
            a.pid,
            StreamStats {
                pid: a.pid,
                stream_type: StreamTypeCode::from_byte(stream_type_byte),
                program_number: prog.program_number,
                ..Default::default()
            },
        );
    }
    for s in subtitle {
        // All four subtitle codecs ride PMT stream_type 0x06
        // (PrivateData); the per-stream PMT descriptor
        // disambiguates between DVB-sub, teletext, CEA-708
        // standalone, and WebVTT-in-TS. The codec-derived label
        // is the one human-readable distinguisher in stats.
        into.insert(
            s.pid,
            StreamStats {
                pid: s.pid,
                stream_type: StreamTypeCode::from_byte(StreamType::KlvPrivate.as_u8()),
                program_number: prog.program_number,
                label: Some(crate::mpegts::stats::subtitle_codec_label(&s.codec).to_string()),
                ..Default::default()
            },
        );
    }
}
