package org.tstrans.rtp;

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
public final class RtspServerCancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    RtspServerCancelHandle(long h) { this.handle.set(h); }

    /** Signal hard cancellation. Idempotent. */
    public synchronized void cancel() { nCancel(ensureOpen()); }

    /** True once {@link #cancel()} was called on the backing flag. */
    public synchronized boolean isCancelled() { return nIsCancelled(ensureOpen()); }

    @Override public synchronized void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    private long ensureOpen() {
        long h = handle.get();
        if (h == 0) throw new IllegalStateException("RtspServerCancelHandle is closed");
        return h;
    }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
