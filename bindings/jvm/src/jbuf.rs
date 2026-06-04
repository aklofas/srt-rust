//! Zero-copy payload buffers (spec §5.4). `new_direct` hands the JVM a *direct*
//! `java.nio.ByteBuffer` view over caller-owned Rust bytes, without copying. The
//! caller owns the backing storage and keeps it alive for the buffer's valid
//! window; in the mpegts keystone that owner is `JniDemuxer::last_payload`, which
//! is overwritten on the next `nextEvent` pull (the documented
//! "valid-until-next-pull" lifetime, see `DemuxEvent.Sample.payload`).
//!
//! The full generation-counter machinery (a `GenBuffer` wrapper whose
//! `invalidate()` bumps a generation so a stale read becomes a defined
//! `IllegalStateException` rather than undefined behaviour) is a future
//! mpegts-completion addition; this module ships only the minting helper, so
//! until then a read after the next pull is undefined and callers must copy.

use jni::JNIEnv;
use jni::objects::JObject;

/// Mint a direct ByteBuffer view over `bytes`. SAFETY: `bytes` must outlive the
/// returned buffer's use on the Java side (the caller owns the backing storage;
/// the demuxer's event holds it until the next pull). Returns a local ref.
pub fn new_direct<'local>(
    env: &mut JNIEnv<'local>,
    bytes: &mut [u8],
) -> jni::errors::Result<JObject<'local>> {
    // SAFETY: new_direct_byte_buffer wraps the pointer without copying; the
    // buffer must not be read after the backing slice is freed/moved.
    unsafe {
        Ok(env
            .new_direct_byte_buffer(bytes.as_mut_ptr(), bytes.len())?
            .into())
    }
}
