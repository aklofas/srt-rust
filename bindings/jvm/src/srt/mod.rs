//! `org.tstrans.srt` — SRT transport JNI surface.

mod demux_receiver;
pub(crate) mod errors;
mod lowlevel;
mod managed_basic;
mod managed_convenience;
mod mux_sender;
mod stats;
mod transport;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::{jboolean, jlong};
use tst_core::transport::TransportCancel;
use tst_pipeline::{BackoffStrategy, OverflowPolicy, ReconnectPolicy};

/// Reconstruct a `tst_pipeline::ReconnectPolicy` from the primitive args the
/// JVM `Managed*.nFromUrl` natives marshal (see `org.tstrans.srt.PolicyArgs`).
/// `backoff_kind`: 0 = Constant, else Exponential. `overflow_policy`: 0 =
/// DropOldest, else Reject. `max_attempts_present == false` -> retry forever.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_reconnect_policy(
    max_attempts_present: bool,
    max_attempts: i32,
    backoff_kind: i32,
    backoff_base_ms: i64,
    backoff_max_ms: i64,
    gap_buffer_capacity: i32,
    overflow_policy: i32,
) -> ReconnectPolicy {
    let backoff = if backoff_kind == 0 {
        BackoffStrategy::Constant(Duration::from_millis(backoff_base_ms.max(0) as u64))
    } else {
        BackoffStrategy::Exponential {
            base: Duration::from_millis(backoff_base_ms.max(0) as u64),
            max: Duration::from_millis(backoff_max_ms.max(0) as u64),
        }
    };
    ReconnectPolicy {
        max_attempts: if max_attempts_present {
            Some(max_attempts.max(0) as u32)
        } else {
            None
        },
        backoff,
        gap_buffer_capacity: gap_buffer_capacity.max(1) as usize,
        overflow_policy: if overflow_policy == 0 {
            OverflowPolicy::DropOldest
        } else {
            OverflowPolicy::Reject
        },
    }
}

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
