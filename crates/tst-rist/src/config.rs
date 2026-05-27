//! [`RistConfig`] + [`RistProfile`] + encryption keys.

use std::time::Duration;

use crate::url::RistUrl;

/// RIST profile. See VSF TR-06-1 (Simple) and TR-06-2 (Main).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RistProfile {
    /// Simple Profile — basic ARQ + multiplex. TR-06-1.
    Simple,
    /// Main Profile — adds encryption, RTCP, tunneling. TR-06-2.
    Main,
}

/// AES PSK with explicit key size.
#[derive(Debug, Clone)]
pub struct EncryptionKey {
    pub size_bits: u32,
    pub secret: String,
    /// Optional key-rotation interval (librist `key_rotation` field, in packet count).
    /// 0 = no rotation.
    pub rotation: u32,
}

impl EncryptionKey {
    /// AES-128 PSK.
    pub fn aes128(secret: impl Into<String>) -> Self {
        Self {
            size_bits: 128,
            secret: secret.into(),
            rotation: 0,
        }
    }
    /// AES-192 PSK.
    pub fn aes192(secret: impl Into<String>) -> Self {
        Self {
            size_bits: 192,
            secret: secret.into(),
            rotation: 0,
        }
    }
    /// AES-256 PSK.
    pub fn aes256(secret: impl Into<String>) -> Self {
        Self {
            size_bits: 256,
            secret: secret.into(),
            rotation: 0,
        }
    }
    /// Set the key-rotation packet count.
    pub fn rotation(mut self, count: u32) -> Self {
        self.rotation = count;
        self
    }
}

/// Per-transport librist configuration.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RistConfig {
    pub profile: RistProfile,
    /// Sender bandwidth cap, kbps.
    pub bandwidth_kbps: Option<u32>,
    /// Recovery buffer.
    pub buffer: Duration,
    /// Encryption (None = unencrypted). Forces Main profile when Some.
    pub encryption: Option<EncryptionKey>,
    /// RTCP CNAME.
    pub cname: Option<String>,
    /// Retransmit bandwidth cap, kbps.
    pub recovery_maxbitrate_kbps: Option<u32>,
    /// Receiver session timeout.
    pub session_timeout: Option<Duration>,
    /// NULL-packet deletion / compression.
    pub compression: bool,
    /// Per-send-call payload cap. Default 1316 (7 × 188, matches ffmpeg).
    pub pkt_size: usize,
}

impl Default for RistConfig {
    fn default() -> Self {
        Self {
            profile: RistProfile::Main,
            bandwidth_kbps: None,
            buffer: Duration::from_millis(200),
            encryption: None,
            cname: None,
            recovery_maxbitrate_kbps: None,
            session_timeout: None,
            compression: false,
            pkt_size: 7 * 188,
        }
    }
}

impl RistConfig {
    /// Default per-send-call payload cap (1316 bytes; 7 × 188).
    pub const DEFAULT_PKT_SIZE: usize = 7 * 188;

    /// Overlay URL-derived values on top of an existing config.
    /// Setting any encryption param promotes profile to Main.
    pub fn merge_from_url(&mut self, url: &RistUrl) {
        if let Some(p) = url.profile {
            self.profile = p;
        }
        if let Some(b) = url.bandwidth_kbps {
            self.bandwidth_kbps = Some(b);
        }
        if let Some(d) = url.buffer_ms {
            self.buffer = d;
        }
        if let (Some(bits), Some(secret)) = (url.aes_type, &url.secret) {
            self.encryption = Some(EncryptionKey {
                size_bits: bits,
                secret: secret.clone(),
                rotation: 0,
            });
            self.profile = RistProfile::Main;
        }
        if let Some(c) = &url.cname {
            self.cname = Some(c.clone());
        }
        if let Some(b) = url.recovery_maxbitrate_kbps {
            self.recovery_maxbitrate_kbps = Some(b);
        }
        if let Some(ms) = url.session_timeout_ms {
            self.session_timeout = Some(Duration::from_millis(ms as u64));
        }
        if let Some(c) = url.compression {
            self.compression = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_main_profile() {
        let cfg = RistConfig::default();
        assert_eq!(cfg.profile, RistProfile::Main);
        assert_eq!(cfg.pkt_size, RistConfig::DEFAULT_PKT_SIZE);
    }

    #[test]
    fn merge_from_url_promotes_to_main_on_encryption() {
        // RistConfig is #[non_exhaustive]; struct expression with
        // ..RistConfig::default() works because we're INSIDE the defining
        // crate (Rust RFC 2008).
        let mut cfg = RistConfig {
            profile: RistProfile::Simple,
            ..RistConfig::default()
        };
        let u = RistUrl::parse("rist://1.2.3.4:8000?aes-type=256&secret=s").unwrap();
        cfg.merge_from_url(&u);
        assert_eq!(cfg.profile, RistProfile::Main);
        assert!(cfg.encryption.is_some());
    }

    #[test]
    fn encryption_key_builder() {
        let k = EncryptionKey::aes256("abc").rotation(1000);
        assert_eq!(k.size_bits, 256);
        assert_eq!(k.rotation, 1000);
    }
}
