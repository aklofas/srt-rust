//! Cross-binding scenario definitions.
//!
//! Each scenario is a self-contained unit that:
//!  1. Generates a deterministic synthetic input artifact (`.ts` bytes or
//!     similar) into `out_dir/<id>/`.
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
fn drain_mux(mux: &mut Muxer) -> Vec<u8> {
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
        // deterministic output.
        // This generator NEVER reads from testfiles/ or local/ directories.
        let cfg = {
            let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
            prog.add_video(0x1011, VideoCodec::H264);
            let mut b = MuxerConfig::builder();
            b.add_program(prog.build());
            b.build().expect("valid muxer config")
        };
        let mut mux = Muxer::new(cfg).expect("muxer init");

        let video_au = synthetic_h264_idr();
        let pts = Pts90khz::new(0);
        mux.push_video(&video_au, pts, /*key_frame=*/ true)
            .expect("push_video");

        let ts_bytes = drain_mux(&mut mux);
        let digest = sha256_hex(&ts_bytes);

        // Write input artifact (the TS output IS the input for roundtrip verification).
        let artifact_rel = PathBuf::from(self.id()).join("output.ts");
        let artifact_abs = out_dir.join(&artifact_rel);
        write_file(&artifact_abs, &ts_bytes);

        // For a roundtrip scenario the golden holds a single Error-free payload
        // identity event in extensions, and a single Video event in core.
        // The byte-identity check is the sha256 in the Video event's
        // payload_sha256; the adapter rebuilds the TS and compares sha256.
        let core = vec![CoreEvent::Video {
            program: 1,
            pid: 0x1011,
            stream_type: "h264".to_string(),
            pts: 0,
            key: true,
            payload_sha256: digest,
        }];

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
fn synthetic_h264_idr() -> Vec<u8> {
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
    use tst_core::mpegts::demux::event::SamplePayload;
    use tst_core::mpegts::demux::{DemuxEvent, Demuxer};

    let mut demuxer = Demuxer::new();
    let mut events = Vec::new();

    match demuxer.feed(ts_bytes) {
        Ok(()) => {}
        Err(e) => {
            events.push(CoreEvent::Error {
                code: demux_error_code(&e),
            });
            return events;
        }
    }

    demuxer.flush();

    while let Some(ev) = demuxer.next_event() {
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
                            stream_type: format!("{codec:?}").to_lowercase(),
                            pts: pts.as_ticks(),
                            key: random_access_indicator,
                            payload_sha256: sha256_hex(&raw),
                        });
                    }
                    SamplePayload::Audio { codec, frames } => {
                        events.push(CoreEvent::Audio {
                            program: stream.program_number,
                            pid: stream.pid,
                            stream_type: format!("{codec:?}").to_lowercase(),
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
            DemuxEvent::Metadata { stream, kind, .. } => {
                let set = metadata_kind_str(&kind);
                events.push(CoreEvent::Klv {
                    program: stream.program_number,
                    pid: stream.pid,
                    stream_type: stream_kind_str(&stream.kind),
                    set,
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

fn metadata_kind_str(kind: &tst_core::mpegts::demux::event::MetadataKind) -> String {
    use tst_core::mpegts::demux::event::MetadataKind;
    match kind {
        MetadataKind::KlvSyncAuCell { .. } => "sync_au_cell".to_string(),
        MetadataKind::KlvAsync => "async".to_string(),
        MetadataKind::Unknown(_) => "unknown".to_string(),
    }
}

fn stream_kind_str(kind: &tst_core::mpegts::demux::event::StreamKind) -> String {
    use tst_core::mpegts::demux::event::StreamKind;
    match kind {
        StreamKind::KlvSync { .. } => "klv_sync".to_string(),
        StreamKind::KlvAsync => "klv_async".to_string(),
        StreamKind::Video(c) => format!("{c:?}").to_lowercase(),
        StreamKind::Audio(c) => format!("{c:?}").to_lowercase(),
        StreamKind::Subtitle(c) => format!("{c:?}").to_lowercase(),
        StreamKind::Unknown(b) => format!("unknown_0x{b:02x}"),
    }
}

/// Public alias for tests.
pub fn demux_error_code_pub(e: &tst_core::error::DemuxError) -> String {
    demux_error_code(e)
}

fn demux_error_code(e: &tst_core::error::DemuxError) -> String {
    use tst_core::error::DemuxError;
    match e {
        DemuxError::StrictRejection(_) => "STRICT_REJECTION".to_string(),
        DemuxError::Unrecoverable { .. } => "STRICT_REJECTION".to_string(),
        DemuxError::SyncBufExhausted { .. } => "STRICT_REJECTION".to_string(),
        _ => "STRICT_REJECTION".to_string(),
    }
}
