use srt_core::{SrtUrl, UrlError};

#[test]
fn ipv4_with_port() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000").unwrap();
    assert_eq!(u.host, "1.2.3.4");
    assert_eq!(u.port, 9000);
}

#[test]
fn dns_with_port() {
    let u = SrtUrl::parse("srt://camera.local:9000").unwrap();
    assert_eq!(u.host, "camera.local");
    assert_eq!(u.port, 9000);
}

#[test]
fn bracketed_ipv6() {
    let u = SrtUrl::parse("srt://[2001:db8::1]:9000").unwrap();
    assert_eq!(u.host, "2001:db8::1");
    assert_eq!(u.port, 9000);
}

#[test]
fn ipv6_loopback() {
    let u = SrtUrl::parse("srt://[::1]:9000").unwrap();
    assert_eq!(u.host, "::1");
    assert_eq!(u.port, 9000);
}

#[test]
fn rejects_wrong_scheme() {
    let e = SrtUrl::parse("https://1.2.3.4:9000").unwrap_err();
    assert!(matches!(e, UrlError::WrongScheme { ref got } if got == "https"));
}

#[test]
fn rejects_no_scheme() {
    // Bare "host:port" doesn't have a scheme separator — url::Url rejects.
    let e = SrtUrl::parse("1.2.3.4:9000").unwrap_err();
    assert!(matches!(e, UrlError::Syntax(_)) || matches!(e, UrlError::WrongScheme { .. }));
}

#[test]
fn rejects_missing_port() {
    let e = SrtUrl::parse("srt://1.2.3.4").unwrap_err();
    assert!(matches!(e, UrlError::MissingPort));
}

#[test]
fn rejects_missing_host() {
    let e = SrtUrl::parse("srt://:9000").unwrap_err();
    assert!(matches!(e, UrlError::MissingHost));
}

#[test]
fn rejects_userinfo() {
    let e = SrtUrl::parse("srt://op:hunter2@1.2.3.4:9000").unwrap_err();
    assert!(matches!(e, UrlError::UserinfoNotSupported));
}

#[test]
fn rejects_userinfo_user_only() {
    let e = SrtUrl::parse("srt://op@1.2.3.4:9000").unwrap_err();
    assert!(matches!(e, UrlError::UserinfoNotSupported));
}

#[test]
fn passphrase_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?passphrase=hunter-too-long-thanks").unwrap();
    assert!(u.overlay.passphrase.is_some());
}

#[test]
fn passphrase_too_short_rejects() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?passphrase=short").unwrap_err();
    assert!(matches!(e, UrlError::OptionValidation { .. }));
}

#[test]
fn passphrase_percent_decoded() {
    // Passphrase contains a percent-encoded space.
    let u = SrtUrl::parse("srt://1.2.3.4:9000?passphrase=hunter%20too%20long").unwrap();
    assert_eq!(
        u.overlay.passphrase.as_ref().unwrap().as_str(),
        "hunter too long"
    );
}

#[test]
fn streamid_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?streamid=front-camera").unwrap();
    assert_eq!(
        u.overlay.stream_id.as_ref().unwrap().as_str(),
        "front-camera"
    );
}

#[test]
fn streamid_with_embedded_equals() {
    // url::Url splits on the FIRST `=` per RFC 3986 form-data; trailing
    // `=` chars are part of the value.
    let u = SrtUrl::parse("srt://1.2.3.4:9000?streamid=foo=bar").unwrap();
    assert_eq!(u.overlay.stream_id.as_ref().unwrap().as_str(), "foo=bar");
}

#[test]
fn streamid_non_ascii_rejects() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?streamid=h%C3%A9llo").unwrap_err();
    assert!(matches!(e, UrlError::OptionValidation { .. }));
}

#[test]
fn congestion_live() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?congestion=live").unwrap();
    assert!(matches!(
        u.overlay.congestion,
        Some(srt_core::Congestion::Live)
    ));
}

#[test]
fn congestion_file() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?congestion=file").unwrap();
    assert!(matches!(
        u.overlay.congestion,
        Some(srt_core::Congestion::File)
    ));
}

#[test]
fn congestion_uppercase_rejects() {
    // Strict-A: enums are lowercase-only.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?congestion=Live").unwrap_err();
    assert!(matches!(e, UrlError::OptionValidation { .. }));
}

#[test]
fn congestion_unknown_rejects() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?congestion=fast").unwrap_err();
    assert!(matches!(e, UrlError::OptionValidation { .. }));
}

#[test]
fn packetfilter_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?packetfilter=fec,cols:10,rows:5,arq:onreq").unwrap();
    assert!(u.overlay.packet_filter.is_some());
}
