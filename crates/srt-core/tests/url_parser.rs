use std::time::Duration;

use srt_core::srt::config::{ListenerConfig, SocketConfig};
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

#[test]
fn mode_caller_accepted_noop() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?mode=caller&latency=100").unwrap();
    assert_eq!(u.overlay.latency, Some(Duration::from_millis(100)));
}

#[test]
fn mode_listener_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?mode=listener").unwrap_err();
    assert!(matches!(e, UrlError::UnsupportedMode { ref mode } if mode == "listener"));
}

#[test]
fn mode_rendezvous_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?mode=rendezvous").unwrap_err();
    assert!(matches!(e, UrlError::UnsupportedMode { ref mode } if mode == "rendezvous"));
}

#[test]
fn group3_conntimeo_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?conntimeo=5000").unwrap_err();
    let UrlError::UnsupportedKey { key, srto } = e else {
        panic!("wrong variant");
    };
    assert_eq!(key, "conntimeo");
    assert_eq!(srto, "SRTO_CONNTIMEO");
}

#[test]
fn group3_transtype_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?transtype=live").unwrap_err();
    let UrlError::UnsupportedKey { key, srto } = e else {
        panic!("wrong variant");
    };
    assert_eq!(key, "transtype");
    assert_eq!(srto, "SRTO_TRANSTYPE");
}

#[test]
fn group3_rcvbuf_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?rcvbuf=1048576").unwrap_err();
    let UrlError::UnsupportedKey { key, srto } = e else {
        panic!("wrong variant");
    };
    assert_eq!(key, "rcvbuf");
    assert_eq!(srto, "SRTO_RCVBUF");
}

#[test]
fn group3_sndbuf_rejected() {
    let e = SrtUrl::parse("srt://1.2.3.4:9000?sndbuf=1048576").unwrap_err();
    assert!(matches!(e, UrlError::UnsupportedKey { ref key, .. } if key == "sndbuf"));
}

#[test]
fn unknown_typo_rejected() {
    // Distinct from UnsupportedKey: this name is not in the libsrt
    // vocabulary at all (typo of "latency").
    let e = SrtUrl::parse("srt://1.2.3.4:9000?lattency=100").unwrap_err();
    assert!(matches!(e, UrlError::UnknownKey { ref key } if key == "lattency"));
}

/// Smoke test that all 24 Group 3 keys reject with a non-empty `srto`.
/// Catches drift if someone forgets to fill in the SRTO_* string.
#[test]
fn all_group3_keys_reject_with_srto() {
    let group3 = [
        "bindtodevice",
        "conntimeo",
        "cryptomode",
        "drifttracer",
        "enforcedencryption",
        "groupconnect",
        "groupminstabletimeo",
        "iptos",
        "ipttl",
        "ipv6only",
        "kmpreannounce",
        "kmrefreshrate",
        "maxrexmitbw",
        "messageapi",
        "mininputbw",
        "minversion",
        "nakreport",
        "peeridletimeo",
        "rcvbuf",
        "retransmitalgo",
        "sndbuf",
        "snddropdelay",
        "transtype",
        "tsbpdmode",
    ];
    for key in group3 {
        let url = format!("srt://1.2.3.4:9000?{key}=1");
        let e = SrtUrl::parse(&url).unwrap_err();
        match e {
            UrlError::UnsupportedKey { key: k, srto } => {
                assert_eq!(k, key, "wrong key in error");
                assert!(!srto.is_empty(), "{key}: srto must not be empty");
                assert!(srto.starts_with("SRTO_"), "{key}: srto must be SRTO_*");
            }
            other => panic!("{key}: expected UnsupportedKey, got {other:?}"),
        }
    }
}

#[test]
fn last_occurrence_wins_on_duplicate_keys() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=100&latency=200").unwrap();
    assert_eq!(u.overlay.latency, Some(Duration::from_millis(200)));
}

#[test]
fn adapter_rejected_as_unsupported() {
    // "adapter" is not in libsrt's vocabulary table per the apps source,
    // and it has no SocketBuilder counterpart in this library. Reject as
    // UnknownKey (spec §4.4 — adapter is rejected for v1).
    let e = SrtUrl::parse("srt://1.2.3.4:9000?adapter=192.168.1.5").unwrap_err();
    assert!(matches!(e, UrlError::UnknownKey { ref key } if key == "adapter"));
}

