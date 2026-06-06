package org.tstrans.rtp;

/**
 * Hard-cancel handle for an {@link RtspServer}. {@link #cancel()} aborts every
 * in-flight session at its next poll boundary, bypassing the graceful Notice-5402
 * path. Mirrors tst-py {@code tstrans.rtp.RtspServerCancelHandle}. All handles
 * obtained from one server share the same backing flag.
 *
 * <p>{@link #cancel()}, {@link #isCancelled()} and {@link #close()} are
 * {@code synchronized} on this instance to guard the cross-thread close/cancel
 * race: {@code close()} may run on one thread while another is inside
 * {@code cancel()}/{@code isCancelled()}. The monitor is this handle's own —
 * distinct from anything a cancelled call blocks on, so no deadlock.
 */
public final class RtspServerCancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<JniRtspServerCancel>; 0 = closed

    RtspServerCancelHandle(long handle) { this.handle = handle; }

    /** Signal hard cancellation. Idempotent. */
    public synchronized void cancel() { ensureOpen(); nCancel(handle); }

    /** True once {@link #cancel()} was called on the backing flag. */
    public synchronized boolean isCancelled() { ensureOpen(); return nIsCancelled(handle); }

    @Override public synchronized void close() { if (handle != 0) { nClose(handle); handle = 0; } }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("RtspServerCancelHandle is closed");
    }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
