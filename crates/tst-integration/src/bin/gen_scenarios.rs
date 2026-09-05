//! Generate or check cross-binding scenario fixtures.
//!
//! Default mode: iterate all scenarios, write input artifacts and
//! `golden.json` files under the crate-local
//! `crates/tst-integration/tests/fixtures/scenarios/<id>/` (resolved from
//! `CARGO_MANIFEST_DIR`), and regenerate
//! `crates/tst-integration/tests/fixtures/scenarios/scenarios.toml`.
//! This crate-local path — not a workspace-root `tests/fixtures/scenarios` — is
//! the canonical location all binding adapters (Rust/C/Python) resolve against.
//!
//! `--check` mode: regenerate into a temp dir, diff the manifest and every
//! golden by content (sha256), exit non-zero with a clear message on any
//! drift.
//!
//! # Synthetic data only
//!
//! This binary NEVER reads from `testfiles/`, any `local/` directory, or any
//! real corpus.  All scenario generators produce purely synthetic inputs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use tst_integration::scenarios::{all_scenarios, golden::Golden};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let check_mode = args.iter().any(|a| a == "--check");

    // Locate the workspace root relative to this binary's location.
    // `CARGO_MANIFEST_DIR` is set during `cargo run`; fall back to CWD.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().expect("cwd"));
    let fixtures_dir = manifest_dir
        .join("tests")
        .join("fixtures")
        .join("scenarios");

    if check_mode {
        run_check(&fixtures_dir);
    } else {
        run_generate(&fixtures_dir);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Manifest types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct ScenariosManifest {
    scenario: Vec<ScenarioEntry>,
}

#[derive(Serialize, Deserialize)]
struct ScenarioEntry {
    id: String,
    kind: String,
    input: String,
    golden: String,
    features: Vec<String>,
    tier: String,
    schema_version: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Generate mode
// ─────────────────────────────────────────────────────────────────────────────

fn run_generate(fixtures_dir: &Path) {
    std::fs::create_dir_all(fixtures_dir).expect("create fixtures dir");

    let scenarios = all_scenarios();
    let mut entries: Vec<ScenarioEntry> = Vec::new();

    for scenario in &scenarios {
        let (input_rel, golden) = scenario.generate(fixtures_dir);

        // Write golden.json.
        let golden_rel = PathBuf::from(scenario.id()).join("golden.json");
        let golden_abs = fixtures_dir.join(&golden_rel);
        let golden_json = serde_json::to_string_pretty(&golden).expect("serialise golden");
        std::fs::write(&golden_abs, &golden_json).expect("write golden.json");

        entries.push(ScenarioEntry {
            id: scenario.id().to_string(),
            kind: scenario.kind().to_string(),
            input: input_rel.to_string_lossy().replace('\\', "/"),
            golden: golden_rel.to_string_lossy().replace('\\', "/"),
            features: scenario.features().iter().map(|s| s.to_string()).collect(),
            tier: scenario.tier().to_string(),
            schema_version: golden.schema_version,
        });

        println!("  generated: {}", scenario.id());
    }

    // Write scenarios.toml manifest.
    let manifest = ScenariosManifest { scenario: entries };
    let toml_str = toml::to_string(&manifest).expect("serialise manifest");
    let manifest_path = fixtures_dir.join("scenarios.toml");
    std::fs::write(&manifest_path, &toml_str).expect("write scenarios.toml");

    println!("scenarios.toml written to {}", manifest_path.display());
}

// ─────────────────────────────────────────────────────────────────────────────
// Check mode
// ─────────────────────────────────────────────────────────────────────────────

fn run_check(fixtures_dir: &Path) {
    // Regenerate into a temp dir.
    let tmp = std::env::temp_dir().join(format!("tst-integration-check-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("create temp dir");

    let scenarios = all_scenarios();
    let mut fresh_entries: Vec<ScenarioEntry> = Vec::new();
    let mut drift_found = false;

    for scenario in &scenarios {
        let (input_rel, golden) = scenario.generate(&tmp);

        let golden_rel = PathBuf::from(scenario.id()).join("golden.json");
        let fresh_golden_json = serde_json::to_string_pretty(&golden).expect("serialise golden");

        // Compare golden.json content.
        let committed_golden_path = fixtures_dir.join(&golden_rel);
        if committed_golden_path.exists() {
            let committed =
                std::fs::read_to_string(&committed_golden_path).expect("read committed golden");
            // Normalise by round-tripping through serde_json to ignore whitespace diff.
            let committed_parsed: Golden =
                serde_json::from_str(&committed).expect("parse committed golden");
            let fresh_parsed: Golden =
                serde_json::from_str(&fresh_golden_json).expect("parse fresh golden");
            if committed_parsed != fresh_parsed {
                eprintln!(
                    "STALE: golden differs for scenario '{}'\n  committed: {}\n  fresh:     {}",
                    scenario.id(),
                    committed_golden_path.display(),
                    tmp.join(&golden_rel).display()
                );
                // Dump a brief diff of the JSON representations.
                eprintln!("  committed JSON:\n{committed}");
                eprintln!("  fresh JSON:\n{fresh_golden_json}");
                drift_found = true;
            }
        } else {
            eprintln!(
                "MISSING: committed golden not found for scenario '{}': {}",
                scenario.id(),
                committed_golden_path.display()
            );
            drift_found = true;
        }

        fresh_entries.push(ScenarioEntry {
            id: scenario.id().to_string(),
            kind: scenario.kind().to_string(),
            input: input_rel.to_string_lossy().replace('\\', "/"),
            golden: golden_rel.to_string_lossy().replace('\\', "/"),
            features: scenario.features().iter().map(|s| s.to_string()).collect(),
            tier: scenario.tier().to_string(),
            schema_version: golden.schema_version,
        });
    }

    // Compare scenarios.toml manifest.
    let committed_manifest_path = fixtures_dir.join("scenarios.toml");
    if committed_manifest_path.exists() {
        let fresh_manifest = ScenariosManifest {
            scenario: fresh_entries,
        };
        let fresh_toml = toml::to_string(&fresh_manifest).expect("serialise manifest");
        let committed_toml =
            std::fs::read_to_string(&committed_manifest_path).expect("read committed manifest");
        // Compare by sha256 of normalised TOML (round-trip through toml to drop
        // whitespace differences).
        let committed_parsed: ScenariosManifest =
            toml::from_str(&committed_toml).expect("parse committed manifest");
        let committed_re = toml::to_string(&committed_parsed).expect("re-serialise");
        if sha256_hex(fresh_toml.as_bytes()) != sha256_hex(committed_re.as_bytes()) {
            eprintln!(
                "STALE: scenarios.toml differs\n  committed: {}\nRun `cargo run -p tst-integration --bin gen-scenarios` to update.",
                committed_manifest_path.display()
            );
            drift_found = true;
        }
    } else {
        eprintln!(
            "MISSING: committed scenarios.toml not found at {}\nRun `cargo run -p tst-integration --bin gen-scenarios` to create it.",
            committed_manifest_path.display()
        );
        drift_found = true;
    }

    // Cleanup temp dir.
    let _ = std::fs::remove_dir_all(&tmp);

    if drift_found {
        eprintln!(
            "\nScenario fixtures are stale.  Run:\n  cargo run -p tst-integration --bin gen-scenarios\nto regenerate them."
        );
        std::process::exit(1);
    }

    println!(
        "check-scenarios: all {} scenarios up to date.",
        scenarios.len()
    );
}

fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    digest.iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write;
        write!(s, "{b:02x}").unwrap();
        s
    })
}
