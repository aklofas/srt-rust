package org.tstrans.rtp;

import org.tstrans.NativeHandle;

/**
 * Hard-cancel handle for an {@link RtspServer}. {@link #cancel()} aborts every
 * in-flight session at its next poll boundary, bypassing the graceful Notice-5402
 * path. Mirrors tst-py {@code tstrans.rtp.RtspServerCancelHandle}. All handles
 * obtained from one server share the same backing flag.
 *
 * <p>The native handle is an {@link java.util.concurrent.atomic.AtomicLong}
 * registry key; {@link #close()} claims it atomically with {@code getAndSet(0)},
 * and the leased {@code HandleRegistry} guarantees no use-after-free or
 * double-free for any native call concurrent with {@code close()} — a
 * use-after-close is a clean {@link IllegalStateException}, never UB. The methods
 * remain {@code synchronized} only to keep the per-handle {@code isCancelled()}
 * observation flag consistent.
 */
public final class RtspServerCancelHandle extends NativeHandle {
    static { org.tstrans.NativeLoader.load(); }

    RtspServerCancelHandle(long h) { setHandle(h); }

    /** Signal hard cancellation. Idempotent. */
    public synchronized void cancel() {
        nCancel(requireOpen("RtspServerCancelHandle is closed"));
    }

    /** True once {@link #cancel()} was called on the backing flag. */
    public synchronized boolean isCancelled() {
        return nIsCancelled(requireOpen("RtspServerCancelHandle is closed"));
    }

    // Preserve synchronized semantics for the cancel/isCancelled/close coordination contract.
    @Override public synchronized void close() { super.close(); }

    @Override protected void nativeClose(long h) { nClose(h); }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
