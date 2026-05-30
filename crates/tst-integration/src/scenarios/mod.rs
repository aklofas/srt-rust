//! Cross-binding scenario definitions.
//!
//! Each scenario is a self-contained unit that:
//!  1. Generates a deterministic synthetic input artifact (`.ts` bytes or
//!     similar) into `out_dir/<id>/`.
//!     (`out_dir` is the crate-local `crates/tst-integration/tests/fixtures/scenarios/`
//!     directory — canonical; all adapters resolve here, not workspace-root.)
//!  2. Returns a `Golden` envelope that the Rust adapter test verifies against.
//!
//! # Synthetic data only
//!
//! These generators NEVER read from `testfiles/`, any `local/` directory, or
//! any real corpus.  All input data is synthesised programmatically so that
//! the scenarios are hermetic and reproducible on any machine.

pub mod golden;

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tst_core::mpegts::common::Pts90khz;
use tst_core::mpegts::mux::{
    KlvStreamType, Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec,
};

use golden::{CoreEvent, Golden};

/// Trait every scenario implements.
pub trait Scenario {
    fn id(&self) -> &'static str;
    fn kind(&self) -> &'static str; // "demux" | "roundtrip" | "binding_contract"
    fn features(&self) -> Vec<&'static str>; // empty for Tier A
    fn tier(&self) -> &'static str; // "A"

    /// Generate the input artifact under `out_dir/<id>/` and return
    /// `(relative_path_of_input, Golden)`.
    fn generate(&self, out_dir: &Path) -> (PathBuf, Golden);
}

/// All registered scenarios.
pub fn all_scenarios() -> Vec<Box<dyn Scenario>> {
    vec![
        Box::new(H264St0601Mp),
        Box::new(VideoRoundtrip),
        Box::new(StrictRejection),
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared mux helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Drain all buffered TS packets from `mux` into a `Vec<u8>`.
pub(crate) fn drain_mux(mux: &mut Muxer) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 1316]; // 7 × 188
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    out
}

/// Hex-encode a SHA-256 digest.
fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(digest)
}

// Minimal hex encoding — avoids pulling in the `hex` crate.
mod hex {
    pub(super) fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write;
            write!(s, "{b:02x}").unwrap();
            s
        })
    }
}

/// Write `data` to `path`, creating parent directories as needed.
fn write_file(path: &Path, data: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create scenario dir");
    }
    std::fs::write(path, data).expect("write scenario input artifact");
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1 — h264-st0601-mp (demux)
// ─────────────────────────────────────────────────────────────────────────────

/// `kind = "demux"`: synthetic H.264 video + async ST 0601 KLV muxed to a TS,
/// golden is the normalised demux event sequence.
///
/// KLV stream is configured `PrivateData` (async, stream_type 0x06) so there
/// is no AU cell wrapping and the demux path is straightforward.  The muxer
/// auto-wraps sync KLV (SynchronousMetadata) in AU cell headers; async does
/// not require that.
///
/// This generator NEVER reads from `testfiles/`, `local/`, or any real corpus.
struct H264St0601Mp;

