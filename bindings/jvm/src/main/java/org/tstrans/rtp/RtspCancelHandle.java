package org.tstrans.rtp;

/**
 * RTSP control-plane cancel handle. {@link #cancel()} breaks any in-flight
 * {@code connect}/{@code pause}/{@code play}/{@code teardown} on the originating
 * session out of blocking I/O at the next poll (typically &lt;100&nbsp;ms); that
 * call then throws {@link org.tstrans.RtspException}. Mirrors tst-py
 * {@code tstrans.rtp.RtspCancelHandle}. All handles obtained from one session
 * share the same backing flag.
 *
 * <p>The native handle is an {@link java.util.concurrent.atomic.AtomicLong}
 * registry key; {@link #close()} claims it atomically with {@code getAndSet(0)},
 * and the leased {@code HandleRegistry} guarantees no use-after-free or
 * double-free for any native call concurrent with {@code close()} — a
 * use-after-close is a clean {@link IllegalStateException}, never UB. The methods
 * remain {@code synchronized} only to keep the per-handle {@code isCancelled()}
 * observation flag consistent.
 */
public final class RtspCancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    RtspCancelHandle(long h) { this.handle.set(h); }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { nCancel(ensureOpen()); }

    /** True once {@link #cancel()} was called on the backing flag. */
    public synchronized boolean isCancelled() { return nIsCancelled(ensureOpen()); }

    @Override public synchronized void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    private long ensureOpen() {
        long h = handle.get();
        if (h == 0) throw new IllegalStateException("RtspCancelHandle is closed");
        return h;
    }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
