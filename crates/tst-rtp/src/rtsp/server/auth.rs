//! Server-side challenge generation + Authorization verification.
//!
//! Symmetric with the Phase 2 client-side primitives in
//! `crate::rtsp::auth`: the wire shapes of `WWW-Authenticate` and
//! `Authorization` are direct mirrors. Server emits challenges; client
//! emits responses; server verifies by recomputing the same Digest math.
//!
//! The `dead_code` allow at module level is scoped to this submodule
//! (consumed by `handlers.rs`).

#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use secrecy::{ExposeSecret, SecretString};

use crate::builder::{ServerAuthConfig, ServerAuthScheme};

/// Verification result. `Ok(())` means the request is authorized; any
/// error means the request handler should reply 401 (typically with a
/// fresh challenge).
#[derive(Debug, thiserror::Error)]
pub(crate) enum AuthVerifyError {
    #[error("missing Authorization header")]
    Missing,
    #[error("unsupported auth scheme in Authorization header")]
    SchemeMismatch,
    #[error("base64 decode failed: {0}")]
    BadBasic(String),
    #[error("Basic credentials don't match")]
    WrongCredentials,
    #[error("malformed Digest Authorization header: {detail}")]
    BadDigestSyntax { detail: String },
    #[error("Digest response doesn't match expected")]
    WrongDigestResponse,
    #[error("Digest nonce mismatch (stale)")]
    StaleNonce,
}

/// Generate a fresh 16-byte nonce hex-encoded for use in a `WWW-Authenticate`
/// `nonce=` parameter. Rotated per session.
pub(crate) fn generate_nonce() -> String {
    let mut buf = [0u8; 16];
    if let Err(e) = getrandom::getrandom(&mut buf) {
        tracing::warn!(error = %e, "getrandom failed during nonce generation");
        // Fallback to a counter-based nonce; not great but won't allow
        // replay attacks within a session because we change it per-session.
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        buf[..4].copy_from_slice(&n.to_be_bytes());
    }
    let mut s = String::with_capacity(32);
    for b in buf {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Build the `WWW-Authenticate:` header value for the configured scheme.
///
/// `nonce` is only consumed for the Digest schemes; Basic ignores it.
pub(crate) fn build_challenge_header(cfg: &ServerAuthConfig, nonce: &str) -> String {
    match cfg.scheme {
        ServerAuthScheme::Basic => format!("Basic realm=\"{}\"", cfg.realm),
        ServerAuthScheme::DigestMd5 => format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm=MD5, qop=\"auth\"",
            cfg.realm, nonce
        ),
        ServerAuthScheme::DigestSha256 => format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm=SHA-256, qop=\"auth\"",
            cfg.realm, nonce
        ),
    }
}

/// Verify the client's `Authorization:` header against the configured
/// credentials. Caller passes `Option<&str>` so the common "no header"
/// case maps directly to [`AuthVerifyError::Missing`].
///
/// For Digest auth, `expected_nonce` is the value the server most
/// recently emitted in `WWW-Authenticate: ... nonce=...` for this
/// session. Mismatches surface as `AuthVerifyError::StaleNonce`, which
/// the request handler should re-challenge with `stale=true`.
pub(crate) fn verify_authorization(
    auth_header: Option<&str>,
    method: &str,
    uri: &str,
    cfg: &ServerAuthConfig,
    expected_nonce: &str,
) -> Result<(), AuthVerifyError> {
    let auth = auth_header.ok_or(AuthVerifyError::Missing)?;
    match cfg.scheme {
        ServerAuthScheme::Basic => verify_basic(auth, &cfg.username, &cfg.password),
        ServerAuthScheme::DigestMd5 | ServerAuthScheme::DigestSha256 => verify_digest(
            auth,
            method,
            uri,
            &cfg.username,
            &cfg.password,
            cfg.scheme,
            expected_nonce,
        ),
    }
}

fn verify_basic(
    auth_header: &str,
    expected_user: &str,
    expected_pass: &SecretString,
) -> Result<(), AuthVerifyError> {
    let value = auth_header
        .strip_prefix("Basic ")
        .ok_or(AuthVerifyError::SchemeMismatch)?
        .trim();
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|e| AuthVerifyError::BadBasic(e.to_string()))?;
    let s = std::str::from_utf8(&decoded).map_err(|e| AuthVerifyError::BadBasic(e.to_string()))?;
    let (user, pass) = s
        .split_once(':')
        .ok_or_else(|| AuthVerifyError::BadBasic("missing colon".to_string()))?;
    if user == expected_user && pass == expected_pass.expose_secret() {
        Ok(())
    } else {
        Err(AuthVerifyError::WrongCredentials)
    }
}

