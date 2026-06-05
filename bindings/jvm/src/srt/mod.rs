//! `org.tstrans.srt` — SRT transport JNI surface.

pub(crate) mod errors;
mod lowlevel;
mod mux_sender;
mod stats;
mod transport;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::{jboolean, jlong};
use tst_core::transport::TransportCancel;

/// Boxed behind a `CancelHandle.handle`. Mirrors tst-py's `PyCancelHandle`:
/// a shared trait-erased cancel target + a per-handle observation flag.
pub(crate) struct JniCancel {
    pub inner: Arc<dyn TransportCancel + Send + Sync>,
    pub flag: AtomicBool,
}

impl JniCancel {
    pub(crate) fn into_handle(self) -> jlong {
        Box::into_raw(Box::new(self)) as jlong
    }

    unsafe fn from_handle<'a>(handle: jlong) -> &'a JniCancel {
        // SAFETY: handle is a valid Box<JniCancel> pointer for the lifetime of
        // the object (closed only via nClose which drops it).
        unsafe { &*(handle as *const JniCancel) }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_CancelHandle_nCancel(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "CancelHandle is closed");
        return;
    }
    // SAFETY: handle is a valid Box<JniCancel> kept alive by the Java object.
    let c = unsafe { JniCancel::from_handle(handle) };
    c.flag.store(true, Ordering::Release);
    c.inner.cancel();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_CancelHandle_nIsCancelled(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jboolean {
    if handle == 0 {
        let _ = env.throw_new("java/lang/IllegalStateException", "CancelHandle is closed");
        return 0;
    }
    // SAFETY: handle is a valid Box<JniCancel> kept alive by the Java object.
    let c = unsafe { JniCancel::from_handle(handle) };
    u8::from(c.flag.load(Ordering::Acquire))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_srt_CancelHandle_nClose(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if handle != 0 {
        // SAFETY: handle is a valid Box<JniCancel>; close() zeroes the field, so
        // this runs at most once per handle.
        drop(unsafe { Box::from_raw(handle as *mut JniCancel) });
    }
}
