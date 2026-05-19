//! MuxSender-side MPEG-TS muxer.
//!
//! The public surface is `Muxer`, `MuxerConfig`, `VideoCodec`,
//! `KlvStreamType`. Internal helpers live in `ts`, `psi`, `pes` submodules.
//!
//! Re-export note: `Muxer`, `VideoCodec`, and `KlvStreamType` are re-exported
//! at the crate root (`tst_core::Muxer` etc.). `MuxerConfig` deliberately is
//! not — callers reach it via `mpegts::mux::MuxerConfig` so the construction
//! site is visually distinct from the SRT `SocketConfig` / `ListenerConfig`
//! already at the crate root. Don't "symmetry-fix" this.

pub(crate) mod pes;
pub(crate) mod psi;
pub(crate) mod ts;

mod state;
mod scheduling;
mod stats_accounting;
mod push_audio;
mod push_klv;
mod push_subtitle;
mod push_video;

use crate::error::MuxError;

mod types;
pub use types::*;

mod config;
pub use config::*;
pub use stats_accounting::MuxerStats;

/// Spec-domain tier of muxer errors.
///
/// The 32-variant [`crate::error::MuxError`] enum (canonical home at
/// [`crate::error::MuxError`]) is re-exported here under the `_detail`
/// convention to signal "spec-knowledge tier; production binding code
/// should prefer [`crate::error::MuxError::kind()`] for
/// action-discriminating dispatch".
///
/// The underscore prefix follows the workspace convention for re-export
/// modules that signal "opt into the spec-domain tier" at the import site:
///
/// ```
/// use tst_core::mpegts::mux::_detail::MuxError;
///
/// fn diagnose_klv(e: &MuxError) -> Option<String> {
///     match e {
///         MuxError::KlvTooLarge { size, max } => Some(format!(
///             "KLV LS exceeds PES cap: {size} > {max}"
///         )),
///         _ => None,
///     }
/// }
/// ```
///
/// Equivalent to importing from the canonical path [`crate::error::MuxError`];
/// the re-export exists for documentary intent (the underscore signals
/// "I know I'm pattern-matching the spec-domain variants and I accept
/// future variant additions").
///
/// # Stability
///
/// The inner variant set is `#[non_exhaustive]` and may grow.
/// New variants WILL be added without a major version bump;
/// pattern-matching code here should include a wildcard arm. If you
/// only need action-discriminating categorization, prefer the
/// coarse-tier [`crate::error::MuxSenderErrorKind`] enum via
/// [`crate::error::MuxError::kind()`].
pub mod _detail {
    /// The canonical 32-variant [`crate::error::MuxError`].
    pub use crate::error::MuxError;
}

use crate::mpegts::common::{StreamType, StreamTypeCode};
use std::collections::{BTreeMap, VecDeque};

use self::pes::MAX_PES_HEADER_SIZE;
use self::psi::KLVA_REGISTRATION_DESCRIPTOR;
use self::state::{
    AudioStreamState, KlvStreamState, SubtitleStreamState, VideoStreamState,
    caller_has_recognized_subtitle_descriptor,
};
use self::ts::ContinuityCounters;

/// MuxSender-side MPEG-TS muxer.
///
/// Construct with `Muxer::new(config)`, push encoded frames via `push_video`
/// and `push_klv`, then drain TS packets with `pull`. The muxer is
/// deterministic — output is a function of inputs only, not wall-clock time.
///
/// # Closing
///
/// `Muxer` is a passive aggregator — it owns no transport and no OS
/// handles. Drop is the only shutdown and is trivially synchronous.
/// Call [`Self::pull`] in a loop until it returns `0` before drop to
/// drain any TS packets sitting in the internal queue; bytes left in
/// the queue at drop are discarded.
///
/// ## Per-language idiom
///
/// | Language | Idiom |
/// |----------|-------|
/// | Rust | `while muxer.pull(&mut buf) > 0 { /* ... */ } drop(muxer);` (or just let it fall out of scope) |
/// | Java | Drain via `pull()`, then let GC reclaim — no `AutoCloseable` needed |
/// | Kotlin | Drain via `pull()`, then let GC reclaim |
/// | Swift | `deinit` calls drop; explicit drain via `pull()` before exit |
/// | Python | Drain via `pull()` at end-of-stream; let GC reclaim |
/// | C | `tst_muxer_close(muxer)` — releases the muxer; caller is responsible for prior drain |
pub struct Muxer {
    config: MuxerConfig,