impl Scenario for H264St0601Mp {
    fn id(&self) -> &'static str {
        "h264-st0601-mp"
    }
    fn kind(&self) -> &'static str {
        "demux"
    }
    fn features(&self) -> Vec<&'static str> {
        vec![]
    }
    fn tier(&self) -> &'static str {
        "A"
    }

    fn generate(&self, out_dir: &Path) -> (PathBuf, Golden) {
        // Build a simple single-program muxer: H.264 video + async KLV.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x1011, VideoCodec::H264);
            prog.add_klv(
                0x1031,
                KlvStreamType::PrivateData,
                /*carries_pts=*/ false,
            );
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().expect("valid muxer config")
        };
        let mut mux = Muxer::new(cfg).expect("muxer init");

        // Synthetic H.264 IDR NAL — Annex-B start code + IDR nal_unit_type (5).
        // This generator NEVER reads from testfiles/ or local/ directories.
        let video_au = synthetic_h264_idr();
        let pts = Pts90khz::new(90_000); // t=1s
        mux.push_video(&video_au, pts, /*key_frame=*/ true)
            .expect("push_video");

        // Minimal valid KLV LS: MISB ST 0601 UL (16 bytes) + BER length 0.
        // This generator NEVER reads from testfiles/ or local/ directories.
        let klv_payload = minimal_st0601_ls();
        mux.push_klv(&klv_payload, pts, /*metadata_service_id=*/ 0x00)
            .expect("push_klv");

        let ts_bytes = drain_mux(&mut mux);

        // Demux the TS to derive the canonical event sequence.
        let core = demux_to_core_events(&ts_bytes);

        // Write input artifact.
        let artifact_rel = PathBuf::from(self.id()).join("input.ts");
        let artifact_abs = out_dir.join(&artifact_rel);
        write_file(&artifact_abs, &ts_bytes);

        let golden = Golden {
            schema_version: 0,
            lossy: false,
            core,
            extensions: serde_json::Value::Null,
        };
        (artifact_rel, golden)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2 — video-roundtrip (roundtrip)
// ─────────────────────────────────────────────────────────────────────────────

/// `kind = "roundtrip"`: mux synthetic H.264 video to TS bytes; golden is the
/// output bytes + sha256 (byte-identity).
///
/// A video-only program is used because:
/// - The muxer is deterministic → sha256 goldens are stable across runs.
/// - Audio payload (AAC ADTS) is more complex to synthesise correctly;
///   H.264 Annex-B is trivially correct with a 4-byte start code.
///
/// This generator NEVER reads from `testfiles/`, `local/`, or any real corpus.
struct VideoRoundtrip;

impl Scenario for VideoRoundtrip {
    fn id(&self) -> &'static str {
        "video-roundtrip"
    }
    fn kind(&self) -> &'static str {
        "roundtrip"
    }
    fn features(&self) -> Vec<&'static str> {
        vec![]
    }
    fn tier(&self) -> &'static str {
        "A"
    }

    fn generate(&self, out_dir: &Path) -> (PathBuf, Golden) {
        // Single-program, single-video muxer (no KLV) for simplest possible
        // deterministic output. Produced via the shared single-source-of-truth
        // helper so the adapter test can re-run the identical recipe.
        // This generator NEVER reads from testfiles/ or local/ directories.
        let ts_bytes = video_roundtrip_ts_bytes();
        let digest = sha256_hex(&ts_bytes);

        // Write input artifact (the TS output IS the golden artifact for
        // roundtrip verification).
        let artifact_rel = PathBuf::from(self.id()).join("output.ts");
        let artifact_abs = out_dir.join(&artifact_rel);
        write_file(&artifact_abs, &ts_bytes);

        // A roundtrip scenario carries no media events — the whole-stream
        // byte-identity digest lives under `extensions.output_sha256` (the
        // demux-path `payload_sha256` field means "NAL payload hash" and must
        // not be overloaded). The adapter re-muxes and compares both the raw
        // bytes against `output.ts` and the digest against `output_sha256`.
        let golden = Golden {
            schema_version: 0,
            lossy: false,
            core: vec![],
            extensions: serde_json::json!({ "output_sha256": digest }),
        };
        (artifact_rel, golden)
    }
}

/// Re-run the `video-roundtrip` mux and return the deterministic TS bytes.
///
/// Single source of truth shared by `VideoRoundtrip::generate` and the Rust
/// adapter test — no hand-retyped mux recipe.
pub fn video_roundtrip_ts_bytes() -> Vec<u8> {
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("valid muxer config")
    };
    let mut mux = Muxer::new(cfg).expect("muxer init");
    mux.push_video(
        &synthetic_h264_idr(),
        Pts90khz::new(0),
        /*key_frame=*/ true,
    )
    .expect("push_video");
    drain_mux(&mut mux)
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3 — strict-rejection (binding_contract, non-media)
// ─────────────────────────────────────────────────────────────────────────────

