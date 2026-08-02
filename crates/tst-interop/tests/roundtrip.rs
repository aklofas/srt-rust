//! All-profile self-roundtrip: `gen` each canonical profile's synthetic
//! traffic to a temp `.ts` file, then `verify::verify_file` it against
//! that same profile's invariants.
//!
//! This is the zero-third-party-tools harness gate — every profile must
//! generate and verify clean here before any transport/tool cell
//! (later tasks) can be trusted to mean anything.

use tst_interop::{r#gen, profiles, verify};

/// Seconds of synthetic traffic per profile. Long enough to clear the
/// 70%-of-nominal count floors (`verify::NOMINAL_COUNT_SLACK`) with
/// margin, short enough to keep the suite fast.
const SECONDS: f64 = 3.0;

/// `gen::run` profile `name` for [`SECONDS`] to a temp file, `verify_file`
/// it, delete the temp file, and assert the report passed.
fn assert_profile_roundtrips(name: &str) {
    let p = profiles::by_name(name).unwrap_or_else(|| panic!("profile {name} must be registered"));

    let path = std::env::temp_dir().join(format!(
        "tst-interop-roundtrip-{name}-{}.ts",
        std::process::id()
    ));

    let result =
        r#gen::run(p, SECONDS, &path).and_then(|()| verify::verify_file(&path, p, SECONDS));
    let _ = std::fs::remove_file(&path);

    let report = result.unwrap_or_else(|e| panic!("{name}: gen/verify IO error: {e}"));
    assert!(
        report.pass,
        "{name}: verify failures: {:?}",
        report.failures
    );
}

#[test]
fn baseline_roundtrips() {
    assert_profile_roundtrips("baseline");
}

#[test]
fn klv_sync_roundtrips() {
    assert_profile_roundtrips("klv-sync");
}

#[test]
fn misp_roundtrips() {
    assert_profile_roundtrips("misp");
}

#[test]
fn h265_klv_roundtrips() {
    assert_profile_roundtrips("h265-klv");
}

#[test]
fn av1_klv_a_roundtrips() {
    assert_profile_roundtrips("av1-klv-a");
}

#[test]
fn av1_klv_b_roundtrips() {
    assert_profile_roundtrips("av1-klv-b");
}

#[test]
fn h266_klv_roundtrips() {
    assert_profile_roundtrips("h266-klv");
}

#[test]
fn audio_roundtrips() {
    assert_profile_roundtrips("audio");
}

#[test]
fn two_program_roundtrips() {
    assert_profile_roundtrips("two-program");
}

#[test]
fn pcr_tight_roundtrips() {
    assert_profile_roundtrips("pcr-tight");
}

#[test]
fn pcr_sparse_roundtrips() {
    assert_profile_roundtrips("pcr-sparse");
}

#[test]
fn pts_rollover_roundtrips() {
    assert_profile_roundtrips("pts-rollover");
}

/// Drift guard: every profile in the registry must have a dedicated test
/// above. Without this, a renamed/added/removed profile would silently
/// under-cover (or over-claim) the roundtrip gate instead of failing loudly.
#[test]
fn every_registered_profile_has_a_dedicated_roundtrip_test() {
    let covered = [
        "baseline",
        "klv-sync",
        "misp",
        "h265-klv",
        "av1-klv-a",
        "av1-klv-b",
        "h266-klv",
        "audio",
        "two-program",
        "pcr-tight",
        "pcr-sparse",
        "pts-rollover",
    ];
    let mut expected: Vec<&str> = profiles::all().iter().map(|p| p.name).collect();
    expected.sort_unstable();
    let mut covered_sorted = covered.to_vec();
    covered_sorted.sort_unstable();
    assert_eq!(
        covered_sorted, expected,
        "roundtrip.rs must have exactly one #[test] per registered profile"
    );
}
