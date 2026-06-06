//! `org.tstrans.rtp` RTSP client JNI surface — `RtspClient`, `RtspSession`,
//! `RtspCancelHandle`, and the auth/config/stats value types' native backing.
//! Ports tst-py's `bindings/python/src/rtp/client.rs`. Natives added in Tasks 4-5.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jint, jlong};

use secrecy::SecretString;
use tst_core::mpegts::demux::DemuxerConfig;
use tst_rtp::RtspClientBuilder;
use tst_rtp::error::RtspError;
use tst_rtp::rtsp::client::RtspCancelHandle as RustRtspCancel;
use tst_rtp::rtsp::client::RtspClient as RustRtspClient;
use tst_rtp::rtsp::client::session::RtspSession as RustRtspSession;

use crate::mpegts::build_demux_config_from_args;

use super::demux_receiver::demux_receiver_handle_from_transport;
use super::errors::{rtsp_error_to_jvm, throw_rtsp};

/// Boxed behind `org.tstrans.rtp.RtspCancelHandle.handle`. Wraps tst-rtp's
/// self-contained `RtspCancelHandle` (owns its own `Arc<AtomicBool>` flag).
pub(super) struct JniRtspCancel {
    pub(super) inner: RustRtspCancel,
}

impl JniRtspCancel {
    pub(super) fn into_handle(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }
}

/// Flip the cancel flag. Wakes a parked connect/pause/play/teardown at the next
/// ~100 ms poll. Guards a closed (zero) handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspCancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspCancelHandle is closed",
        );
        return;
    }
    // SAFETY: valid Box<JniRtspCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspCancel) };
    c.inner.cancel();
}

/// Report whether the backing flag has been flipped. Guards a closed handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspCancelHandle_nIsCancelled(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspCancelHandle is closed",
        );
        return 0;
    }
    // SAFETY: valid Box<JniRtspCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspCancel) };
    // tst-rtp uses American spelling is_canceled(); the JVM method is isCancelled().
    u8::from(c.inner.is_canceled())
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspCancelHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtspCancel>; close() zeroes the field (runs once).
        drop(unsafe { Box::from_raw(handle as *mut JniRtspCancel) });
    }
}

/// Native backing for `org.tstrans.rtp.RtspSession`. Faithful port of tst-py's
/// `PyRtspSession`: `client` + `session` behind `Arc<Mutex<Option<..>>>` (so the
/// control natives clone the Arc and lock, never holding a `&mut` to the box;
/// `cancel_handle` clones a self-contained handle out from under the lock);
/// `torn_down` so a duplicate teardown is a no-op.
struct JniRtspSession {
    client: Arc<Mutex<Option<RustRtspClient>>>,
    session: Arc<Mutex<Option<RustRtspSession>>>,
    torn_down: Arc<AtomicBool>,
}