    /// Per-program pre-composed PMT descriptor bytes.
    /// `pmt_descriptor_caches[prog_idx][stream_idx]` = concatenated TLVs for
    /// that stream (KLVA auto-emit + caller-supplied). Indexed parallel to
    /// `config.programs[prog_idx].streams`. Borrowed at PMT emission time;
    /// never reallocated after construction.
    pmt_descriptor_caches: Vec<Vec<Vec<u8>>>,

    /// Per-program video stream state. `video_streams[prog_idx]` is the
    /// list of video streams for program `prog_idx`, in declaration order.
    /// `VideoStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    video_streams: Vec<Vec<VideoStreamState>>,

    /// Per-program KLV stream state. Same indexing as `video_streams`.
    klv_streams: Vec<Vec<KlvStreamState>>,

    /// Per-program audio stream state. Same indexing as `video_streams`.
    /// `AudioStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    audio_streams: Vec<Vec<AudioStreamState>>,

    /// Per-program subtitle stream state. Same indexing as `video_streams`.
    /// `SubtitleStreamHandle::unpack()` → `(prog_idx, within_idx)` indexes here.
    subtitle_streams: Vec<Vec<SubtitleStreamState>>,

    /// Per-program resolved PCR PID. Indexed parallel to `config.programs`.
    pcr_pids: Vec<u16>,

    pcr_interval_27mhz: u64,
    psi_interval_90khz: i64,

    queue: VecDeque<[u8; 188]>,
    counters: ContinuityCounters,

    /// Per-program last PSI emission PTS, masked to 33 bits. None until first.
    /// Indexed parallel to `config.programs`.
    psi_last: Vec<Option<u64>>,

    /// Per-program last PCR emission value at 27 MHz. None until first.
    /// Indexed parallel to `config.programs`.
    pcr_last: Vec<Option<u64>>,

    // ── Stats counters ────────────────────────────────────────────────────
    ts_packets_emitted: u64,
    ts_bytes_emitted: u64,
    /// Keyed by PID. Populated eagerly at construction for every configured
    /// stream; only the `items` / `bytes` / `discontinuities` fields change
    /// at runtime. `pid` and `stream_type` are set at construction and
    /// never modified.
    per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats>,

    /// Per-PID codec-specific counters. Allocated lazily on first push
    /// for a PID whose StreamSpec falls into a counted family. Subtitle
    /// PIDs and audio PIDs with LATM/AC-3 codecs do NOT get an entry.
    stream_codec_counters: BTreeMap<u16, crate::mpegts::stats::StreamCodecCounters>,

    /// Scratch buffer reused across push_video / push_klv / push_audio /
    /// push_subtitle calls. Cleared and refilled each AU; avoids one
    /// per-AU heap allocation on the hot path. Grows to the largest AU seen
    /// and stays there — typical size is MAX_PES_HEADER_SIZE + payload.
    pes_scratch: Vec<u8>,
}

