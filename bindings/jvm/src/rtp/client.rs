//! `org.tstrans.rtp` RTSP client JNI surface — `RtspClient`, `RtspSession`,
//! `RtspCancelHandle`, and the auth/config/stats value types' native backing.
//! Ports tst-py's `bindings/python/src/rtp/client.rs`. Natives added in Tasks 4-5.

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::{jboolean, jlong};

use tst_rtp::rtsp::client::RtspCancelHandle as RustRtspCancel;

/// Boxed behind `org.tstrans.rtp.RtspCancelHandle.handle`. Wraps tst-rtp's
/// self-contained `RtspCancelHandle` (owns its own `Arc<AtomicBool>` flag).
// constructed by the RTSP client natives (Task 5)
#[allow(dead_code)]
pub(super) struct JniRtspCancel {
    pub(super) inner: RustRtspCancel,
}

impl JniRtspCancel {
    // constructed by the RTSP client natives (Task 5)
    #[allow(dead_code)]
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