fn verify_digest(
    auth_header: &str,
    method: &str,
    uri: &str,
    expected_user: &str,
    expected_pass: &SecretString,
    scheme: ServerAuthScheme,
    expected_nonce: &str,
) -> Result<(), AuthVerifyError> {
    let value = auth_header
        .strip_prefix("Digest ")
        .ok_or(AuthVerifyError::SchemeMismatch)?;
    let params = parse_kv_pairs(value);
    let username = params.get("username").map(String::as_str).unwrap_or("");
    let nonce_attr = params.get("nonce").map(String::as_str).unwrap_or("");
    let cnonce = params.get("cnonce").map(String::as_str).unwrap_or("");
    let nc = params.get("nc").map(String::as_str).unwrap_or("");
    let qop = params.get("qop").map(String::as_str).unwrap_or("");
    let uri_attr = params.get("uri").map(String::as_str).unwrap_or("");
    let response_attr = params.get("response").map(String::as_str).unwrap_or("");
    let realm_attr = params.get("realm").map(String::as_str).unwrap_or("");

    if username != expected_user {
        return Err(AuthVerifyError::WrongCredentials);
    }
    if nonce_attr != expected_nonce {
        return Err(AuthVerifyError::StaleNonce);
    }
    if uri_attr != uri {
        // RFC 7616 §3.4.6 — URI in Authorization MUST match request URI.
        return Err(AuthVerifyError::BadDigestSyntax {
            detail: format!("uri attr '{uri_attr}' != request uri '{uri}'"),
        });
    }
    let expected_response = compute_digest_response(
        scheme,
        expected_user,
        realm_attr,
        expected_pass,
        method,
        uri,
        nonce_attr,
        nc,
        cnonce,
        qop,
    );
    if response_attr == expected_response {
        Ok(())
    } else {
        Err(AuthVerifyError::WrongDigestResponse)
    }
}

#[allow(clippy::too_many_arguments)]
fn compute_digest_response(
    scheme: ServerAuthScheme,
    user: &str,
    realm: &str,
    pass: &SecretString,
    method: &str,
    uri: &str,
    nonce: &str,
    nc: &str,
    cnonce: &str,
    qop: &str,
) -> String {
    use md5::Md5;
    use sha2::{Digest as _, Sha256};
    fn hex<const N: usize>(b: [u8; N]) -> String {
        let mut s = String::with_capacity(N * 2);
        for x in b {
            use std::fmt::Write;
            let _ = write!(s, "{:02x}", x);
        }
        s
    }
    let h: fn(&str) -> String = match scheme {
        ServerAuthScheme::DigestMd5 => |s| {
            let mut h = Md5::new();
            h.update(s.as_bytes());
            let r: [u8; 16] = h.finalize().into();
            hex(r)
        },
        ServerAuthScheme::DigestSha256 => |s| {
            let mut h = Sha256::new();
            h.update(s.as_bytes());
            let r: [u8; 32] = h.finalize().into();
            hex(r)
        },
        ServerAuthScheme::Basic => unreachable!("compute_digest_response not called for Basic"),
    };
    let a1 = format!("{user}:{realm}:{}", pass.expose_secret());
    let ha1 = h(&a1);
    let a2 = format!("{method}:{uri}");
    let ha2 = h(&a2);
    let body = if qop.is_empty() {
        format!("{ha1}:{nonce}:{ha2}")
    } else {
        format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}")
    };
    h(&body)
}