impl Muxer {
    /// Construct and validate.
    pub fn new(config: MuxerConfig) -> Result<Self, MuxError> {
        config.validate()?;

        let n_programs = config.programs.len();
        let pcr_interval_27mhz = (config.pcr_interval_ms as u64) * 27_000;
        let psi_interval_90khz = (config.psi_interval_ms as i64) * 90;

        // Build per-program state vectors in a single pass over programs.
        let mut video_streams: Vec<Vec<VideoStreamState>> = Vec::with_capacity(n_programs);
        let mut klv_streams: Vec<Vec<KlvStreamState>> = Vec::with_capacity(n_programs);
        let mut audio_streams: Vec<Vec<AudioStreamState>> = Vec::with_capacity(n_programs);
        let mut subtitle_streams: Vec<Vec<SubtitleStreamState>> = Vec::with_capacity(n_programs);
        let mut pcr_pids: Vec<u16> = Vec::with_capacity(n_programs);
        let mut pmt_descriptor_caches: Vec<Vec<Vec<u8>>> = Vec::with_capacity(n_programs);
        let mut per_stream: BTreeMap<u16, crate::mpegts::stats::StreamStats> = BTreeMap::new();

        for prog in &config.programs {
            // Per-program video + KLV stream state.
            let prog_video: Vec<VideoStreamState> = prog
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
            let prog_klv: Vec<KlvStreamState> = prog
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
            let prog_audio: Vec<AudioStreamState> = prog
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
            let prog_subtitle: Vec<SubtitleStreamState> = prog
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

            // Resolve PCR PID for this program: caller-pin or auto-fallback.
            // Priority: caller-pinned > first video > first KLV > first audio.
            let pcr_pid = prog.pcr_pid.unwrap_or_else(|| {
                prog.first_video_pid()
                    .or_else(|| prog.first_klv_pid())
                    .or_else(|| prog.first_audio_pid())
                    .expect("validate() guarantees ≥1 stream per program")
            });

            // Pre-compose per-stream descriptor bytes for this program.
            // The auto-emitted KLVA Registration on PrivateData KLV PIDs is
            // suppressed when the caller supplies their own Registration
            // descriptor — TSDuck and ffprobe both flag duplicate Registrations,
            // and the corpus shows real senders never duplicate.
            let mut prog_cache: Vec<Vec<u8>> = Vec::with_capacity(prog.streams.len());
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
                        stream_type: KlvStreamType::PrivateData
                            | KlvStreamType::SynchronousMetadata,
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
                        bytes.extend_from_slice(
                            &crate::mpegts::descriptors::format_identifier_av01(),
                        );
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
                        bytes.extend_from_slice(
                            &crate::mpegts::descriptors::format_identifier_ac3(),
                        );
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
                        bytes.extend_from_slice(&crate::mpegts::descriptors::iso_639_language(
                            *lang, 0x00,
                        ));
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
                prog_cache.push(bytes);
            }

            // Eagerly create per-stream stats entries.
            for v in &prog_video {
                let stream_type_byte = match v.codec {
                    VideoCodec::H264 => StreamType::H264.as_u8(),
                    VideoCodec::H265 => StreamType::H265.as_u8(),
                    VideoCodec::H266 => StreamType::H266.as_u8(),
                    // AV1 rides PMT stream_type 0x06; the AV01
                    // registration_descriptor disambiguates on the receiver
                    // (auto-emitted in the per-stream descriptor cache).
                    VideoCodec::Av1 => StreamType::KlvPrivate.as_u8(),
                };
                per_stream.insert(
                    v.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: v.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_byte),
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for k in &prog_klv {
                let stream_type_byte = match k.stream_type {
                    KlvStreamType::PrivateData => StreamType::KlvPrivate.as_u8(),
                    KlvStreamType::SynchronousMetadata => StreamType::KlvSyncMetadata.as_u8(),
                };
                per_stream.insert(
                    k.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: k.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_byte),
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for a in &prog_audio {
                let stream_type_byte = match a.codec {
                    AudioCodec::Mp2 => StreamType::AudioMp2.as_u8(),
                    AudioCodec::Aac => StreamType::AudioAac.as_u8(),
                    AudioCodec::AacLatm => StreamType::AudioAacLatm.as_u8(),
                    AudioCodec::Ac3 => StreamType::AudioAc3.as_u8(),
                };
                per_stream.insert(
                    a.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: a.pid,
                        stream_type: StreamTypeCode::from_byte(stream_type_byte),
                        program_number: prog.program_number,
                        ..Default::default()
                    },
                );
            }
            for s in &prog_subtitle {
                // All four subtitle codecs ride PMT stream_type 0x06
                // (PrivateData); the per-stream PMT descriptor
                // disambiguates between DVB-sub, teletext, CEA-708
                // standalone, and WebVTT-in-TS. The codec-derived label
                // is the one human-readable distinguisher in stats.
                per_stream.insert(
                    s.pid,
                    crate::mpegts::stats::StreamStats {
                        pid: s.pid,
                        stream_type: StreamTypeCode::from_byte(StreamType::KlvPrivate.as_u8()),
                        program_number: prog.program_number,
                        label: Some(
                            crate::mpegts::stats::subtitle_codec_label(&s.codec).to_string(),
                        ),
                        ..Default::default()
                    },
                );
            }

            video_streams.push(prog_video);
            klv_streams.push(prog_klv);
            audio_streams.push(prog_audio);
            subtitle_streams.push(prog_subtitle);
            pcr_pids.push(pcr_pid);
            pmt_descriptor_caches.push(prog_cache);
        }

        Ok(Self {
            config,
            pmt_descriptor_caches,
            video_streams,
            klv_streams,
            audio_streams,
            subtitle_streams,
            pcr_pids,
            pcr_interval_27mhz,
            psi_interval_90khz,
            queue: VecDeque::with_capacity(64),
            counters: ContinuityCounters::new(),
            psi_last: vec![None; n_programs],
            pcr_last: vec![None; n_programs],
            ts_packets_emitted: 0,
            ts_bytes_emitted: 0,
            per_stream,
            stream_codec_counters: BTreeMap::new(),
            // 8 KiB heuristic: covers a typical small AU without reallocation;
            // grows automatically on first oversized frame and stays there.
            pes_scratch: Vec::with_capacity(MAX_PES_HEADER_SIZE + 8192),
        })
    }

