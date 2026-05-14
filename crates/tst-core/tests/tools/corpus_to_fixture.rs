//! corpus_to_fixture — extract a minimal TS-packet sub-sequence from a `.ts`
//! file as a committed regression fixture. Optional Rust shim generation.
//!
//! See `docs/cookbook.md` "Capture a regression from the corpus" recipe and
//! `crates/tst-core/tests/fixtures/regression/README.md` for the workflow.
//!
//! Usage:
//!
//!   cargo run -p tst-core --bin corpus_to_fixture -- \
//!     --input /path/to/sample.ts \
//!     --pid 0x1011 \
//!     --packets 1000..2000 \
//!     --out crates/tst-core/tests/fixtures/regression/bug_xyz.bin \
//!     --emit-shim
//!
//! Flag reference:
//!   --input PATH           input .ts file (188-byte-aligned)
//!   --pid HEXDEC           filter to packets with this PID (optional)
//!   --packets START..END   packet-index range, 0-indexed, half-open (optional)
//!   --out PATH             output .bin file (must be under tests/fixtures/regression/)
//!   --emit-shim            also emit tests/regression_<slug>.rs with a
//!                          no-panic smoke test (optional)
//!   -h, --help             print this help

use std::path::PathBuf;
use std::process::ExitCode;

// Used in Task 2 (extraction logic); declared here so the constant is visible
// to run() once that stub is filled in.
#[allow(dead_code)]
const TS_PACKET_SIZE: usize = 188;

#[derive(Debug, PartialEq, Eq)]
struct Args {
    input: PathBuf,
    out: PathBuf,
    pid: Option<u16>,
    packets: Option<(usize, usize)>, // half-open [start, end)
    emit_shim: bool,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut pid: Option<u16> = None;
    let mut packets: Option<(usize, usize)> = None;
    let mut emit_shim = false;
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--input" => {
                input = Some(PathBuf::from(
                    raw.get(i + 1).ok_or("--input needs a value")?,
                ));
                i += 2;
            }
            "--out" => {
                out = Some(PathBuf::from(raw.get(i + 1).ok_or("--out needs a value")?));
                i += 2;
            }
            "--pid" => {
                let s = raw.get(i + 1).ok_or("--pid needs a value")?;
                pid = Some(parse_u16_dec_or_hex(s)?);
                i += 2;
            }
            "--packets" => {
                let s = raw.get(i + 1).ok_or("--packets needs a value")?;
                packets = Some(parse_range(s)?);
                i += 2;
            }
            "--emit-shim" => {
                emit_shim = true;
                i += 1;
            }
            "-h" | "--help" => return Err("HELP".to_string()),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Args {
        input: input.ok_or("missing --input")?,
        out: out.ok_or("missing --out")?,
        pid,
        packets,
        emit_shim,
    })
}

fn parse_u16_dec_or_hex(s: &str) -> Result<u16, String> {
    let (radix, body) = if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (16, rest)
    } else {
        (10, s)
    };
    u16::from_str_radix(body, radix).map_err(|e| format!("invalid PID {s:?}: {e}"))
}

fn parse_range(s: &str) -> Result<(usize, usize), String> {
    let (start, end) = s
        .split_once("..")
        .ok_or_else(|| format!("range must be START..END, got {s:?}"))?;
    let start: usize = start.parse().map_err(|e| format!("bad range start: {e}"))?;
    let end: usize = end.parse().map_err(|e| format!("bad range end: {e}"))?;
    if start >= end {
        return Err(format!("range start {start} >= end {end}"));
    }
    Ok((start, end))
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) if e == "HELP" => {
            print_help();
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}");
            print_help();
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_help() {
    eprintln!("{}", include_str!("corpus_to_fixture_help.txt"));
}

fn run(_args: &Args) -> Result<(), String> {
    Err("not yet implemented".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_args() {
        let argv = vec![
            "--input".into(),
            "/tmp/x.ts".into(),
            "--out".into(),
            "/tmp/o.bin".into(),
        ];
        let args = parse_args(&argv).unwrap();
        assert_eq!(args.input, PathBuf::from("/tmp/x.ts"));
        assert_eq!(args.out, PathBuf::from("/tmp/o.bin"));
        assert_eq!(args.pid, None);
        assert_eq!(args.packets, None);
        assert!(!args.emit_shim);
    }

    #[test]
    fn parse_all_args() {
        let argv = vec![
            "--input".into(),
            "/tmp/x.ts".into(),
            "--out".into(),
            "/tmp/o.bin".into(),
            "--pid".into(),
            "0x1011".into(),
            "--packets".into(),
            "100..200".into(),
            "--emit-shim".into(),
        ];
        let args = parse_args(&argv).unwrap();
        assert_eq!(args.pid, Some(0x1011));
        assert_eq!(args.packets, Some((100, 200)));
        assert!(args.emit_shim);
    }

    #[test]
    fn parse_pid_decimal() {
        let argv = vec![
            "--input".into(),
            "i".into(),
            "--out".into(),
            "o".into(),
            "--pid".into(),
            "256".into(),
        ];
        assert_eq!(parse_args(&argv).unwrap().pid, Some(256));
    }

    #[test]
    fn parse_range_empty_rejected() {
        assert!(parse_range("100..100").is_err());
        assert!(parse_range("200..100").is_err());
    }

    #[test]
    fn missing_required_input() {
        let argv = vec!["--out".into(), "/tmp/o.bin".into()];
        assert!(parse_args(&argv).is_err());
    }
}
