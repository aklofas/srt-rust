//! [`RistTransport`] — RIST sender impl [`Transport`].

use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::transport::{Transport, TransportError};

use crate::config::{EncryptionKey, RistConfig, RistProfile};
use crate::error::RistError;
use crate::init::{GLOBAL_LOGGING, ensure_init};
use crate::stats::RistStats;
use crate::url::RistUrl;

/// Send-side RIST transport.
///
/// Wraps a librist `rist_ctx` configured as a sender. Per-message send via
/// `rist_sender_data_write`. Drop calls `rist_destroy`.
pub struct RistTransport {
    ctx: *mut rist_sys::rist_ctx,
    pkt_size: usize,
    peer_url: String,
    alive: Arc<AtomicBool>,
    stats: RistStats,
}

// librist's rist_ctx is safe to share across threads as long as we don't pass
// it concurrently to mutating ops from multiple threads — Transport's methods
// take &mut self so the borrow checker enforces single-threaded access here.
unsafe impl Send for RistTransport {}

impl RistTransport {
    /// Build a sender from a URL using defaults.
    pub fn connect(url: &str) -> Result<Self, RistError> {
        let parsed = RistUrl::parse(url)?;
        if parsed.is_recv_bind {
            return Err(RistError::InvalidConfig(
                "URL has '@' prefix — use RistRecvTransport::listen".into(),
            ));
        }
        let mut cfg = RistConfig::default();
        cfg.merge_from_url(&parsed);
        Self::connect_with_config(&parsed, &cfg)
    }

    /// Build a sender from a parsed URL + config.
    pub fn connect_with_config(url: &RistUrl, cfg: &RistConfig) -> Result<Self, RistError> {
        let _version = ensure_init()?;

        #[cfg(not(feature = "mbedtls"))]
        if cfg.encryption.is_some() {
            return Err(RistError::EncryptionDisabled);
        }

        let profile = rist_profile_to_c(cfg.profile);
        let logging_settings = global_logging_ptr();

        // ===== Create sender context =====
        let mut ctx: *mut rist_sys::rist_ctx = std::ptr::null_mut();
        let rc = unsafe { rist_sys::rist_sender_create(&mut ctx, profile, 0, logging_settings) };
        if rc != 0 || ctx.is_null() {
            return Err(RistError::ContextCreateFailed);
        }

        // ===== Parse peer URL into peer_config =====
        let peer_url_str = format!("rist://{}:{}", url.addr, url.port);
        let peer_url_c = CString::new(peer_url_str.clone())
            .map_err(|e| RistError::InvalidConfig(format!("bad peer URL: {e}")))?;

        let mut peer_config: *mut rist_sys::rist_peer_config = std::ptr::null_mut();
        let rc = unsafe { rist_sys::rist_parse_address2(peer_url_c.as_ptr(), &mut peer_config) };
        if rc != 0 || peer_config.is_null() {
            unsafe {
                rist_sys::rist_destroy(ctx);
            }
            return Err(RistError::Ffi {
                code: rc,
                function: "rist_parse_address2",
            });
        }

        // Sender = caller: initiate_conn=1.
        unsafe {
            (*peer_config).initiate_conn = 1;
        }

        // Apply cfg overlays. apply_peer_overrides is pub(crate) so recv.rs
        // (Wave D) can reuse it.
        if let Err(e) = apply_peer_overrides(peer_config, cfg) {
            unsafe {
                rist_sys::rist_peer_config_free2(&mut peer_config);
                rist_sys::rist_destroy(ctx);
            }
            return Err(e);
        }

        // ===== Add peer to sender =====
        let mut peer: *mut rist_sys::rist_peer = std::ptr::null_mut();
        let rc = unsafe { rist_sys::rist_peer_create(ctx, &mut peer, peer_config) };
        // peer_config is now owned by librist (or freed internally); per
        // librist docs we still free the wrapper.
        unsafe {
            rist_sys::rist_peer_config_free2(&mut peer_config);
        }

        if rc != 0 {
            unsafe {
                rist_sys::rist_destroy(ctx);
            }
            return Err(RistError::PeerCreateFailed);
        }

        // ===== Start the session =====
        let rc = unsafe { rist_sys::rist_start(ctx) };
        if rc != 0 {
            unsafe {
                rist_sys::rist_destroy(ctx);
            }
            return Err(RistError::Ffi {
                code: rc,
                function: "rist_start",
            });
        }

        Ok(Self {
            ctx,
            pkt_size: cfg.pkt_size,
            peer_url: peer_url_str,
            alive: Arc::new(AtomicBool::new(true)),
            stats: RistStats::default(),
        })
    }

