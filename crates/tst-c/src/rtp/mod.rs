//! `tst_rtp_*` C ABI entry points. Gated on `feature = "rtp"`.
//!
//! This module exposes constructors that open RTP transports and return
//! new opaque handle types (`TstRtpSender`, `TstRtpReceiver`,
//! `TstRtpMuxSender`, `TstRtpDemuxReceiver`). Once open, callers can
//! send/receive using the handle-specific data-path entry points added
//! by later tasks (Wave B/C). The handles have their own `_close`
//! entry points to free them.
//!
//! URL form accepted: `rtp://host:port[?key=value&...]`
//! See `tst_rtp::RtpUrl` for the recognized query keys (ttl, iface,
//! pkt_size, ssrc).

mod url;

use std::os::raw::c_char;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use tst_core::{RecvTransport, Transport};
use tst_pipeline::{
    DemuxReceiver, MuxSender, ReceiverConfig, Sender, SenderConfig, TransportCancel,
};
use tst_rtp::{RtpRecvSocketBuilder, RtpRecvTransport, RtpSocketBuilder, RtpTransport};

use crate::config::TstMuxConfig;
use crate::demux_config::TstDemuxConfig;
use crate::error::{TstError, record_mux_error, set_last_error};
use crate::handle::Handle;

// ---------------------------------------------------------------------------
// Handle types
// ---------------------------------------------------------------------------