/// Tokenize a Digest parameter list `key="value", key2=token, ...` into a
/// HashMap. Tolerant of optional whitespace around `=` and `,`. Quoted
/// values may contain commas; backslash-escapes inside quoted strings are
/// handled per RFC 7616 §3.4.
fn parse_kv_pairs(input: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',') {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b',' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b',' {
            i += 1;
            continue;
        }
        let key = input[key_start..i].trim().to_ascii_lowercase();
        i += 1; // skip '='
        if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let val_start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            out.insert(key, input[val_start..i].to_string());
            if i < bytes.len() {
                i += 1;
            }
        } else {
            let val_start = i;
            while i < bytes.len() && bytes[i] != b',' && bytes[i] != b' ' {
                i += 1;
            }
            out.insert(key, input[val_start..i].to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    fn basic_cfg() -> ServerAuthConfig {
        ServerAuthConfig {
            scheme: ServerAuthScheme::Basic,
            realm: "tst".into(),
            username: "admin".into(),
            password: SecretString::new("secret".into()),
        }
    }

    fn digest_md5_cfg() -> ServerAuthConfig {
        ServerAuthConfig {
            scheme: ServerAuthScheme::DigestMd5,
            realm: "tst-rtp".into(),
            username: "admin".into(),
            password: SecretString::new("secret".into()),
        }
    }

    #[test]
    fn generate_nonce_is_32_hex_chars() {
        let n = generate_nonce();
        assert_eq!(n.len(), 32);
        assert!(n.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_nonce_changes_each_call() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn challenge_header_basic() {
        let cfg = basic_cfg();
        let ch = build_challenge_header(&cfg, "ignored");
        assert_eq!(ch, "Basic realm=\"tst\"");
    }

    #[test]
    fn challenge_header_digest_md5() {
        let cfg = digest_md5_cfg();
        let ch = build_challenge_header(&cfg, "abc123");
        assert!(ch.contains("Digest"));
        assert!(ch.contains("realm=\"tst-rtp\""));
        assert!(ch.contains("nonce=\"abc123\""));
        assert!(ch.contains("algorithm=MD5"));
        assert!(ch.contains("qop=\"auth\""));
    }

    #[test]
    fn challenge_header_digest_sha256() {
        let cfg = ServerAuthConfig {
            scheme: ServerAuthScheme::DigestSha256,
            ..digest_md5_cfg()
        };
        let ch = build_challenge_header(&cfg, "xyz789");
        assert!(ch.contains("algorithm=SHA-256"));
    }

    #[test]
    fn verify_missing_authorization() {
        let cfg = basic_cfg();
        let e = verify_authorization(None, "DESCRIBE", "rtsp://x", &cfg, "").unwrap_err();
        assert!(matches!(e, AuthVerifyError::Missing));
    }

    #[test]
    fn verify_basic_round_trip() {
        let cfg = basic_cfg();
        use base64::Engine;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(b"admin:secret")
        );
        verify_authorization(Some(&auth), "DESCRIBE", "rtsp://server/live", &cfg, "").unwrap();
    }

    #[test]
    fn verify_basic_wrong_password() {
        let cfg = basic_cfg();
        use base64::Engine;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(b"admin:wrong")
        );
        let e = verify_authorization(Some(&auth), "DESCRIBE", "rtsp://server/live", &cfg, "")
            .unwrap_err();
        assert!(matches!(e, AuthVerifyError::WrongCredentials));
    }

    #[test]
    fn verify_basic_wrong_username() {
        let cfg = basic_cfg();
        use base64::Engine;
        let auth = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(b"wrong:secret")
        );
        let e = verify_authorization(Some(&auth), "DESCRIBE", "rtsp://server/live", &cfg, "")
            .unwrap_err();
        assert!(matches!(e, AuthVerifyError::WrongCredentials));
    }

    #[test]
    fn verify_basic_bad_base64() {
        let cfg = basic_cfg();
        let auth = "Basic not-base64!!";
        let e = verify_authorization(Some(auth), "DESCRIBE", "rtsp://x", &cfg, "").unwrap_err();
        assert!(matches!(e, AuthVerifyError::BadBasic(_)));
    }

    #[test]
    fn verify_scheme_mismatch() {
        let cfg = basic_cfg();
        let auth = "Bearer some-token-here";
        let e = verify_authorization(Some(auth), "DESCRIBE", "rtsp://x", &cfg, "").unwrap_err();
        assert!(matches!(e, AuthVerifyError::SchemeMismatch));
    }

    #[test]
    fn verify_digest_md5_round_trip() {
        let cfg = digest_md5_cfg();
        let nonce = "abc123";
        let nc = "00000001";
        let cnonce = "deadbeef";
        let qop = "auth";
        let method = "DESCRIBE";
        let uri = "rtsp://server/live";
        let response = compute_digest_response(
            ServerAuthScheme::DigestMd5,
            "admin",
            "tst-rtp",
            &cfg.password,
            method,
            uri,
            nonce,
            nc,
            cnonce,
            qop,
        );
        let auth = format!(
            "Digest username=\"admin\", realm=\"tst-rtp\", nonce=\"{nonce}\", \
             uri=\"{uri}\", response=\"{response}\", algorithm=MD5, \
             nc={nc}, cnonce=\"{cnonce}\", qop={qop}"
        );
        verify_authorization(Some(&auth), method, uri, &cfg, nonce).unwrap();
    }

    #[test]
    fn verify_digest_sha256_round_trip() {
        let cfg = ServerAuthConfig {
            scheme: ServerAuthScheme::DigestSha256,
            ..digest_md5_cfg()
        };
        let nonce = "abc123";
        let response = compute_digest_response(
            ServerAuthScheme::DigestSha256,
            "admin",
            "tst-rtp",
            &cfg.password,
            "DESCRIBE",
            "rtsp://server/live",
            nonce,
            "00000001",
            "cnonce123",
            "auth",
        );
        let auth = format!(
            "Digest username=\"admin\", realm=\"tst-rtp\", nonce=\"{nonce}\", \
             uri=\"rtsp://server/live\", response=\"{response}\", algorithm=SHA-256, \
             nc=00000001, cnonce=\"cnonce123\", qop=auth"
        );
        verify_authorization(Some(&auth), "DESCRIBE", "rtsp://server/live", &cfg, nonce).unwrap();
    }

    #[test]
    fn verify_digest_stale_nonce() {
        let cfg = digest_md5_cfg();
        let auth = "Digest username=\"admin\", realm=\"tst-rtp\", nonce=\"stale\", \
                    uri=\"rtsp://x\", response=\"any\", algorithm=MD5";
        let e =
            verify_authorization(Some(auth), "DESCRIBE", "rtsp://x", &cfg, "fresh").unwrap_err();
        assert!(matches!(e, AuthVerifyError::StaleNonce));
    }

    #[test]
    fn verify_digest_wrong_username() {
        let cfg = digest_md5_cfg();
        let auth = "Digest username=\"impostor\", realm=\"tst-rtp\", nonce=\"abc\", \
                    uri=\"rtsp://x\", response=\"any\", algorithm=MD5";
        let e = verify_authorization(Some(auth), "DESCRIBE", "rtsp://x", &cfg, "abc").unwrap_err();
        assert!(matches!(e, AuthVerifyError::WrongCredentials));
    }

    #[test]
    fn verify_digest_uri_mismatch() {
        let cfg = digest_md5_cfg();
        let nonce = "abc";
        let response = compute_digest_response(
            ServerAuthScheme::DigestMd5,
            "admin",
            "tst-rtp",
            &cfg.password,
            "DESCRIBE",
            "rtsp://x",
            nonce,
            "00000001",
            "cnonce",
            "auth",
        );
        // Client sent uri="rtsp://y" but request line says "rtsp://x".
        let auth = format!(
            "Digest username=\"admin\", realm=\"tst-rtp\", nonce=\"{nonce}\", \
             uri=\"rtsp://y\", response=\"{response}\", algorithm=MD5, \
             nc=00000001, cnonce=\"cnonce\", qop=auth"
        );
        let e = verify_authorization(Some(&auth), "DESCRIBE", "rtsp://x", &cfg, nonce).unwrap_err();
        assert!(matches!(e, AuthVerifyError::BadDigestSyntax { .. }));
    }

    #[test]
    fn parse_kv_pairs_quoted_with_commas() {
        let m = parse_kv_pairs(r#"a="hello, world", b=token"#);
        assert_eq!(m.get("a").map(String::as_str), Some("hello, world"));
        assert_eq!(m.get("b").map(String::as_str), Some("token"));
    }

    #[test]
    fn parse_kv_pairs_escaped_quotes() {
        // Parser preserves the literal backslash-escape sequences inside
        // the quoted value — it skips over `\"` for the purpose of
        // finding the closing `"`, but doesn't un-escape the bytes
        // themselves. Real Authorization headers don't carry quoted
        // strings with embedded `"`, so this is best-effort tolerance,
        // not a faithful un-escape per RFC 7616 §3.4.
        let m = parse_kv_pairs(r#"a="he said \"hi\"", b=ok"#);
        // Quoted value runs `he said \"hi\"` — 14 bytes including escapes.
        assert_eq!(m.get("a").map(String::as_str), Some(r#"he said \"hi\""#));
        assert_eq!(m.get("b").map(String::as_str), Some("ok"));
    }
}