#[test]
fn multi_key_url_combines() {
    let u = SrtUrl::parse(
        "srt://camera.local:9000?streamid=front&latency=200&passphrase=hunter-too-long&pbkeylen=24",
    )
    .unwrap();
    assert_eq!(u.host, "camera.local");
    assert_eq!(u.port, 9000);
    assert_eq!(u.overlay.stream_id.as_ref().unwrap().as_str(), "front");
    assert_eq!(u.overlay.latency, Some(Duration::from_millis(200)));
    assert!(u.overlay.passphrase.is_some());
    assert!(matches!(
        u.overlay.key_length,
        Some(srt_core::KeyLength::Aes192)
    ));
}

#[test]
fn empty_value_rejected() {
    // Strict-A: empty INT can't parse.
    let e = SrtUrl::parse("srt://1.2.3.4:9000?latency=").unwrap_err();
    assert!(matches!(e, UrlError::InvalidValue { ref key, .. } if key == "latency"));
}

#[test]
fn invalid_percent_encoding_does_not_panic() {
    // url::Url is lenient about malformed percent-encoding in query strings:
    // it passes through the raw bytes (e.g. "%2" stays "%2") rather than
    // erroring. StreamId::new accepts "%2" as valid ASCII, so the parse
    // succeeds. What matters: no panic regardless of outcome.
    let _ = SrtUrl::parse("srt://1.2.3.4:9000?streamid=%2");
}

#[test]
fn passphrase_with_plus_sign_is_literal_plus() {
    // url::Url's query_pairs decodes `+` as space (form-urlencoded
    // convention). For SRT URLs we want `+` literal — but rather than
    // diverge from url::Url, document the convention: SRT URLs follow
    // form-urlencoded, so passphrase containing `+` must be percent-
    // encoded as `%2B`.
    let u = SrtUrl::parse("srt://1.2.3.4:9000?passphrase=hunter%2Btoo%2Blong").unwrap();
    assert_eq!(
        u.overlay.passphrase.as_ref().unwrap().as_str(),
        "hunter+too+long"
    );
}

#[test]
fn apply_to_socket_writes_through() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=200&streamid=front&pbkeylen=24").unwrap();
    let mut cfg = SocketConfig::default();
    u.overlay.apply_to_socket(&mut cfg);
    assert_eq!(cfg.latency, Some(Duration::from_millis(200)));
    assert_eq!(cfg.stream_id.as_ref().unwrap().as_str(), "front");
    assert!(matches!(cfg.key_length, srt_core::KeyLength::Aes192));
}

#[test]
fn apply_to_socket_url_wins_over_existing() {
    // Pre-populate cfg with a builder value; overlay should overwrite.
    let mut cfg = SocketConfig {
        latency: Some(Duration::from_millis(100)),
        ..Default::default()
    };
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=200").unwrap();
    u.overlay.apply_to_socket(&mut cfg);
    assert_eq!(cfg.latency, Some(Duration::from_millis(200)));
}

#[test]
fn apply_to_socket_does_not_clear_unset_fields() {
    // Overlay has only latency set; other pre-populated fields stay.
    let mut cfg = SocketConfig {
        mss: Some(1316),
        ..Default::default()
    };
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=100").unwrap();
    u.overlay.apply_to_socket(&mut cfg);
    assert_eq!(cfg.latency, Some(Duration::from_millis(100)));
    assert_eq!(cfg.mss, Some(1316)); // preserved
}

#[test]
fn apply_to_socket_x_recvtimeout_lands() {
    let u = SrtUrl::parse("srt://1.2.3.4:9000?x-recvtimeout=5000").unwrap();
    let mut cfg = SocketConfig::default();
    u.overlay.apply_to_socket(&mut cfg);
    assert_eq!(cfg.recv_timeout, Some(Duration::from_millis(5000)));
}

#[test]
fn apply_to_listener_subset() {
    // ListenerConfig shares many fields with SocketConfig; the apply
    // method writes the overlapping subset.
    let u = SrtUrl::parse("srt://1.2.3.4:9000?latency=100&passphrase=hunter-too-long").unwrap();
    let mut cfg = ListenerConfig::default();
    u.overlay.apply_to_listener(&mut cfg);
    assert_eq!(cfg.latency, Some(Duration::from_millis(100)));
    assert!(cfg.passphrase.is_some());
}