/// `RtspClient.nConnect(url, authUser, authPassword, keepalive)` — run
/// OPTIONS/DESCRIBE/SETUP/PLAY, return a `Box<JniRtspSession>` handle, or 0 with a
/// pending `RtspException` on failure. Ports `PyRtspClient::connect`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspClient_nConnect(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    url: JString<'_>,
    auth_user: JString<'_>,
    auth_password: JString<'_>,
    keepalive: jboolean,
) -> jlong {
    let url_str: String = match env.get_string(&url) {
        Ok(s) => s.into(),
        Err(e) => {
            let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
            return 0;
        }
    };

    // 1. Builder construction (URL parse) — the only eager typed RtspError.
    let mut builder = match RtspClientBuilder::new(&url_str) {
        Ok(b) => b,
        Err(e) => {
            rtsp_error_to_jvm(&mut env, &e);
            return 0;
        }
    };

    // 2. Wire credentials when present. SecretString wrapping happens here.
    //    A null Java String arrives as a null JString (is_null()). The algorithm
    //    is NOT passed: tst-rtp's challenge handler picks it from the server's
    //    WWW-Authenticate header (matches tst-py).
    if !auth_user.is_null() {
        let user: String = match env.get_string(&auth_user) {
            Ok(s) => s.into(),
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                return 0;
            }
        };
        let pw: String = if auth_password.is_null() {
            String::new()
        } else {
            match env.get_string(&auth_password) {
                Ok(s) => s.into(),
                Err(e) => {
                    let _ = env.throw_new("java/lang/RuntimeException", e.to_string());
                    return 0;
                }
            }
        };
        builder = builder.auth(user, SecretString::from(pw));
    }

    // 3. keepalive=false disables the auto-keepalive thread.
    if keepalive == 0 {
        builder = builder.no_auto_keepalive(true);
    }

    // 4. Drive the control-plane to PLAY (network I/O). Ports tst-py's
    //    allow_threads closure — JNI has no GIL to release.
    let result: Result<(RustRtspClient, RustRtspSession), RtspError> = (|| {
        let mut client = builder.connect()?;
        let _opts = client.options()?;
        let sdp = client.describe()?;
        let session = client.setup_mp2t_auto(&sdp)?;
        let _info = client.play()?;
        Ok((client, session))
    })();

    let (client, session) = match result {
        Ok(pair) => pair,
        Err(e) => {
            rtsp_error_to_jvm(&mut env, &e);
            return 0;
        }
    };

    Box::into_raw(Box::new(JniRtspSession {
        client: Arc::new(Mutex::new(Some(client))),
        session: Arc::new(Mutex::new(Some(session))),
        torn_down: Arc::new(AtomicBool::new(false)),
    })) as jlong
}

/// `RtspSession.nPause` — clone the client Arc, lock, `pause()`. Ports
/// `PyRtspSession::pause`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nPause(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    session_control(&mut env, handle, |c| c.pause());
}

/// `RtspSession.nPlay`. Ports `PyRtspSession::play`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nPlay(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    session_control(&mut env, handle, |c| c.play().map(|_info| ()));
}

/// `RtspSession.nTeardown` — no-op if already torn down, else `teardown()` and set
/// the flag. Ports `PyRtspSession::teardown`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nTeardown(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "RtspSession is closed");
        return;
    }
    // SAFETY: validated non-zero live `Box<JniRtspSession>`. The contract for every
    // RtspSession `&*ptr` deref, stated once here: we take ONLY `&*ptr` (shared) to
    // clone the `Arc`s — never `&mut *ptr` — so concurrent session methods are
    // memory-safe (the inner `Mutex` serializes the actual mutation). The single
    // genuinely-unsafe race is `nClose` freeing the box vs another method's
    // top-of-call `&*ptr`; that is forbidden by the documented contract: cross-thread
    // stop routes through `RtspCancelHandle` (not `close()`), and `close()` is a
    // same-thread best-effort teardown that must NOT race a concurrent control call.
    // Same accepted bounded-window contract as srt `Sender`/`Receiver` (see
    // `RtspSession.java`'s class javadoc).
    let s = unsafe { &*(handle as *const JniRtspSession) };
    if s.torn_down.load(Ordering::Relaxed) {
        return;
    }
    let client = s.client.clone();
    let torn = s.torn_down.clone();
    let res = (|| -> Result<(), RtspError> {
        let mut guard = client.lock().map_err(|_| RtspError::SessionExpired)?;
        let r = match guard.as_mut() {
            Some(c) => c.teardown(),
            None => Ok(()),
        };
        torn.store(true, Ordering::Relaxed);
        r
    })();
    if let Err(e) = res {
        rtsp_error_to_jvm(&mut env, &e);
    }
}

/// Shared control-call helper for pause/play. Clones the client Arc, locks, and
/// runs `op`; maps any RtspError.
fn session_control(
    env: &mut JNIEnv,
    handle: jlong,
    op: impl FnOnce(&mut RustRtspClient) -> Result<(), RtspError>,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "RtspSession is closed");
        return;
    }
    // SAFETY: validated non-zero live `Box<JniRtspSession>`; `&*ptr` to clone the
    // `Arc` only — see `nTeardown` for the full bounded-window contract.
    let s = unsafe { &*(handle as *const JniRtspSession) };
    let client = s.client.clone();
    let res = (|| -> Result<(), RtspError> {
        let mut guard = client.lock().map_err(|_| RtspError::SessionExpired)?;
        match guard.as_mut() {
            Some(c) => op(c),
            None => Err(RtspError::SessionExpired),
        }
    })();
    if let Err(e) = res {
        rtsp_error_to_jvm(env, &e);
    }
}

