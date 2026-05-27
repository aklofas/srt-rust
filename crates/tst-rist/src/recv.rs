//! [`RistRecvTransport`] — RIST receiver impl [`RecvTransport`].

use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::transport::{RecvTransport, TransportError};

use crate::config::RistConfig;
use crate::error::RistError;
use crate::init::ensure_init;
use crate::stats::RistStats;
use crate::transport::{apply_peer_overrides, global_logging_ptr, rist_profile_to_c};
use crate::url::RistUrl;

/// Default per-`recv_bytes` poll interval in milliseconds. Short enough that
/// `close()` from another thread (via `alive` swap) is observed quickly.
const POLL_TIMEOUT_MS: i32 = 100;

/// Receive-side RIST transport.
///
/// Wraps a librist `rist_ctx` configured as a receiver. Per-call recv via
/// `rist_receiver_data_read2` with a short poll timeout + cancel-poll loop.
/// Drop calls `rist_destroy`.
pub struct RistRecvTransport {
    ctx: *mut rist_sys::rist_ctx,
    pkt_size: usize,
    bind_url: String,
    alive: Arc<AtomicBool>,
    stats: RistStats,
}

// Same Send/!Sync reasoning as RistTransport: RecvTransport methods take
// &mut self, so single-threaded mutation is borrow-checker enforced.
unsafe impl Send for RistRecvTransport {}

impl RistRecvTransport {
    /// Build a receiver from a URL using defaults.
    ///
    /// The URL must be a bind form (`rist://@host:port`) per the ffmpeg
    /// convention.
    pub fn listen(url: &str) -> Result<Self, RistError> {
        let parsed = RistUrl::parse(url)?;
        if !parsed.is_recv_bind {
            return Err(RistError::InvalidConfig(
                "URL missing '@' prefix — use rist://@host:port for receivers".into(),
            ));
        }
        let mut cfg = RistConfig::default();
        cfg.merge_from_url(&parsed);
        Self::listen_with_config(&parsed, &cfg)
    }

    /// Build a receiver from a parsed URL + config.
    pub fn listen_with_config(url: &RistUrl, cfg: &RistConfig) -> Result<Self, RistError> {
        let _version = ensure_init()?;

        #[cfg(not(feature = "mbedtls"))]
        if cfg.encryption.is_some() {
            return Err(RistError::EncryptionDisabled);
        }

        let profile = rist_profile_to_c(cfg.profile);
        let logging_settings = global_logging_ptr();

        // ===== Create receiver context =====
        let mut ctx: *mut rist_sys::rist_ctx = std::ptr::null_mut();
        let rc = unsafe {
            rist_sys::rist_receiver_create(&mut ctx, profile, logging_settings)
        };
        if rc != 0 || ctx.is_null() {
            return Err(RistError::ContextCreateFailed);
        }

        // ===== Parse bind URL into peer_config =====
        let bind_url_str = format!("rist://@{}:{}", url.addr, url.port);
        let bind_url_c = CString::new(bind_url_str.clone())
            .map_err(|e| RistError::InvalidConfig(format!("bad bind URL: {e}")))?;

        let mut peer_config: *mut rist_sys::rist_peer_config = std::ptr::null_mut();
        let rc = unsafe {
            rist_sys::rist_parse_address2(bind_url_c.as_ptr(), &mut peer_config)
        };
        if rc != 0 || peer_config.is_null() {
            unsafe { rist_sys::rist_destroy(ctx); }
            return Err(RistError::Ffi {
                code: rc,
                function: "rist_parse_address2",
            });
        }

        // Receiver = listener: initiate_conn=0.
        unsafe {
            (*peer_config).initiate_conn = 0;
        }

        // Reuse the sender-side override application (cname / secret / buffer
        // / etc. apply symmetrically).
        if let Err(e) = apply_peer_overrides(peer_config, cfg) {
            unsafe {
                rist_sys::rist_peer_config_free2(&mut peer_config);
                rist_sys::rist_destroy(ctx);
            }
            return Err(e);
        }

        // ===== Add peer (bind endpoint) =====
        let mut peer: *mut rist_sys::rist_peer = std::ptr::null_mut();
        let rc = unsafe { rist_sys::rist_peer_create(ctx, &mut peer, peer_config) };
        unsafe { rist_sys::rist_peer_config_free2(&mut peer_config); }
        if rc != 0 {
            unsafe { rist_sys::rist_destroy(ctx); }
            return Err(RistError::PeerCreateFailed);
        }

        // ===== Start the session =====
        let rc = unsafe { rist_sys::rist_start(ctx) };
        if rc != 0 {
            unsafe { rist_sys::rist_destroy(ctx); }
            return Err(RistError::Ffi { code: rc, function: "rist_start" });
        }

        Ok(Self {
            ctx,
            pkt_size: cfg.pkt_size,
            bind_url: bind_url_str,
            alive: Arc::new(AtomicBool::new(true)),
            stats: RistStats::default(),
        })
    }