/// `kind = "binding_contract"`: deliberately malformed TS fed to a strict-mode
/// demuxer; golden asserts `STRICT_REJECTION` error identity AND that
/// close/drop after an error is idempotent (no panic).
///
/// The "input" artifact is a garbage byte sequence that cannot be a valid
/// MPEG-TS stream: all 0xFF, which contains no 0x47 sync byte within the
/// `SYNC_SEARCH_WINDOW` distance, triggering `DemuxError::Unrecoverable`.
/// With `StrictMode::Full`, *any* non-conformance also becomes a
/// `StrictRejection`; but for a completely garbled stream the simpler
/// `Unrecoverable` error is guaranteed first.
///
/// Stable public code: `"STRICT_REJECTION"` is the umbrella code used by the
/// C and Python adapters for any `DemuxError` that maps to a hard-reject; the
/// Rust test emits it directly for the unrecoverable-input case so the golden
/// code is uniform across languages.
///
/// This generator NEVER reads from `testfiles/`, `local/`, or any real corpus.
struct StrictRejection;

impl Scenario for StrictRejection {
    fn id(&self) -> &'static str {
        "strict-rejection"
    }
    fn kind(&self) -> &'static str {
        "binding_contract"
    }
    fn features(&self) -> Vec<&'static str> {
        vec![]
    }
    fn tier(&self) -> &'static str {
        "A"
    }

    fn generate(&self, out_dir: &Path) -> (PathBuf, Golden) {
        // Garbage bytes: no 0x47 sync byte anywhere → Unrecoverable after
        // walking SYNC_SEARCH_WINDOW (188 * 32 = 6016) bytes.
        // Must be larger than SYNC_SEARCH_WINDOW to trigger the error.
        // This generator NEVER reads from testfiles/ or local/ directories.
        let garbage: Vec<u8> = vec![0xFF; 8192];

        let artifact_rel = PathBuf::from(self.id()).join("input.bin");
        let artifact_abs = out_dir.join(&artifact_rel);
        write_file(&artifact_abs, &garbage);

        let golden = Golden {
            schema_version: 0,
            lossy: false,
            core: vec![CoreEvent::Error {
                code: "STRICT_REJECTION".to_string(),
            }],
            extensions: serde_json::Value::Null,
        };
        (artifact_rel, golden)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Synthetic data helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Minimal valid H.264 IDR access unit in Annex-B framing.
///
/// 4-byte start code + 1-byte NAL header (IDR nal_unit_type = 5,
/// nal_ref_idc = 3) + 15 deterministic filler bytes.
pub(crate) fn synthetic_h264_idr() -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // Annex-B start code
    buf.push(0x65); // forbidden_zero=0, nal_ref_idc=11, nal_unit_type=5 (IDR)
    for i in 0u8..15 {
        buf.push(0xA5 ^ i);
    }
    buf
}

/// Minimal MISB ST 0601 UL (16 bytes) + BER short-form length 0.
///
/// This is the smallest self-consistent KLV LS: the UL identifies the ST 0601
/// set and BER length 0 means zero value bytes.  The muxer treats KLV payload
/// opaquely; the demuxer surfaces it verbatim on the Metadata event.
fn minimal_st0601_ls() -> Vec<u8> {
    vec![
        // MISB ST 0601 UAS Datalink LS UL (SMPTE 336M 16-byte form):
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01, 0x00, 0x00,
        0x00, // UL bytes 1-16
        0x00, // BER short-form length = 0 (no value bytes)
    ]
}

// ─────────────────────────────────────────────────────────────────────────────
// Demux-to-CoreEvent normaliser
// ─────────────────────────────────────────────────────────────────────────────

