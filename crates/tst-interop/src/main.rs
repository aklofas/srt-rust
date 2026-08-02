use std::env;
use std::path::PathBuf;

use tst_interop::r#gen;
use tst_interop::profiles;
use tst_interop::verify;

fn usage() -> String {
    "usage: tst-interop <subcommand> [options...]

Subcommands:
  gen       Generate synthetic test fixtures
  send      Send test data to endpoint
  recv      Receive test data from endpoint
  verify    Verify interop test results
  proxy     Proxy between endpoints
  report    Generate interop report

Options:
  -h, --help   Show this help message"
        .to_string()
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("{}", usage());
        std::process::exit(0);
    }

    let subcommand = &args[1];

    match subcommand.as_str() {
        "-h" | "--help" => {
            println!("{}", usage());
            std::process::exit(0);
        }
        "gen" => run_gen(&args[2..]),
        "send" => {
            eprintln!("send: not implemented");
            std::process::exit(2);
        }
        "recv" => {
            eprintln!("recv: not implemented");
            std::process::exit(2);
        }
        "verify" => run_verify(&args[2..]),
        "proxy" => {
            eprintln!("proxy: not implemented");
            std::process::exit(2);
        }
        "report" => {
            eprintln!("report: not implemented");
            std::process::exit(2);
        }
        _ => {
            eprintln!("Unknown subcommand: {}", subcommand);
            println!("{}", usage());
            std::process::exit(2);
        }
    }
}

/// `gen --profile NAME --seconds N --out PATH`
///
/// Generates `N` seconds of profile `NAME`'s synthetic MPEG-TS/KLV traffic
/// (offline pacing, no transport) and writes it to `PATH`. Exits 0 on
/// success, 2 on usage/IO error.
fn run_gen(args: &[String]) -> ! {
    let mut profile: Option<String> = None;
    let mut seconds: Option<f64> = None;
    let mut out: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--profile" => {
                profile = args.get(i + 1).cloned();
                i += 2;
            }
            "--seconds" => {
                seconds = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--out" => {
                out = args.get(i + 1).map(PathBuf::from);
                i += 2;
            }
            other => {
                eprintln!("gen: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let profile_name = profile.unwrap_or_else(|| {
        eprintln!("gen: --profile is required");
        std::process::exit(2);
    });
    let seconds = seconds.unwrap_or_else(|| {
        eprintln!("gen: --seconds is required (and must be a number)");
        std::process::exit(2);
    });
    let out = out.unwrap_or_else(|| {
        eprintln!("gen: --out is required");
        std::process::exit(2);
    });
    let p = profiles::by_name(&profile_name).unwrap_or_else(|| {
        eprintln!("gen: unknown profile: {profile_name}");
        std::process::exit(2);
    });

    if let Err(e) = r#gen::run(p, seconds, &out) {
        eprintln!("gen: {e}");
        std::process::exit(2);
    }

    eprintln!(
        "gen: wrote {seconds}s of {profile_name} to {}",
        out.display()
    );
    std::process::exit(0);
}

/// `verify --file F --expect PROFILE --seconds N [--json OUT]`
///
/// Demuxes `F` and checks it against `PROFILE`'s invariants for an
/// `N`-second capture. Exits 0 on pass, 1 on fail, 2 on usage/IO error.
/// `--json OUT` additionally writes the full `VerifyReport` as JSON to
/// `OUT` (or stdout, if `OUT` is `-`).
fn run_verify(args: &[String]) -> ! {
    let mut file: Option<PathBuf> = None;
    let mut expect: Option<String> = None;
    let mut seconds: Option<f64> = None;
    let mut json_out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                file = args.get(i + 1).map(PathBuf::from);
                i += 2;
            }
            "--expect" => {
                expect = args.get(i + 1).cloned();
                i += 2;
            }
            "--seconds" => {
                seconds = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 2;
            }
            "--json" => {
                json_out = args.get(i + 1).cloned();
                i += 2;
            }
            other => {
                eprintln!("verify: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let file = file.unwrap_or_else(|| {
        eprintln!("verify: --file is required");
        std::process::exit(2);
    });
    let expect = expect.unwrap_or_else(|| {
        eprintln!("verify: --expect is required");
        std::process::exit(2);
    });
    let seconds = seconds.unwrap_or_else(|| {
        eprintln!("verify: --seconds is required (and must be a number)");
        std::process::exit(2);
    });
    let profile = profiles::by_name(&expect).unwrap_or_else(|| {
        eprintln!("verify: unknown profile: {expect}");
        std::process::exit(2);
    });

    let report = verify::verify_file(&file, profile, seconds).unwrap_or_else(|e| {
        eprintln!("verify: {e}");
        std::process::exit(2);
    });

    if report.pass {
        eprintln!("verify: PASS ({expect})");
    } else {
        eprintln!("verify: FAIL ({expect}): {}", report.failures.join("; "));
    }

    if let Some(target) = json_out {
        let json = serde_json::to_string_pretty(&report).expect("VerifyReport always serializes");
        if target == "-" {
            println!("{json}");
        } else if let Err(e) = std::fs::write(&target, json) {
            eprintln!("verify: failed to write {target}: {e}");
            std::process::exit(2);
        }
    }

    std::process::exit(if report.pass { 0 } else { 1 });
}