/// `RtspSession.nCancelHandle` — clone a self-contained cancel handle out of the
/// client. Returns 0 (no throw) when torn down (the Java side converts that to
/// IllegalStateException). Ports `PyRtspSession::cancel_handle`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nCancelHandle(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jlong {
    if handle == 0 {
        return 0;
    }
    // SAFETY: validated non-zero live `Box<JniRtspSession>`; `&*ptr` to clone the
    // `Arc` only — see `nTeardown` for the full bounded-window contract.
    let s = unsafe { &*(handle as *const JniRtspSession) };
    let guard = match s.client.lock() {
        Ok(g) => g,
        Err(_) => return 0,
    };
    match guard.as_ref() {
        Some(c) => JniRtspCancel {
            inner: c.cancel_handle(),
        }
        .into_handle(),
        None => 0,
    }
}

/// `RtspSession.nIntoDemuxReceiver` — take the SETUP-time RtspSession, convert to
/// an RtpRecvTransport, build a wave-B DemuxReceiver handle. Double-consume →
/// RtspException(PROTOCOL) + 0. Ports `PyRtspSession::into_demux_receiver`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nIntoDemuxReceiver(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
    with_config: jboolean,
    strict: jint,
    pes_cap_per_pid: jlong,
    pes_cap_total: jlong,
    cfi: jboolean,
    av1: jint,
    au_cell_cap: jlong,
    lenient_psi: jboolean,
) -> jlong {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "RtspSession is closed");
        return 0;
    }
    // SAFETY: validated non-zero live `Box<JniRtspSession>`; `&*ptr` to clone/lock the
    // `Arc` only — see `nTeardown` for the full bounded-window contract.
    let s = unsafe { &*(handle as *const JniRtspSession) };
    // Take the data-plane RtspSession; double-consume = protocol error.
    let session = {
        let mut guard = match s.session.lock() {
            Ok(g) => g,
            Err(_) => {
                throw_rtsp(&mut env, "PROTOCOL", "RtspSession lock poisoned");
                return 0;
            }
        };
        match guard.take() {
            Some(sess) => sess,
            None => {
                throw_rtsp(
                    &mut env,
                    "PROTOCOL",
                    "RtspSession.intoDemuxReceiver: already consumed",
                );
                return 0;
            }
        }
    };
    let transport = session.into_recv_transport();
    let opts: Option<DemuxerConfig> = if with_config != 0 {
        Some(build_demux_config_from_args(
            strict,
            pes_cap_per_pid,
            pes_cap_total,
            cfi,
            av1,
            au_cell_cap,
            lenient_psi,
        ))
    } else {
        None
    };
    demux_receiver_handle_from_transport(transport, opts)
}

/// `RtspSession.nIsTornDown`. Ports `PyRtspSession::is_torn_down`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nIsTornDown(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        return 1;
    }
    // SAFETY: validated non-zero live `Box<JniRtspSession>`; `&*ptr` to read the
    // atomic flag only — see `nTeardown` for the full bounded-window contract.
    let s = unsafe { &*(handle as *const JniRtspSession) };
    u8::from(s.torn_down.load(Ordering::Relaxed))
}

/// `RtspSession.nClose` — best-effort teardown (swallow errors, like tst-py's
/// `__exit__`), then free the box.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspSession_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle from Box::into_raw, dropped once (Java zeroes its field).
        let b = unsafe { Box::from_raw(handle as *mut JniRtspSession) };
        if !b.torn_down.load(Ordering::Relaxed) {
            if let Ok(mut guard) = b.client.lock() {
                if let Some(c) = guard.as_mut() {
                    let _ = c.teardown(); // best-effort
                }
            }
            b.torn_down.store(true, Ordering::Relaxed);
        }
        drop(b);
    }
}
