//! `org.tstrans.rtp` — RTP transport JNI surface.

pub(crate) mod errors;
mod client;
mod demux_receiver;
mod mux_sender;
mod stats;
mod transport;

use std::sync::Arc;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::jlong;
use tst_core::transport::TransportCancel;

/// Boxed behind `org.tstrans.rtp.CancelHandle.handle`. Mirrors tst-py's rtp
/// `PyCancelHandle`: a shared trait-erased cancel target. Unlike the srt
/// `JniCancel` there is NO observation flag — tst-py's rtp `CancelHandle`
/// exposes only `cancel()`.
pub(crate) struct JniRtpCancel {
    pub inner: Arc<dyn TransportCancel + Send + Sync>,
}

impl JniRtpCancel {
    pub(crate) fn into_handle(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }
}

/// Signal cancellation. Wakes a thread parked in `Sender.send` / `Receiver.recv`
/// at the next ~100 ms cancel-poll tick.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_CancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "CancelHandle is closed");
        return;
    }
    // SAFETY: handle is a valid Box<JniRtpCancel> kept alive by the Java object.
    let c = unsafe { &*(handle as *const JniRtpCancel) };
    c.inner.cancel();
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_CancelHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: valid Box<JniRtpCancel>; close() zeroes the field so this runs once.
        drop(unsafe { Box::from_raw(handle as *mut JniRtpCancel) });
    }
}
