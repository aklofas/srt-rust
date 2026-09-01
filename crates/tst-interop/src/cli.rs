//! Shared CLI argument-parsing and output helpers for `main.rs`'s subcommands.

/// Parse a `--seconds` argument, rejecting anything that isn't a finite,
/// strictly positive number.
///
/// `gen`/`send`/`recv`/`verify` all divide by (or multiply nominal
/// per-second counts against) this value — zero, negative, NaN, or
/// infinite would either produce no traffic at all or make
/// `Tally::finish`'s `(nominal * seconds * slack).floor() as u64` floor
/// collapse to `0`, letting `verify`/`recv` report PASS on a capture that
/// carried essentially nothing. Rejecting those shapes here, at parse
/// time, keeps that floor meaningful.
pub fn parse_seconds(s: &str) -> Option<f64> {
    let v: f64 = s.parse().ok()?;
    if v > 0.0 && v.is_finite() {
        Some(v)
    } else {
        None
    }
}

/// Serialize `value` to pretty JSON and write it to `target`, or print it
/// to stdout if `target` is `"-"`.
///
/// Shared by every subcommand's `--json OUT` flag (`send`/`recv`/`verify`)
/// so a write failure is reported identically everywhere, instead of each
/// call site inlining its own `serde_json::to_string_pretty` +
/// `fs::write` (which had drifted: `verify`'s inline copy reported a
/// write failure directly via `eprintln!` + `process::exit`, not via the
/// `Result` the other two call sites already used).
pub fn write_json<T: serde::Serialize>(target: &str, value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value).expect("value always serializes");
    if target == "-" {
        println!("{json}");
    } else {
        std::fs::write(target, json).map_err(|e| format!("write {target}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_normal_positive_value() {
        assert_eq!(parse_seconds("3.0"), Some(3.0));
        assert_eq!(parse_seconds("0.5"), Some(0.5));
    }

    #[test]
    fn rejects_zero_negative_nan_and_infinite() {
        assert_eq!(parse_seconds("0"), None);
        assert_eq!(parse_seconds("0.0"), None);
        assert_eq!(parse_seconds("-1"), None);
        assert_eq!(parse_seconds("-3.5"), None);
        assert_eq!(parse_seconds("NaN"), None);
        assert_eq!(parse_seconds("inf"), None);
        assert_eq!(parse_seconds("-inf"), None);
    }

    #[test]
    fn rejects_unparseable_input() {
        assert_eq!(parse_seconds("not-a-number"), None);
        assert_eq!(parse_seconds(""), None);
    }

    #[derive(serde::Serialize)]
    struct Sample {
        n: u32,
    }

    #[test]
    fn write_json_writes_pretty_json_to_a_file() {
        let path = std::env::temp_dir().join("tst-interop-cli-test-write.json");
        let path_str = path.to_str().unwrap();
        write_json(path_str, &Sample { n: 7 }).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"n\": 7"));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn write_json_dash_target_does_not_error() {
        // "-" means "print to stdout" — just assert it doesn't error;
        // stdout content isn't worth capturing here.
        write_json("-", &Sample { n: 1 }).unwrap();
    }

    #[test]
    fn write_json_surfaces_the_write_error() {
        // A path under a nonexistent directory can never be created.
        let bad = std::env::temp_dir()
            .join("tst-interop-cli-test-nonexistent-dir")
            .join("out.json");
        let err = write_json(bad.to_str().unwrap(), &Sample { n: 1 }).unwrap_err();
        assert!(err.contains("write"));
    }
}
