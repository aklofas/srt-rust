use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use tst_interop::cli;
use tst_interop::r#gen;
use tst_interop::impair::ImpairConfig;
use tst_interop::profiles;
use tst_interop::proxy;
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
  proxy     UDP impairment relay (loss/dup/reorder/jitter/scheduled outage)
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
        "proxy" => run_proxy(&args[2..]),
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
                seconds = args.get(i + 1).and_then(|s| cli::parse_seconds(s));
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
        eprintln!("gen: --seconds is required and must be a finite, positive number");
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
                seconds = args.get(i + 1).and_then(|s| cli::parse_seconds(s));
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
        eprintln!("send: --seconds is required and must be a finite, positive number");
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
                seconds = args.get(i + 1).and_then(|s| cli::parse_seconds(s));
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
        eprintln!("recv: --seconds is required and must be a finite, positive number");
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
                seconds = args.get(i + 1).and_then(|s| cli::parse_seconds(s));
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
        eprintln!("verify: --seconds is required and must be a finite, positive number");
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

/// `proxy --listen ADDR --forward ADDR [--loss PCT] [--dup PCT]
/// [--reorder PCT,HOLD_MS] [--jitter MS] [--seed N]
/// [--outage period=DUR,dur=DUR] [--stats-json PATH] [--run-seconds N]`
///
/// Binds a UDP impairment relay at `--listen` (an ephemeral `:0` port is
/// printed as `{"listening": "..."}` on stdout as soon as it's bound —
/// see `proxy::run`'s doc comment) and relays to `--forward` under the
/// configured impairment. Every impairment knob defaults to fully
/// transparent (`ImpairConfig::default()`) when its flag is omitted.
/// `--run-seconds` bounds how long the relay runs before exiting (the
/// default, omitted, runs until the process is killed — this
/// subcommand's normal long-running CLI mode). Exits 0 on a clean
/// finish, 2 on a usage or IO error.
fn run_proxy(args: &[String]) -> ! {
    let mut listen: Option<SocketAddr> = None;
    let mut forward: Option<SocketAddr> = None;
    let mut loss_pct = 0.0f64;
    let mut dup_pct = 0.0f64;
    let mut reorder_pct = 0.0f64;
    let mut reorder_hold = 0u32;
    let mut jitter_ms_max = 0u32;
    let mut seed = 0u64;
    let mut outage_period_s: Option<u64> = None;
    let mut outage_dur_s = 0u64;
    let mut stats_json: Option<PathBuf> = None;
    let mut run_seconds: Option<u64> = None;

    let bad_arg = |flag: &str, expected: &str| -> ! {
        eprintln!("proxy: --{flag} must be {expected}");
        std::process::exit(2);
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                listen = Some(
                    args.get(i + 1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| bad_arg("listen", "a socket address (host:port)")),
                );
                i += 2;
            }
            "--forward" => {
                forward = Some(
                    args.get(i + 1)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(|| bad_arg("forward", "a socket address (host:port)")),
                );
                i += 2;
            }
            "--loss" => {
                loss_pct = args
                    .get(i + 1)
                    .and_then(|s| proxy::parse_percent(s))
                    .unwrap_or_else(|| bad_arg("loss", "a percent in 0..=100"));
                i += 2;
            }
            "--dup" => {
                dup_pct = args
                    .get(i + 1)
                    .and_then(|s| proxy::parse_percent(s))
                    .unwrap_or_else(|| bad_arg("dup", "a percent in 0..=100"));
                i += 2;
            }
            "--reorder" => {
                let (pct, hold) = args
                    .get(i + 1)
                    .and_then(|s| proxy::parse_reorder(s))
                    .unwrap_or_else(|| bad_arg("reorder", "PCT,HOLD_MS (e.g. 1,200)"));
                reorder_pct = pct;
                reorder_hold = hold;
                i += 2;
            }
            "--jitter" => {
                jitter_ms_max = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| bad_arg("jitter", "a non-negative integer (milliseconds)"));
                i += 2;
            }
            "--seed" => {
                seed = args
                    .get(i + 1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| bad_arg("seed", "a non-negative integer"));
                i += 2;
            }
            "--outage" => {
                let (period, dur) = args
                    .get(i + 1)
                    .and_then(|s| proxy::parse_outage(s))
                    .unwrap_or_else(|| {
                        bad_arg("outage", "period=DUR,dur=DUR (e.g. period=6h,dur=90s)")
                    });
                outage_period_s = Some(period);
                outage_dur_s = dur;
                i += 2;
            }
            "--stats-json" => {
                stats_json = args.get(i + 1).map(PathBuf::from);
                i += 2;
            }
            "--run-seconds" => {
                run_seconds = Some(
                    args.get(i + 1)
                        .and_then(|s| proxy::parse_run_seconds(s))
                        .unwrap_or_else(|| bad_arg("run-seconds", "a positive integer")),
                );
                i += 2;
            }
            other => {
                eprintln!("proxy: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    let listen = listen.unwrap_or_else(|| {
        eprintln!("proxy: --listen is required (host:port)");
        std::process::exit(2);
    });
    let forward = forward.unwrap_or_else(|| {
        eprintln!("proxy: --forward is required (host:port)");
        std::process::exit(2);
    });

    let cfg = ImpairConfig {
        loss_pct,
        dup_pct,
        reorder_pct,
        reorder_hold,
        jitter_ms_max,
        seed,
        outage_period_s,
        outage_dur_s,
    };

    match proxy::run(listen, forward, cfg, stats_json, run_seconds, None) {
        Ok(stats) => {
            eprintln!(
                "proxy: forwarded={} dropped={} duped={} reordered={}",
                stats.forwarded, stats.dropped, stats.duped, stats.reordered
            );
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("proxy: {e}");
            std::process::exit(2);
        }
    }
}
