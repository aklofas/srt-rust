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
/// normalised `CoreEvent`s.  The shared normaliser surfaces NonConformant
/// events as `CoreEvent::Error` alongside media events, so malformed-input
/// `demux` scenarios assert their diagnostic codes through this same path.
fn run_demux(input: &[u8]) -> Vec<CoreEvent> {
    demux_to_core_events(input)
}

/// Run a `roundtrip` scenario: re-run the generator's single-source-of-truth
/// mux recipe and assert byte-identity against the committed artifact.
///
/// The roundtrip golden has `core: []` and an `extensions.output_sha256` hex
/// digest of the whole TS output. This asserts:
///  1. The freshly-muxed bytes are byte-identical to the committed `output.ts`.
///  2. Their sha256 equals the golden's `extensions.output_sha256`.
///
/// `committed_output_ts` is the committed `output.ts` artifact (the scenario's
/// `input`). Returns `core: []` (roundtrip carries no media events).
fn run_roundtrip(committed_output_ts: &[u8], golden: &Golden) -> Vec<CoreEvent> {
    use sha2::{Digest, Sha256};
    use tst_integration::scenarios::video_roundtrip_ts_bytes;

    // Re-run the generator's exact recipe — no hand-retyped mux.
    let fresh = video_roundtrip_ts_bytes();

    // 1. Byte-identity against the committed artifact.
    assert_eq!(
        fresh, committed_output_ts,
        "roundtrip TS output changed: fresh mux bytes differ from committed output.ts"
    );

    // 2. sha256 against the golden's stored output digest.
    let digest: String = {
        use std::fmt::Write;
        Sha256::digest(&fresh)
            .iter()
            .fold(String::new(), |mut s, b| {
                write!(s, "{b:02x}").unwrap();
                s
            })
    };
    let expected = golden
        .extensions
        .get("output_sha256")
        .and_then(|v| v.as_str())
        .expect("roundtrip golden must carry extensions.output_sha256");
    assert_eq!(
        digest, expected,
        "roundtrip sha256 mismatch: TS output changed"
    );

    // Roundtrip scenarios carry no media events.
    vec![]
}

/// Run a `binding_contract` scenario: exercise the non-media contract.
///
/// For `strict-rejection`: feed garbage bytes to a demuxer and assert the
/// error maps to the stable public code `STRICT_REJECTION`.  Also verify that
/// dropping the demuxer after an error is idempotent (no panic).
///
/// For `malformed-psi-strict`: feed a TS with a valid PAT but corrupted PMT
/// CRC to a `StrictMode::Full` demuxer. The `PsiChecksumMismatch` NonConformant
/// escalates to `DemuxError::StrictRejection`, which maps to `"STRICT_REJECTION"`.
fn run_binding_contract(id: &str, input: &[u8]) -> Vec<CoreEvent> {
    use tst_core::mpegts::demux::Demuxer;
    use tst_integration::scenarios::demux_error_code_pub;

    match id {
        "strict-rejection" => {
            // Garbage bytes: no 0x47 sync byte → Unrecoverable → STRICT_REJECTION.
            let mut demuxer = Demuxer::new();
            let result = demuxer.feed(input);
            let code = match &result {
                Err(e) => demux_error_code_pub(e),
                Ok(()) => panic!("expected feed to return Err on garbage input, got Ok"),
            };
            drop(demuxer);
            vec![CoreEvent::Error { code }]
        }
        "malformed-psi-strict" => {
            use tst_core::mpegts::demux::{DemuxerBuilder, StrictMode};
            // Use StrictMode::Full so PsiChecksumMismatch → StrictRejection.
            let mut demuxer = DemuxerBuilder::new().strict(StrictMode::Full).build();
            let result = demuxer.feed(input);
            let code = match &result {
                Err(e) => demux_error_code_pub(e),
                Ok(()) => {
                    // Unreachable for this input — the PMT CRC mismatch is
                    // detected within feed()'s packet loop, which always
                    // returns Err(StrictRejection) before returning Ok. A
                    // reached Ok means the malformation didn't trigger rejection.
                    panic!(
                        "malformed-psi-strict: expected StrictRejection error from feed, got Ok"
                    );
                }
            };
            drop(demuxer);
            vec![CoreEvent::Error { code }]
        }
        other => panic!("unknown binding_contract scenario: {other}"),
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
