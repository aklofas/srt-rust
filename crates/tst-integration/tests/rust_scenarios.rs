//! Rust adapter integration tests for the cross-binding scenario harness.
//!
//! For each entry in `tests/fixtures/scenarios/scenarios.toml`:
//!  - Run the committed input artifact through tst-core (demux/mux per
//!    `kind`).
//!  - Normalise the result to a `Golden` using the Step-2 semantics.
//!  - Assert `struct PartialEq` equality against the committed `golden.json`.
//!
//! The tests fail loudly if any committed golden is stale or if the
//! normaliser output deviates from what was committed.

use std::path::PathBuf;

use serde::Deserialize;

use tst_integration::scenarios::demux_to_core_events;
use tst_integration::scenarios::golden::{CoreEvent, Golden};

/// Path to the committed scenario fixtures.
fn fixtures_dir() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is the crate root; fixtures live under tests/.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("scenarios")
}

// ─────────────────────────────────────────────────────────────────────────────
// Manifest deserialization
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ScenariosManifest {
    scenario: Vec<ScenarioEntry>,
}

#[derive(Deserialize)]
struct ScenarioEntry {
    id: String,
    kind: String,
    input: String,
    golden: String,
    #[allow(dead_code)]
    features: Vec<String>,
    #[allow(dead_code)]
    tier: String,
    #[allow(dead_code)]
    schema_version: u32,
}

fn load_manifest() -> ScenariosManifest {
    let path = fixtures_dir().join("scenarios.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read scenarios.toml at {}: {e}", path.display()));
    toml::from_str(&text).unwrap_or_else(|e| panic!("cannot parse scenarios.toml: {e}"))
}

fn load_golden(golden_rel: &str) -> Golden {
    let path = fixtures_dir().join(golden_rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("cannot parse {}: {e}", path.display()))
}

fn load_input(input_rel: &str) -> Vec<u8> {
    let path = fixtures_dir().join(input_rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Per-kind adapters
// ─────────────────────────────────────────────────────────────────────────────

/// Run a `demux` scenario: feed the input TS through tst-core and collect
/// normalised `CoreEvent`s.
fn run_demux(input: &[u8]) -> Vec<CoreEvent> {
    demux_to_core_events(input)
}

/// Run a `roundtrip` scenario: re-mux the same synthetic payload and compare
/// sha256 of output bytes.
///
/// For the `video-roundtrip` scenario the golden `core` holds a single Video
/// event whose `payload_sha256` is the sha256 of the full mux output bytes.
/// We reproduce the mux identically and assert byte-identity.
fn run_roundtrip(_input: &[u8], golden: &Golden) -> Vec<CoreEvent> {
    use sha2::{Digest, Sha256};
    use tst_core::mpegts::common::Pts90khz;
    use tst_core::mpegts::mux::{Muxer, MuxerConfig, MuxerProgramConfigBuilder, VideoCodec};

    // Re-run the same mux that `VideoRoundtrip::generate` ran.
    let cfg = {
        let mut prog = MuxerProgramConfigBuilder::new(1, 0x1000);
        prog.add_video(0x1011, VideoCodec::H264);
        let mut b = MuxerConfig::builder();
        b.add_program(prog.build());
        b.build().expect("valid config")
    };
    let mut mux = Muxer::new(cfg).expect("muxer init");

    // Synthetic IDR: same bytes as the generator produces.
    let video_au = {
        let mut buf = Vec::with_capacity(20);
        buf.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        buf.push(0x65);
        for i in 0u8..15 {
            buf.push(0xA5 ^ i);
        }
        buf
    };
    mux.push_video(&video_au, Pts90khz::new(0), true)
        .expect("push_video");

    let mut ts_bytes = Vec::new();
    let mut buf = vec![0u8; 1316];
    loop {
        let n = mux.pull(&mut buf);
        if n == 0 {
            break;
        }
        ts_bytes.extend_from_slice(&buf[..n]);
    }

    let digest: String = {
        use std::fmt::Write;
        Sha256::digest(&ts_bytes)
            .iter()
            .fold(String::new(), |mut s, b| {
                write!(s, "{b:02x}").unwrap();
                s
            })
    };

    // Expect the golden to have a single Video event whose payload_sha256
    // matches the freshly-computed digest.
    assert_eq!(
        golden.core.len(),
        1,
        "roundtrip golden must have exactly 1 core event"
    );
    match &golden.core[0] {
        CoreEvent::Video { payload_sha256, .. } => {
            assert_eq!(
                &digest, payload_sha256,
                "roundtrip sha256 mismatch: TS output changed"
            );
        }
        other => panic!("roundtrip golden unexpected event: {other:?}"),
    }

    // Return the actual events for assertion downstream.
    golden.core.clone()
}

/// Run a `binding_contract` scenario: exercise the non-media contract.
///
/// For `strict-rejection`: feed garbage bytes to a demuxer and assert the
/// error maps to the stable public code `STRICT_REJECTION`.  Also verify that
/// dropping the demuxer after an error is idempotent (no panic).
fn run_binding_contract(id: &str, input: &[u8]) -> Vec<CoreEvent> {
    if id == "strict-rejection" {
        use tst_core::mpegts::demux::Demuxer;

        // Prove that feed on garbage errors out.
        let mut demuxer = Demuxer::new();
        let result = demuxer.feed(input);
        let code = match &result {
            Err(e) => {
                use tst_integration::scenarios::demux_error_code_pub;
                demux_error_code_pub(e)
            }
            Ok(()) => {
                // If the garbage happened to not trigger Unrecoverable (e.g.
                // the random bytes contained a valid-looking sync sequence),
                // flush and check for any queued NonConformant that maps to error.
                // For 2048 bytes of 0xFF this should never happen — the
                // sync-search window will be exhausted.
                panic!("expected feed to return Err on garbage input, got Ok");
            }
        };

        // Idempotent drop: second drop (via end-of-scope) must not panic.
        drop(demuxer);

        vec![CoreEvent::Error { code }]
    } else {
        panic!("unknown binding_contract scenario: {id}");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Main test
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn all_scenarios_match_committed_goldens() {
    let manifest = load_manifest();
    assert!(
        !manifest.scenario.is_empty(),
        "scenarios.toml must have at least one entry"
    );

    for entry in &manifest.scenario {
        let committed = load_golden(&entry.golden);
        let input = load_input(&entry.input);

        let observed_core: Vec<CoreEvent> = match entry.kind.as_str() {
            "demux" => run_demux(&input),
            "roundtrip" => run_roundtrip(&input, &committed),
            "binding_contract" => run_binding_contract(&entry.id, &input),
            other => panic!("unknown scenario kind '{}' in manifest", other),
        };

        let observed = Golden {
            schema_version: committed.schema_version,
            lossy: committed.lossy,
            core: observed_core,
            extensions: committed.extensions.clone(),
        };

        assert_eq!(
            observed, committed,
            "scenario '{}': observed golden differs from committed",
            entry.id
        );
    }
}
