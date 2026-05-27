//! HTTP Basic auth (RFC 7617) check for the HLS server.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// Check whether the `Authorization: Basic ...` header matches the expected
/// (user, password).  Returns `true` on match, `false` on mismatch / absent /
/// malformed.
pub(crate) fn check_basic_auth(
    expected_user: &str,
    expected_pass: &str,
    header_value: Option<&str>,
) -> bool {
    let header = match header_value {
        Some(h) => h,
        None => return false,
    };
    let Some(b64) = header.strip_prefix("Basic ").or_else(|| header.strip_prefix("basic ")) else {
        return false;
    };
    let Ok(decoded) = STANDARD.decode(b64.trim()) else {
        return false;
    };
    let Ok(s) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some((user, pass)) = s.split_once(':') else {
        return false;
    };
    user == expected_user && pass == expected_pass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(s: &str) -> String {
        STANDARD.encode(s.as_bytes())
    }

    #[test]
    fn correct_credentials_match() {
        let h = format!("Basic {}", b64("alice:s3cret"));
        assert!(check_basic_auth("alice", "s3cret", Some(&h)));
    }

    #[test]
    fn wrong_password_rejected() {
        let h = format!("Basic {}", b64("alice:wrong"));
        assert!(!check_basic_auth("alice", "s3cret", Some(&h)));
    }

    #[test]
    fn absent_header_rejected() {
        assert!(!check_basic_auth("alice", "s3cret", None));
    }

    #[test]
    fn malformed_b64_rejected() {
        assert!(!check_basic_auth("alice", "s3cret", Some("Basic !@#$")));
    }

    #[test]
    fn wrong_scheme_rejected() {
        let h = format!("Bearer {}", b64("alice:s3cret"));
        assert!(!check_basic_auth("alice", "s3cret", Some(&h)));
    }
}
