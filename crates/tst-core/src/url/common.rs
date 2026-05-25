//! Scheme-neutral URL parsing for `srt://`, `rtp://`, `rtsp://`, `rtsps://`.
//!
//! See [the module docs](super) for the URL shape we accept.

use std::borrow::Cow;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use thiserror::Error;

/// A parsed URL with no scheme-specific interpretation applied.
///
/// Lifetime-borrows from the input string for the small fields; the query
/// vector owns its key/value pairs because percent-decoding may produce
/// new strings.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl<'a> {
    /// Lowercase scheme: `"srt"`, `"rtp"`, `"rtsp"`, `"rtsps"`, etc.
    pub scheme: &'a str,
    /// `user` from `user[:password]@host` — None when no userinfo present.
    pub username: Option<&'a str>,
    /// `password` from `user:password@host` — None when no `:` in userinfo.
    pub password: Option<&'a str>,
    /// Host, with IPv6 brackets stripped (`::1` not `[::1]`).
    pub host: &'a str,
    /// Port — None when the URL omits it (`scheme://host/path`).
    pub port: Option<u16>,
    /// Path, including the leading `/`. Empty string when the URL has no path.
    pub path: &'a str,
    /// Query pairs in URL order. Last-occurrence wins is the caller's
    /// responsibility. Values are percent-decoded.
    pub query: Vec<(Cow<'a, str>, Cow<'a, str>)>,
}

/// Errors that may arise from [`parse_url`] / [`parse_host_port`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum UrlError {
    /// `://` separator missing.
    #[error("URL missing '://' separator")]
    MissingSchemeSeparator,
    /// Scheme component is empty (e.g., `"://host"`).
    #[error("URL has empty scheme before '://'")]
    EmptyScheme,
    /// Port couldn't be parsed as a u16.
    #[error("invalid port '{got}': {detail}")]
    InvalidPort { got: String, detail: String },
    /// IPv6 host opened with `[` but never closed with `]`.
    #[error("URL has '[' but no matching ']' in host")]
    UnclosedIpv6Bracket,
    /// A `%XY` percent-escape was malformed (non-hex, truncated).
    #[error("malformed percent-encoding in URL: {detail}")]
    BadPercentEncoding { detail: String },
    /// Host string is empty in a context where it must be present
    /// (caller's responsibility to call this; `parse_url` itself accepts
    /// empty hosts and leaves the policy to the caller).
    #[error("URL must include a host")]
    MissingHost,
}

/// Stub — implemented in Task 2.
pub fn parse_url(_s: &str) -> Result<ParsedUrl<'_>, UrlError> {
    todo!("Task 2 implements this")
}

/// Stub — implemented in Task 6.
pub fn parse_host_port(_s: &str) -> Result<(IpAddr, u16), UrlError> {
    todo!("Task 6 implements this")
}

/// IPv4 multicast: `224.0.0.0/4` (RFC 5771).
#[must_use]
pub fn is_multicast_v4(addr: Ipv4Addr) -> bool {
    let oct = addr.octets()[0];
    (224..=239).contains(&oct)
}

/// IPv6 multicast: `ff00::/8` (RFC 4291 §2.7).
#[must_use]
pub fn is_multicast_v6(addr: Ipv6Addr) -> bool {
    addr.octets()[0] == 0xff
}
