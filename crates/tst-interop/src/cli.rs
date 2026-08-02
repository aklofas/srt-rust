//! Shared CLI argument parsing helpers for `main.rs`'s subcommands.

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
}
