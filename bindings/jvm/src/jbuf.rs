//! Zero-copy payload buffers (spec §5.4). A `GenBuffer` owns a Rust allocation
//! and hands the JVM a *direct* `java.nio.ByteBuffer` over it. The buffer is
//! valid until `invalidate()` bumps the generation (next event pulled / parent
//! released). Java holds the generation it was minted at; a read after
//! invalidation is caught Java-side and raised as IllegalStateException.
//!
//! For the keystone we expose the minting helper; the Java-side generation
//! guard lands with the typed payload in the mpegts-completion wave. Until
//! then callers copy (see Task 1 notes).

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