    /// Bind URL the receiver was built against (for diagnostics).
    pub fn bind_url(&self) -> &str {
        &self.bind_url
    }

    /// Current snapshot of cumulative stats.
    pub fn stats(&self) -> RistStats {
        self.stats
    }
}

impl RecvTransport for RistRecvTransport {
    fn recv_bytes(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(TransportError::Closed);
        }

        let mut block: *mut rist_sys::rist_data_block = std::ptr::null_mut();
        let rc = unsafe {
            rist_sys::rist_receiver_data_read2(self.ctx, &mut block, POLL_TIMEOUT_MS)
        };

        if rc < 0 {
            self.alive.store(false, Ordering::Release);
            return Err(TransportError::Broken {
                msg: format!("rist_receiver_data_read2 returned {rc}"),
                errno_code: Some(rc),
            });
        }
        if rc == 0 || block.is_null() {
            // Timeout — transport is alive, nothing to read this tick. Per
            // RecvTransport contract, surface as Backpressure (retryable).
            return Err(TransportError::Backpressure {
                msg: "rist recv timeout".into(),
                errno_code: None,
            });
        }

        // SAFETY: block is non-null per rc > 0; payload + payload_len come
        // from librist's internal ringbuffer and are valid until we call
        // rist_receiver_data_block_free2.
        let (payload_ptr, payload_len) = unsafe {
            ((*block).payload as *const u8, (*block).payload_len)
        };

        let copy_n = payload_len.min(buf.len());
        if copy_n > 0 && !payload_ptr.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(payload_ptr, buf.as_mut_ptr(), copy_n);
            }
        }

        // Always release the block back to librist. After this call block is
        // NULL'd by librist.
        unsafe { rist_sys::rist_receiver_data_block_free2(&mut block); }

        if payload_len > buf.len() {
            // Caller's buffer was too small. librist gives us no partial-read
            // contract, so map this to a Broken-style failure.
            self.alive.store(false, Ordering::Release);
            return Err(TransportError::Broken {
                msg: format!(
                    "rist recv buffer too small: have {}, need {payload_len}",
                    buf.len()
                ),
                errno_code: None,
            });
        }

        self.stats.bytes_received = self.stats.bytes_received.wrapping_add(copy_n as u64);
        self.stats.packets_received = self.stats.packets_received.wrapping_add(1);
        Ok(copy_n)
    }

    fn max_payload(&self) -> usize {
        self.pkt_size
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn close(&mut self) {
        if self.alive.swap(false, Ordering::AcqRel) && !self.ctx.is_null() {
            unsafe { rist_sys::rist_destroy(self.ctx); }
            self.ctx = std::ptr::null_mut();
        }
    }
}

impl Drop for RistRecvTransport {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_bind_url() {
        // Plain rist:// without @ prefix is a sender URL.
        let r = RistRecvTransport::listen("rist://1.2.3.4:0");
        match r {
            Err(RistError::InvalidConfig(msg)) => {
                assert!(
                    msg.contains('@'),
                    "expected '@' diagnostic, got: {msg}"
                );
            }
            Err(_) => { /* other error acceptable (port=0 etc.) */ }
            Ok(_) => panic!("expected error for non-bind URL"),
        }
    }
}
