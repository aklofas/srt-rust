use std::time::Duration;

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

#[test]
fn pbkeylen_16() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?pbkeylen=16").unwrap();
    assert!(matches!(
        u.overlay.key_length,
        Some(srt_core::KeyLength::Aes128)
    ));
}

#[test]
fn pbkeylen_24() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?pbkeylen=24").unwrap();
    assert!(matches!(
        u.overlay.key_length,
        Some(srt_core::KeyLength::Aes192)
    ));
}

#[test]
fn pbkeylen_32() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?pbkeylen=32").unwrap();
    assert!(matches!(
        u.overlay.key_length,
        Some(srt_core::KeyLength::Aes256)
    ));
}

#[test]
fn pbkeylen_invalid() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?pbkeylen=15").unwrap_err();
    assert!(matches!(e, UrlError::OptionValidation { .. }));
}

#[test]
fn latency_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=200").unwrap();
    assert_eq!(u.overlay.latency, Some(Duration::from_millis(200)));
}

#[test]
fn latency_with_suffix_rejects() {
    // Strict-A: no "ms"/"s" suffixes.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?latency=200ms").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { ref key, .. } if key == "latency"));
}

#[test]
fn latency_negative_rejects() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?latency=-1").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { .. }));
}

#[test]
fn latency_overflow_rejects() {
    // 2^31 = 2147483648 — outside i32 range; libsrt SRTO_LATENCY is i32.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?latency=2147483648").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { .. }));
}

#[test]
fn rcvlatency_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?rcvlatency=120").unwrap();
    assert_eq!(u.overlay.recv_latency, Some(Duration::from_millis(120)));
}

#[test]
fn peerlatency_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?peerlatency=80").unwrap();
    assert_eq!(u.overlay.peer_latency, Some(Duration::from_millis(80)));
}

#[test]
fn mss_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?mss=1400").unwrap();
    assert_eq!(u.overlay.mss, Some(1400));
}

#[test]
fn payloadsize_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?payloadsize=1316").unwrap();
    assert_eq!(u.overlay.payload_size, Some(1316));
}

#[test]
fn maxbw_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?maxbw=10000000").unwrap();
    assert!(matches!(
        u.overlay.max_bandwidth,
        Some(srt_core::MaxBandwidth::Limited(10_000_000))
    ));
}

#[test]
fn inputbw_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?inputbw=5000000").unwrap();
    assert_eq!(u.overlay.input_bandwidth, Some(5_000_000));
}

#[test]
fn oheadbw_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?oheadbw=25").unwrap();
    assert_eq!(u.overlay.overhead_bandwidth_pct, Some(25));
}

#[test]
fn oheadbw_out_of_range() {
    // Builder enforces 5..=100 in apply_socket_config; URL parser rejects
    // here at value-conversion since we know the bound.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?oheadbw=4").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { .. }));
    let e = SrtUrl::parse("srt://1.2.3.4:9000?oheadbw=101").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { .. }));
}

#[test]
fn lossmaxttl_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?lossmaxttl=20").unwrap();
    assert_eq!(u.overlay.loss_max_ttl, Some(20));
}

#[test]
fn fc_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?fc=8192").unwrap();
    assert_eq!(u.overlay.flow_window_packets, Some(8192));
}

#[test]
fn tlpktdrop_zero() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?tlpktdrop=0").unwrap();
    assert_eq!(u.overlay.too_late_packet_drop, Some(false));
}

#[test]
fn tlpktdrop_one() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?tlpktdrop=1").unwrap();
    assert_eq!(u.overlay.too_late_packet_drop, Some(true));
}

#[test]
fn tlpktdrop_true_rejects() {
    // Strict-A: BOOL is "0"/"1" only.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?tlpktdrop=true").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { ref key, .. } if key == "tlpktdrop"));
}

#[test]
fn tlpktdrop_two_rejects() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?tlpktdrop=2").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { .. }));
}

#[test]
fn x_recvtimeout_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?x-recvtimeout=5000").unwrap();
    assert_eq!(u.overlay.recv_timeout, Some(Duration::from_millis(5000)));
}

#[test]
fn x_sendtimeout_query() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?x-sendtimeout=2000").unwrap();
    assert_eq!(u.overlay.send_timeout, Some(Duration::from_millis(2000)));
}

#[test]
fn x_unknown_extension_rejects() {
    // x- prefix is reserved but not a free-for-all (spec §4.2).
    let e = SrtUrl::parse("srt://1.2.3.4:9000?x-foo=bar").unwrap_err();
    assert!(matches!(e, UrlError::UnknownKey { ref key } if key == "x-foo"));
}
