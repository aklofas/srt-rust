//! CLI argument-parsing robustness tests, general (not proxy-specific —
//! see `tests/proxy.rs`'s `stats_json_missing_value_exits_with_usage_error`
//! for the original instance of this regression class, which this file
//! extends to `gen`/`send`/`recv`/`verify`/`report`).
//!
//! Every value-taking flag across these subcommands used to fetch its
//! value with a bare `args.get(i + 1)` (see `main.rs`'s `require_value`
//! doc comment). A flag given with NO value that's immediately followed
//! by ANOTHER recognized flag silently consumed that flag's own name as
//! if it were the first flag's value, then desynced every argument
//! after it — the user saw a misleading "unknown argument: <some later
//! token>" error instead of anything naming the flag that actually had
//! no value. A flag simply positioned as the very last, unfollowed
//! token on the command line does NOT reproduce this — every flag
//! tested here already has a post-loop "is required" check that catches
//! that degenerate case reasonably even under the old bug — so each
//! test below deliberately follows the value-less flag with another
//! real flag name, the one shape that actually distinguishes the fixed
//! behavior from the bug.
//!
//! Drives the real built CLI binary as a subprocess (the only way to
//! exercise `main.rs`'s argument loop directly — it calls
//! `std::process::exit`, so it can't be unit-tested in-process).

use std::process::Command;

#[test]
fn report_merge_cells_dir_followed_by_another_flag_names_cells_dir() {
    let output = Command::new(env!("CARGO_BIN_EXE_tst-interop"))
        .args([
            "report",
            "merge",
            "--cells-dir",
            "--expectations",
            "expectations.toml",
        ])
        .output()
        .expect("spawn tst-interop binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing --cells-dir value must exit 2, stderr: {stderr}"
    );
    assert!(
        stderr.contains("cells-dir"),
        "usage error should name the flag that's actually missing a value, got: {stderr}"
    );
    assert!(
        !stderr.contains("unknown argument"),
        "must not misreport this as an unrecognized argument, got: {stderr}"
    );
}

#[test]
fn send_profile_followed_by_another_flag_names_profile() {
    let output = Command::new(env!("CARGO_BIN_EXE_tst-interop"))
        .args(["send", "--profile", "--url", "udp://127.0.0.1:1"])
        .output()
        .expect("spawn tst-interop binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing --profile value must exit 2, stderr: {stderr}"
    );
    assert!(
        stderr.contains("profile"),
        "usage error should name the flag that's actually missing a value, got: {stderr}"
    );
    assert!(
        !stderr.contains("unknown argument"),
        "must not misreport this as an unrecognized argument, got: {stderr}"
    );
}

#[test]
fn verify_file_followed_by_another_flag_names_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_tst-interop"))
        .args(["verify", "--file", "--expect", "baseline"])
        .output()
        .expect("spawn tst-interop binary");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing --file value must exit 2, stderr: {stderr}"
    );
    assert!(
        stderr.contains("file"),
        "usage error should name the flag that's actually missing a value, got: {stderr}"
    );
    assert!(
        !stderr.contains("unknown argument"),
        "must not misreport this as an unrecognized argument, got: {stderr}"
    );
}
