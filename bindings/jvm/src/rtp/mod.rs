//! `org.tstrans.rtp` — RTP transport JNI surface.

pub(crate) mod errors;
mod client;
mod demux_receiver;
mod mux_sender;
mod server;
mod stats;
mod transport;

use std::sync::Arc;
use std::sync::LazyLock;

use jni::JNIEnv;
use jni::objects::JClass;
use jni::sys::jlong;
use tst_core::transport::TransportCancel;

use crate::handle::HandleRegistry;

/// Boxed behind `org.tstrans.rtp.CancelHandle.handle`. Mirrors tst-py's rtp
/// `PyCancelHandle`: a shared trait-erased cancel target. Unlike the srt
/// `JniCancel` there is NO observation flag — tst-py's rtp `CancelHandle`
/// exposes only `cancel()`.
pub(crate) struct JniRtpCancel {
    pub inner: Arc<dyn TransportCancel + Send + Sync>,
}

/// Per-type leased-handle registry for `org.tstrans.rtp.CancelHandle`. A cancel
/// target (no parked op to wake) — register with `insert` (cancel = None).
static REGISTRY_CANCEL: LazyLock<HandleRegistry<JniRtpCancel>> = LazyLock::new(HandleRegistry::new);

impl JniRtpCancel {
    pub(crate) fn into_handle(self) -> jlong {
        REGISTRY_CANCEL.insert(self) as jlong
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
    crate::panic::jni_catch(&mut env, (), |env| {
        if REGISTRY_CANCEL
            .with(handle as u64, |c| c.inner.cancel())
            .is_none()
        {
            crate::error::throw_closed(env, "CancelHandle");
        }
    })
}

/// Free the boxed cancel handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_tstrans_rtp_CancelHandle_nClose(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    // Atomic + idempotent drop.
    crate::panic::jni_catch(&mut env, (), |_env| {
        let _ = REGISTRY_CANCEL.close(handle as u64);
    })
}