    /// Drain ready TS packets into `out`.
    ///
    /// Returns the number of bytes written: 0 or a positive multiple of 188.
    /// `0` indicates either an empty queue or `out.len() < 188`. Pull is
    /// infallible — there are no failure modes that don't already surface
    /// at `push_video` / `push_klv` time (buffer-full, validation).
    ///
    /// # C ABI
    ///
    /// `tst_muxer_pull` — see `crates/tst-c/include/tstrans.h`.
    pub fn pull(&mut self, out: &mut [u8]) -> usize {
        if out.len() < 188 {
            return 0;
        }
        let max_packets = (out.len() / 188).min(self.queue.len());
        for i in 0..max_packets {
            let pkt = self.queue.pop_front().expect("checked count");
            out[i * 188..(i + 1) * 188].copy_from_slice(&pkt);
        }
        let n = max_packets * 188;
        self.ts_packets_emitted += max_packets as u64;
        self.ts_bytes_emitted += n as u64;
        n
    }

    /// Number of 188-byte TS packets currently queued in the muxer's
    /// internal output buffer awaiting [`Muxer::pull`]. This is a live
    /// gauge — non-zero between a `push_*` call and the subsequent
    /// drain. Compared against [`Muxer::capacity_packets`] this gives
    /// the back-pressure ratio used by `tst_pipeline::MuxSender`'s
    /// observability hooks.
    pub fn pending_packets(&self) -> u64 {
        self.queue.len() as u64
    }

    /// The configured queue capacity in 188-byte TS packets — a snapshot
    /// of `MuxerConfig::buffer_packets`. A `push_*` that would push the
    /// queue past this cap returns [`crate::error::MuxError::BufferFull`].
    #[must_use]
    pub fn capacity_packets(&self) -> u64 {
        self.config.buffer_packets as u64
    }

    /// Return the resolved PCR PID for program at `prog_idx` (0-based index).
    /// Returns `None` if `prog_idx` is out of range.
    #[cfg(test)]
    pub(crate) fn pcr_pid_for_program(&self, prog_idx: usize) -> Option<u16> {
        self.pcr_pids.get(prog_idx).copied()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────
//
// The 2500+ LoC of inline test code is split into focused files under
// `tests/`. Each file is declared as a direct child of this module via
// `#[path]` so that `use super::*` inside each file resolves against
// `mpegts::mux` — the same scope the original inline blocks had.
//
// DO NOT add a `mod tests { ... }` wrapper here. The `#[path]` attribute
// makes each file a DIRECT child of `mux`, preserving the same
// `use super::*` scope that the inline tests relied on (including
// `pub(super)` helpers like `validate_language_code` from `config.rs`).

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests_config;

#[cfg(test)]
#[path = "tests/handles.rs"]
mod tests_handles;

#[cfg(test)]
#[path = "tests/push.rs"]
mod tests_push;

#[cfg(test)]
#[path = "tests/subtitle.rs"]
mod tests_subtitle;

#[cfg(test)]
#[path = "tests/validation.rs"]
mod tests_validation;

#[cfg(test)]
#[path = "tests/stats.rs"]
mod tests_stats;