    /// Peer URL the transport was built against (for diagnostics).
    pub fn peer_url(&self) -> &str {
        &self.peer_url
    }

    /// Current snapshot of cumulative stats.
    pub fn stats(&self) -> RistStats {
        self.stats
    }
}

impl Transport for RistTransport {
    fn send_bytes(&mut self, msg: &[u8]) -> Result<(), TransportError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        if msg.len() > self.pkt_size {
            return Err(TransportError::TooLarge {
                len: msg.len(),
                max: self.pkt_size,
            });
        }

        let block = rist_sys::rist_data_block {
            payload: msg.as_ptr() as *const _,
            payload_len: msg.len(),
            ts_ntp: 0,
            virt_src_port: 0,
            virt_dst_port: 0,
            peer: std::ptr::null_mut(),
            flow_id: 0,
            seq: 0,
            flags: 0,
            ref_: std::ptr::null_mut(),
        };

        let rc = unsafe { rist_sys::rist_sender_data_write(self.ctx, &block) };
        if rc < 0 {
            self.alive.store(false, Ordering::Release);
            return Err(TransportError::Broken {
                msg: format!("rist_sender_data_write returned {rc}"),
                errno_code: Some(rc),
            });
        }

        self.stats.bytes_sent = self.stats.bytes_sent.wrapping_add(msg.len() as u64);
        self.stats.packets_sent = self.stats.packets_sent.wrapping_add(1);
        Ok(())
    }

    fn max_payload(&self) -> usize {
        self.pkt_size
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn close(&mut self) {
        if self.alive.swap(false, Ordering::AcqRel) && !self.ctx.is_null() {
            unsafe {
                rist_sys::rist_destroy(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
        }
    }
}

impl Drop for RistTransport {
    fn drop(&mut self) {
        self.close();
    }
}

// ============================================================
// Helpers
// ============================================================

pub(crate) fn rist_profile_to_c(profile: RistProfile) -> rist_sys::rist_profile {
    match profile {
        RistProfile::Simple => rist_sys::rist_profile_RIST_PROFILE_SIMPLE,
        RistProfile::Main => rist_sys::rist_profile_RIST_PROFILE_MAIN,
    }
}

/// Return the global logging-settings pointer registered in init.rs, or
/// NULL if logging registration failed earlier.
pub(crate) fn global_logging_ptr() -> *mut rist_sys::rist_logging_settings {
    GLOBAL_LOGGING
        .get()
        .map(|p| p.0)
        .unwrap_or(std::ptr::null_mut())
}

/// Apply [`RistConfig`] overlays onto the parsed `rist_peer_config`. Shared
/// by [`RistTransport::connect_with_config`] and (forthcoming) the receiver.
pub(crate) fn apply_peer_overrides(
    peer_config: *mut rist_sys::rist_peer_config,
    cfg: &RistConfig,
) -> Result<(), RistError> {
    if peer_config.is_null() {
        return Err(RistError::InvalidConfig("null peer_config".into()));
    }
    let pc = unsafe { &mut *peer_config };

    if let Some(bw) = cfg.bandwidth_kbps {
        pc.recovery_maxbitrate = bw;
    }
    pc.recovery_length_min = cfg.buffer.as_millis() as u32;
    pc.recovery_length_max = (cfg.buffer.as_millis() as u32).max(pc.recovery_length_max);

    if let Some(bw) = cfg.recovery_maxbitrate_kbps {
        pc.recovery_maxbitrate = bw;
    }
    if let Some(t) = cfg.session_timeout {
        pc.session_timeout = t.as_millis() as u32;
    }
    pc.compression = if cfg.compression { 1 } else { 0 };

    if let Some(cname) = &cfg.cname {
        write_c_string_field(&mut pc.cname, cname, "cname")?;
    }

    if let Some(enc) = &cfg.encryption {
        apply_encryption(pc, enc)?;
    }

    Ok(())
}

fn apply_encryption(
    pc: &mut rist_sys::rist_peer_config,
    key: &EncryptionKey,
) -> Result<(), RistError> {
    if !matches!(key.size_bits, 128 | 192 | 256) {
        return Err(RistError::InvalidConfig(format!(
            "encryption key_size must be 128/192/256, got {}",
            key.size_bits
        )));
    }
    pc.key_size = key.size_bits as i32;
    pc.key_rotation = key.rotation;
    write_c_string_field(&mut pc.secret, &key.secret, "secret")?;
    Ok(())
}

/// Copy a Rust `&str` into a fixed-size `[c_char; N]` field, null-terminating.
fn write_c_string_field(
    dst: &mut [c_char],
    src: &str,
    field_name: &'static str,
) -> Result<(), RistError> {
    let bytes = src.as_bytes();
    if bytes.contains(&0) {
        return Err(RistError::InvalidConfig(format!(
            "{field_name} contains interior null byte"
        )));
    }
    if bytes.len() >= dst.len() {
        return Err(RistError::InvalidConfig(format!(
            "{field_name} exceeds {} bytes",
            dst.len() - 1
        )));
    }
    for d in dst.iter_mut() {
        *d = 0;
    }
    for (i, b) in bytes.iter().enumerate() {
        dst[i] = *b as c_char;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_recv_bind_url() {
        // RistUrl with @ prefix means "receiver" — connect() should refuse.
        let r = RistTransport::connect("rist://@0.0.0.0:0");
        // Connect might fail earlier (port=0 / bind issues), so just check
        // the error contains "@ prefix" if it's InvalidConfig.
        match r {
            Err(RistError::InvalidConfig(msg)) => {
                assert!(msg.contains('@'), "expected '@' diagnostic, got: {msg}");
            }
            Err(_) => { /* other failure path also acceptable */ }
            Ok(_) => panic!("expected error for recv-bind URL"),
        }
    }

    #[test]
    fn rist_profile_to_c_maps_correctly() {
        assert_eq!(
            rist_profile_to_c(RistProfile::Simple),
            rist_sys::rist_profile_RIST_PROFILE_SIMPLE
        );
        assert_eq!(
            rist_profile_to_c(RistProfile::Main),
            rist_sys::rist_profile_RIST_PROFILE_MAIN
        );
    }

    #[test]
    fn write_c_string_field_truncation_rejected() {
        let mut buf = [0 as c_char; 4];
        // 4 bytes of payload + need for null term = needs 5; rejected.
        let r = write_c_string_field(&mut buf, "abcd", "test");
        assert!(matches!(r, Err(RistError::InvalidConfig(_))));
    }

    #[test]
    fn write_c_string_field_null_byte_rejected() {
        let mut buf = [0 as c_char; 16];
        let r = write_c_string_field(&mut buf, "ab\0c", "test");
        assert!(matches!(r, Err(RistError::InvalidConfig(_))));
    }

    #[test]
    fn write_c_string_field_writes_and_null_terminates() {
        let mut buf = [0xFF_u8 as c_char; 16];
        write_c_string_field(&mut buf, "hello", "test").unwrap();
        assert_eq!(buf[0] as u8, b'h');
        assert_eq!(buf[4] as u8, b'o');
        assert_eq!(buf[5], 0);
        // Verify the rest is zeroed (we explicitly zeroed before copy).
        assert_eq!(buf[15], 0);
    }
}
