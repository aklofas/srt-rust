//! `org.tstrans.rtp` RTSP SERVER JNI surface — `RtspServer`, `MountHandle`,
//! `RtspServerCancelHandle`. Ports tst-py's `bindings/python/src/rtp/server.rs`.
//! The underlying `tst_rtp::rtsp::server::RtspServer` owns a tokio Runtime inside
//! the native Box; there is no JNI-side async handling.

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::{jboolean, jlong};

use tst_rtp::RtspServerCancelHandle as RustServerCancel;

/// Boxed behind `org.tstrans.rtp.RtspServerCancelHandle.handle`. Wraps tst-rtp's
/// `RtspServerCancelHandle` (Clone; owns its own `Arc<AtomicBool>` flag, so it is
/// independent of the `RtspServer` box lifetime).
pub(super) struct JniRtspServerCancel {
    pub(super) inner: RustServerCancel,
}

impl JniRtspServerCancel {
    #[allow(dead_code)] // called by RtspServer::cancelHandle() — lands in a later task
    pub(super) fn into_handle(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }
}

/// Fire the HARD cancel. Guards a closed (zero) handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspServerCancelHandle is closed",
        );
        return;
    }
    // SAFETY: valid Box<JniRtspServerCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspServerCancel) };
    c.inner.cancel();
}

/// Report whether the flag was flipped. Guards a closed handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nIsCancelled(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        let _ = env.throw_new(
            "java/lang/IllegalStateException",
            "RtspServerCancelHandle is closed",
        );
        return 0;
    }
    // SAFETY: valid Box<JniRtspServerCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtspServerCancel) };
    // tst-rtp uses American spelling is_canceled(); the JVM method is isCancelled().
    u8::from(c.inner.is_canceled())
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_RtspServerCancelHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtspServerCancel>; close() zeroes the field (runs once).
        drop(unsafe { Box::from_raw(handle as *mut JniRtspServerCancel) });
    }
}
