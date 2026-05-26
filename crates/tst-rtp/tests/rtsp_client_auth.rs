#![allow(clippy::field_reassign_with_default)]

mod fixtures;
use fixtures::rtsp_loopback_server::*;

#[test]
fn basic_auth_succeeds() {
    let mut cfg = FixtureConfig::default();
    cfg.auth = AuthMode::Basic;
    let h = FixtureHandle::spawn(cfg);
    let url = format!("rtsp://admin:secret@127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let _opts = client.options().unwrap();
    let _sdp = client.describe().unwrap();
    drop(client);
    drop(h);
}

#[test]
fn digest_md5_auth_succeeds() {
    let mut cfg = FixtureConfig::default();
    cfg.auth = AuthMode::DigestMd5;
    let h = FixtureHandle::spawn(cfg);
    let url = format!("rtsp://admin:secret@127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let _sdp = client.describe().unwrap();
    drop(client);
    drop(h);
}

#[test]
fn digest_sha256_auth_succeeds() {
    let mut cfg = FixtureConfig::default();
    cfg.auth = AuthMode::DigestSha256;
    let h = FixtureHandle::spawn(cfg);
    let url = format!("rtsp://admin:secret@127.0.0.1:{}/test", h.port);
    let mut client = tst_rtp::RtspClient::connect(&url).unwrap();
    let _sdp = client.describe().unwrap();
    drop(client);
    drop(h);
}
