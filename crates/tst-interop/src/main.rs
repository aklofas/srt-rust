use std::env;
use std::path::PathBuf;

use tst_interop::r#gen;
use tst_interop::profiles;
use tst_interop::recv;
use tst_interop::send;
use tst_interop::serve;
use tst_interop::verify;

fn usage() -> String {
    "usage: tst-interop <subcommand> [options...]

Subcommands:
  gen       Generate synthetic test fixtures
  send      Send test data to endpoint (hls:// and rtsp:// URLs BIND and
            serve instead of connecting — see `send`'s own doc comment)
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
        "send" => run_send(&args[2..]),
        "recv" => run_recv(&args[2..]),
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

/// `send --profile NAME --url URL --seconds N [--json OUT]`
///
/// Builds a live transport from `URL` and pushes `N` seconds of profile
/// `NAME`'s synthetic MPEG-TS/KLV traffic through it, paced to real
/// time. Exits 0 on success, 2 on usage/transport error. `--json OUT`
/// additionally writes the sent-side `CellMetrics` as JSON to `OUT`
/// (or stdout, if `OUT` is `-`).
///
/// `hls://`/`hlss://` and `rtsp://`/`rtsps://` URLs are serve (BIND)
/// modes, not connect modes: this subcommand binds a real HLS HTTP
/// server / RTSP server at the URL's host:port and waits for a peer to
/// pull, instead of connecting out to one (see `tst_interop::serve`'s
/// doc comment for why these two schemes work this way). `--json` is
/// ignored for these two schemes — there is no sent-side `CellMetrics`
/// to write (no wire-level Transport tee; see `serve.rs`'s scope notes).
fn run_send(args: &[String]) -> ! {
    let mut profile: Option<String> = None;
    let mut url: Option<String> = None;
    let mut seconds: Option<f64> = None;
    let mut json_out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--profile" => {
                profile = args.get(i + 1).cloned();
                i += 2;
            }
            "--url" => {
                url = args.get(i + 1).cloned();
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
                eprintln!("send: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let profile_name = profile.unwrap_or_else(|| {
        eprintln!("send: --profile is required");
        std::process::exit(2);
    });
    let url = url.unwrap_or_else(|| {
        eprintln!("send: --url is required");
        std::process::exit(2);
    });
    let seconds = seconds.unwrap_or_else(|| {
        eprintln!("send: --seconds is required (and must be a number)");
        std::process::exit(2);
    });
    let p = profiles::by_name(&profile_name).unwrap_or_else(|| {
        eprintln!("send: unknown profile: {profile_name}");
        std::process::exit(2);
    });

    // hls:// / rtsp:// (+ TLS variants) are serve (bind) modes — branch
    // out before the connect-side transport path below.
    if let Some(scheme) = serve::serve_scheme_of(&url) {
        let result = match scheme {
            serve::ServeScheme::Hls => serve::run_hls_url(p, &url, seconds),
            serve::ServeScheme::Rtsp => serve::run_rtsp_url(p, &url, seconds),
        };
        if let Err(e) = result {
            eprintln!("send: {e}");
            std::process::exit(2);
        }
        eprintln!("send: served {seconds}s of {profile_name} at {url}");
        std::process::exit(0);
    }

    let metrics = send::run(p, &url, seconds, json_out.as_deref()).unwrap_or_else(|e| {
        eprintln!("send: {e}");
        std::process::exit(2);
    });

    eprintln!(
        "send: pushed {} video AUs, {} klv records to {url}",
        metrics.video_aus, metrics.klv_records
    );
    std::process::exit(0);
}

/// `recv --url URL --expect PROFILE --seconds N [--json OUT]`
///
/// Builds a live transport from `URL` and receives `N` seconds of
/// traffic from it, checking the result against `PROFILE`'s invariants.
/// Exits 0 on pass, 1 on fail, 2 on usage/transport error. `--json OUT`
/// additionally writes the full `VerifyReport` as JSON to `OUT` (or
/// stdout, if `OUT` is `-`).
fn run_recv(args: &[String]) -> ! {
    let mut url: Option<String> = None;
    let mut expect: Option<String> = None;
    let mut seconds: Option<f64> = None;
    let mut json_out: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                url = args.get(i + 1).cloned();
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
                eprintln!("recv: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let url = url.unwrap_or_else(|| {
        eprintln!("recv: --url is required");
        std::process::exit(2);
    });
    let expect = expect.unwrap_or_else(|| {
        eprintln!("recv: --expect is required");
        std::process::exit(2);
    });
    let seconds = seconds.unwrap_or_else(|| {
        eprintln!("recv: --seconds is required (and must be a number)");
        std::process::exit(2);
    });
    let profile = profiles::by_name(&expect).unwrap_or_else(|| {
        eprintln!("recv: unknown profile: {expect}");
        std::process::exit(2);
    });

    let report = recv::run(&url, profile, seconds, json_out.as_deref()).unwrap_or_else(|e| {
        eprintln!("recv: {e}");
        std::process::exit(2);
    });

    if report.pass {
        eprintln!("recv: PASS ({expect})");
    } else {
        eprintln!("recv: FAIL ({expect}): {}", report.failures.join("; "));
    }

    std::process::exit(if report.pass { 0 } else { 1 });
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