/// Run tst-core's Demuxer over `ts_bytes` and convert the event stream to a
/// `Vec<CoreEvent>` in canonical order.
///
/// Rules:
/// - `ProgramMap` → skipped (not part of media-event golden).
/// - `Sample { .. Video }` → `CoreEvent::Video`; `payload_sha256` covers the
///   full raw NAL bytes (all NAL units concatenated).
/// - `Sample { .. Audio }` → `CoreEvent::Audio`.
/// - `Metadata { .. }` → `CoreEvent::Klv`.
/// - `Discontinuity` / `NonConformant` / `ReconnectDiscontinuity` → skipped
///   (diagnostics, not part of the media golden).
/// - Unknown stream type → `CoreEvent::Unknown`.
///
/// Errors from `feed` are mapped to `CoreEvent::Error { code }` using the
/// stable public code.
pub fn demux_to_core_events(ts_bytes: &[u8]) -> Vec<CoreEvent> {
    use std::collections::HashMap;
    use tst_core::mpegts::demux::event::SamplePayload;
    use tst_core::mpegts::demux::{DemuxEvent, Demuxer};

    let mut demuxer = Demuxer::new();

    if let Err(e) = demuxer.feed(ts_bytes) {
        return vec![CoreEvent::Error {
            code: demux_error_code(&e),
        }];
    }

    demuxer.flush();

    // Drain all events first so we can build a pid → PMT stream_type byte map
    // from the ProgramMap events before emitting Sample / Metadata events.
    // The golden's `stream_type` is the raw PMT byte (e.g. "0x1b"), which is
    // binding-neutral — a C / Python adapter sees the wire byte, not a Rust
    // enum name.
    let mut raw_events = Vec::new();
    while let Some(ev) = demuxer.next_event() {
        raw_events.push(ev);
    }

    let mut stream_type_by_pid: HashMap<u16, u8> = HashMap::new();
    for ev in &raw_events {
        if let DemuxEvent::ProgramMap(pm) = ev {
            for si in &pm.streams {
                stream_type_by_pid.insert(si.pid, si.stream_type.as_byte());
            }
        }
    }

    // Hex-format a PMT stream_type byte for `pid`, e.g. "0x1b". Falls back to
    // mapping the codec/kind to its canonical PMT byte if the PID was never
    // seen in a ProgramMap (defensive — should not happen for well-formed TS).
    let stream_type_hex = |pid: u16, fallback: u8| -> String {
        let byte = stream_type_by_pid.get(&pid).copied().unwrap_or(fallback);
        format!("0x{byte:02x}")
    };

    let mut events = Vec::new();
    for ev in raw_events {
        match ev {
            DemuxEvent::ProgramMap(_) => { /* skip — topology, not media */ }
            DemuxEvent::Sample {
                stream,
                pts,
                payload,
                ..
            } => {
                match payload {
                    SamplePayload::Video {
                        codec,
                        payload: vp,
                        random_access_indicator,
                    } => {
                        let raw = video_payload_bytes(&vp);
                        events.push(CoreEvent::Video {
                            program: stream.program_number,
                            pid: stream.pid,
                            stream_type: stream_type_hex(stream.pid, video_codec_pmt_byte(codec)),
                            pts: pts.as_ticks(),
                            key: random_access_indicator,
                            payload_sha256: sha256_hex(&raw),
                        });
                    }
                    SamplePayload::Audio { codec, frames } => {
                        events.push(CoreEvent::Audio {
                            program: stream.program_number,
                            pid: stream.pid,
                            stream_type: stream_type_hex(stream.pid, audio_codec_pmt_byte(codec)),
                            pts: pts.as_ticks(),
                            payload_sha256: sha256_hex(&frames),
                        });
                    }
                    SamplePayload::Subtitle { .. } => { /* skip for now */ }
                    SamplePayload::Unknown { .. } => {
                        events.push(CoreEvent::Unknown { pid: stream.pid });
                    }
                }
            }
            DemuxEvent::Metadata {
                stream,
                payload,
                kind,
                ..
            } => {
                // `set` is the MISB set identity derived from the KLV
                // Universal Label key (binding-neutral). Framing info
                // (sync_au_cell vs async) is NOT in the frozen core — it
                // lives under `extensions` if a scenario needs it.
                let _ = &kind;
                events.push(CoreEvent::Klv {
                    program: stream.program_number,
                    pid: stream.pid,
                    stream_type: stream_type_hex(stream.pid, klv_kind_pmt_byte(&stream.kind)),
                    set: klv_set_from_ul(&payload),
                });
            }
            DemuxEvent::Discontinuity { .. }
            | DemuxEvent::NonConformant { .. }
            | DemuxEvent::ReconnectDiscontinuity => { /* diagnostic — skip */ }
        }
    }

    events
}