/// Opaque handle for an RTP-backed raw TS byte sender.
///
/// Returned by [`tst_rtp_sender_open`]. Freed with
/// [`tst_rtp_sender_close`].
pub struct TstRtpSender {
    pub(crate) inner: Handle<Sender<RtpTransport>>,
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

/// Opaque handle for an RTP-backed raw TS byte receiver.
///
/// Returned by [`tst_rtp_recv_open`]. Freed with
/// [`tst_rtp_receiver_close`].
pub struct TstRtpReceiver {
    pub(crate) inner: Handle<tst_pipeline::Receiver<RtpRecvTransport>>,
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

/// Opaque handle for an RTP-backed mux sender.
///
/// Returned by [`tst_rtp_mux_sender_open`]. Freed with
/// [`tst_rtp_mux_sender_close`].
pub struct TstRtpMuxSender {
    pub(crate) inner: Handle<MuxSender<RtpTransport>>,
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

/// Opaque handle for an RTP-backed demux receiver.
///
/// Returned by [`tst_rtp_demux_receiver_open`]. Freed with
/// [`tst_rtp_demux_receiver_close`].
pub struct TstRtpDemuxReceiver {
    pub(crate) inner: Handle<DemuxReceiver<RtpRecvTransport>>,
    /// Reusable backing storage for `tst_rtp_demux_receiver_recv_event` (Wave B).
    /// Allocated at open time so Wave B data-path calls never allocate on the
    /// hot path. Unused until Wave B adds the recv-event entry point.
    #[allow(dead_code)]
    pub(crate) arena: Mutex<crate::event::EventArena>,
    pub(crate) cancel: Option<Arc<dyn TransportCancel + Send + Sync>>,
    pub(crate) was_cancelled: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Open functions
// ---------------------------------------------------------------------------

/// Open an RTP sender to the unicast or multicast endpoint described by
/// `url`. Returns `NULL` on error; check `tst_get_last_error()` for the
/// negative error code and `tst_get_last_error_str()` for a detail message.
///
/// URL form: `rtp://host:port[?ttl=N&iface=eth0&pkt_size=1316&ssrc=N]`.
/// The transport is a UDP socket that sends RTP packets wrapping 7
/// MPEG-TS packets per datagram (RFC 2250 §2). Multicast destinations
/// (`224.0.0.0/4` for IPv4, `ff00::/8` for IPv6) are detected
/// automatically from the destination address.
///
/// # Safety
///
/// `url` must be a NUL-terminated C string valid for the duration of
/// this call. The returned handle must eventually be freed with
/// `tst_rtp_sender_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_sender_open(url: *const c_char) -> *mut TstRtpSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let rtp_url = match unsafe { url::parse_url(url) } {
            Some(u) => u,
            None => return std::ptr::null_mut(),
        };
        // Build the URL string for RtpSocketBuilder::from_url. We
        // re-serialize because `parse_url` returns the structured `RtpUrl`
        // which RtpSocketBuilder::new() can accept directly.
        let builder = RtpSocketBuilder::new(rtp_url.host.clone(), rtp_url.port);
        // Apply parsed query parameters onto the builder.
        let mut builder = builder;
        if let Some(ttl) = rtp_url.ttl {
            builder.ttl(ttl);
        }
        if let Some(ref iface) = rtp_url.iface {
            builder.iface(iface.clone());
        }
        builder.pkt_size(rtp_url.pkt_size);
        if let Some(ssrc) = rtp_url.ssrc {
            builder.ssrc(ssrc);
        }
        let transport = match builder.connect() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(TstError::Transport, &format!("rtp connect: {e}"));
                return std::ptr::null_mut();
            }
        };
        let cancel = transport.cancel_handle();
        let sender = Sender::new(transport, SenderConfig::default());
        Box::into_raw(Box::new(TstRtpSender {
            inner: Handle::new(sender),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

/// Open an RTP receiver listening on the unicast or multicast endpoint
/// described by `url`. Returns `NULL` on error.
///
/// For unicast, pass `rtp://0.0.0.0:port` or `rtp://127.0.0.1:port`
/// (host is the bind address). For multicast, pass the group address
/// (`rtp://239.0.0.1:port?iface=eth0`); the socket joins the group on
/// `iface` (or the OS-default interface when absent).
///
/// Port `0` causes the kernel to assign an ephemeral port; call
/// `tst_rtp_receiver_local_port` (Task 6) to learn the assigned port.
///
/// # Safety
///
/// `url` must be a NUL-terminated C string. The returned handle must
/// eventually be freed with `tst_rtp_receiver_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_recv_open(url: *const c_char) -> *mut TstRtpReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let rtp_url = match unsafe { url::parse_url(url) } {
            Some(u) => u,
            None => return std::ptr::null_mut(),
        };
        let builder = RtpRecvSocketBuilder::new(rtp_url.host.clone(), rtp_url.port);
        let mut builder = builder;
        if let Some(ref iface) = rtp_url.iface {
            builder.iface(iface.clone());
        }
        builder.pkt_size(rtp_url.pkt_size);
        let transport = match builder.listen() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(TstError::Transport, &format!("rtp listen: {e}"));
                return std::ptr::null_mut();
            }
        };
        let cancel = transport.cancel_handle();
        let receiver = tst_pipeline::Receiver::new(transport, ReceiverConfig::default());
        Box::into_raw(Box::new(TstRtpReceiver {
            inner: Handle::new(receiver),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

/// Open an RTP-backed `MuxSender` that muxes MPEG-TS in real time and
/// sends over UDP/RTP. `mux_cfg` must be a valid `tst_mux_config_t`
/// (constructed via `tst_mux_config_new`). Returns `NULL` on error.
///
/// The mux config is borrowed — the caller still owns it and must free
/// it. The returned handle is independent of the config after this call.
///
/// # Safety
///
/// `url` is a NUL-terminated C string. `mux_cfg` must be a non-null
/// pointer to a `tst_mux_config_t` valid for this call. The returned
/// handle must eventually be freed with `tst_rtp_mux_sender_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_mux_sender_open(
    url: *const c_char,
    mux_cfg: *const TstMuxConfig,
) -> *mut TstRtpMuxSender {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let rtp_url = match unsafe { url::parse_url(url) } {
            Some(u) => u,
            None => return std::ptr::null_mut(),
        };
        let cfg_ref = match unsafe { mux_cfg.as_ref() } {
            Some(c) => c,
            None => {
                set_last_error(TstError::InvalidConfig, "mux_cfg is null");
                return std::ptr::null_mut();
            }
        };
        let built = match cfg_ref.build_config() {
            Ok(c) => c,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        let builder = RtpSocketBuilder::new(rtp_url.host.clone(), rtp_url.port);
        let mut builder = builder;
        if let Some(ttl) = rtp_url.ttl {
            builder.ttl(ttl);
        }
        if let Some(ref iface) = rtp_url.iface {
            builder.iface(iface.clone());
        }
        builder.pkt_size(rtp_url.pkt_size);
        if let Some(ssrc) = rtp_url.ssrc {
            builder.ssrc(ssrc);
        }
        let transport = match builder.connect() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(TstError::Transport, &format!("rtp connect: {e}"));
                return std::ptr::null_mut();
            }
        };
        let cancel = transport.cancel_handle();
        let mux_sender = match MuxSender::new(transport, built) {
            Ok(s) => s,
            Err(e) => {
                record_mux_error(&e);
                return std::ptr::null_mut();
            }
        };
        Box::into_raw(Box::new(TstRtpMuxSender {
            inner: Handle::new(mux_sender),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

/// Open an RTP-backed `DemuxReceiver`. `demux_cfg` may be `NULL`, in
/// which case default demux options apply (lenient mode). Returns `NULL`
/// on error.
///
/// # Safety
///
/// `url` is a NUL-terminated C string. `demux_cfg` may be NULL or a
/// valid `tst_demux_config_t*`. The returned handle must eventually be
/// freed with `tst_rtp_demux_receiver_close`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_open(
    url: *const c_char,
    demux_cfg: *const TstDemuxConfig,
) -> *mut TstRtpDemuxReceiver {
    crate::panic::ffi_catch(std::ptr::null_mut(), || {
        let rtp_url = match unsafe { url::parse_url(url) } {
            Some(u) => u,
            None => return std::ptr::null_mut(),
        };
        let builder = RtpRecvSocketBuilder::new(rtp_url.host.clone(), rtp_url.port);
        let mut builder = builder;
        if let Some(ref iface) = rtp_url.iface {
            builder.iface(iface.clone());
        }
        builder.pkt_size(rtp_url.pkt_size);
        let transport = match builder.listen() {
            Ok(t) => t,
            Err(e) => {
                set_last_error(TstError::Transport, &format!("rtp listen: {e}"));
                return std::ptr::null_mut();
            }
        };
        let cancel = transport.cancel_handle();
        let receiver = if let Some(cfg) = unsafe { demux_cfg.as_ref() } {
            DemuxReceiver::with_demux_options(transport, cfg.build_options())
        } else {
            DemuxReceiver::new(transport)
        };
        Box::into_raw(Box::new(TstRtpDemuxReceiver {
            inner: Handle::new(receiver),
            arena: Mutex::new(crate::event::EventArena::new()),
            cancel,
            was_cancelled: Arc::new(AtomicBool::new(false)),
        }))
    })
}

// ---------------------------------------------------------------------------
// Close / free functions
// ---------------------------------------------------------------------------

/// Close and free a `tst_rtp_sender_t`.
///
/// Safe to call with `NULL` (no-op). After this call the pointer is
/// invalid; passing the same non-null pointer twice is undefined
/// behavior (use-after-free on the consumed `Box`).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpSender` returned
/// by `tst_rtp_sender_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_sender_close(p: *mut TstRtpSender) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &boxed.cancel {
            c.cancel();
        }
        boxed.inner.close();
        drop(boxed);
    });
}

/// Close and free a `tst_rtp_receiver_t`.
///
/// Safe to call with `NULL` (no-op). See `tst_rtp_sender_close` for
/// the ownership semantics.
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpReceiver` returned
/// by `tst_rtp_recv_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_receiver_close(p: *mut TstRtpReceiver) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &boxed.cancel {
            c.cancel();
        }
        boxed.inner.close();
        drop(boxed);
    });
}

/// Close and free a `tst_rtp_mux_sender_t`.
///
/// Safe to call with `NULL` (no-op).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpMuxSender` returned
/// by `tst_rtp_mux_sender_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_mux_sender_close(p: *mut TstRtpMuxSender) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &boxed.cancel {
            c.cancel();
        }
        boxed.inner.close();
        drop(boxed);
    });
}

/// Close and free a `tst_rtp_demux_receiver_t`.
///
/// Safe to call with `NULL` (no-op).
///
/// # Safety
///
/// `p` must be NULL or a valid non-freed `*mut TstRtpDemuxReceiver`
/// returned by `tst_rtp_demux_receiver_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tst_rtp_demux_receiver_close(p: *mut TstRtpDemuxReceiver) {
    crate::panic::ffi_catch((), || {
        if p.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(p) };
        boxed.was_cancelled.store(true, Ordering::Release);
        if let Some(c) = &boxed.cancel {
            c.cancel();
        }
        boxed.inner.close();
        drop(boxed);
    });
}