fn video_payload_bytes(vp: &tst_core::mpegts::demux::event::VideoPayload) -> Vec<u8> {
    use tst_core::mpegts::demux::event::{NalUnit, VideoPayload};
    match vp {
        VideoPayload::Nals(nals) => {
            let mut out = Vec::new();
            for n in nals {
                match n {
                    NalUnit::H264 { payload, .. }
                    | NalUnit::H265 { payload, .. }
                    | NalUnit::H266 { payload, .. } => out.extend_from_slice(payload),
                }
            }
            out
        }
        VideoPayload::Obus(obus) => {
            let mut out = Vec::new();
            for o in obus {
                out.extend_from_slice(&o.payload);
            }
            out
        }
    }
}

// Canonical PMT `stream_type` bytes per ISO/IEC 13818-1 Table 2-34 + the
// codec/metadata extensions tst-core supports.  Used only as a fallback when
// the PID was not seen in a ProgramMap; the primary source is the parsed PMT
// (`StreamInfo::stream_type.as_byte()`).
//   H.264 = 0x1B, H.265 = 0x24, H.266 = 0x33, AV1 = 0x06 (with AV01 reg-desc).
fn video_codec_pmt_byte(codec: tst_core::mpegts::demux::event::VideoCodec) -> u8 {
    use tst_core::mpegts::demux::event::VideoCodec;
    match codec {
        VideoCodec::H264 => 0x1B,
        VideoCodec::H265 => 0x24,
        VideoCodec::H266 => 0x33,
        VideoCodec::Av1 => 0x06,
    }
}

//   MP2 = 0x03/0x04, AAC ADTS = 0x0F, AAC LATM = 0x11, AC-3 = 0x81.
fn audio_codec_pmt_byte(codec: tst_core::mpegts::demux::event::AudioCodec) -> u8 {
    use tst_core::mpegts::demux::event::AudioCodec;
    match codec {
        AudioCodec::Mp2 => 0x03,
        AudioCodec::Aac => 0x0F,
        AudioCodec::AacLatm => 0x11,
        AudioCodec::Ac3 => 0x81,
    }
}

//   KLV sync metadata = 0x15, KLV async (private_data) = 0x06.
fn klv_kind_pmt_byte(kind: &tst_core::mpegts::demux::event::StreamKind) -> u8 {
    use tst_core::mpegts::demux::event::StreamKind;
    match kind {
        StreamKind::KlvSync { .. } => 0x15,
        StreamKind::KlvAsync => 0x06,
        _ => 0x06,
    }
}

/// Detect the MISB KLV set identity from the Universal Label key prefix of the
/// raw KLV LS bytes.  Binding-neutral: the same 16-byte UL is visible to a C /
/// Python adapter.  Returns `"st0601"` for the ST 0601 UAS Datalink LS UL,
/// else `"unknown"`.
fn klv_set_from_ul(payload: &[u8]) -> String {
    // MISB ST 0601 UAS Datalink LS Universal Label (SMPTE 336M 16-byte key).
    // The first 13 bytes are the canonical ST 0601 designator; bytes 14-16
    // carry the version, which varies, so we match on the stable prefix.
    const ST0601_UL_PREFIX: [u8; 13] = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x0B, 0x01, 0x01, 0x0E, 0x01, 0x03, 0x01, 0x01,
    ];
    if payload.len() >= ST0601_UL_PREFIX.len()
        && payload[..ST0601_UL_PREFIX.len()] == ST0601_UL_PREFIX
    {
        "st0601".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Public alias for tests.
pub fn demux_error_code_pub(e: &tst_core::error::DemuxError) -> String {
    demux_error_code(e)
}

// Umbrella public code for the pilot — every fatal demux error maps to the
// single binding-neutral reject code.
// TODO: distinct public codes when non-strict binding_contract scenarios land.
fn demux_error_code(_e: &tst_core::error::DemuxError) -> String {
    "STRICT_REJECTION".into()
}
