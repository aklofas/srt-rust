package org.tstrans.rtp;

/**
 * RTSP control-plane cancel handle. {@link #cancel()} breaks any in-flight
 * {@code connect}/{@code pause}/{@code play}/{@code teardown} on the originating
 * session out of blocking I/O at the next poll (typically &lt;100&nbsp;ms); that
 * call then throws {@link org.tstrans.RtspException}. Mirrors tst-py
 * {@code tstrans.rtp.RtspCancelHandle}. All handles obtained from one session
 * share the same backing flag.
 *
 * <p>{@link #cancel()}, {@link #isCancelled()} and {@link #close()} are
 * {@code synchronized} on this instance to guard the cross-thread close/cancel
 * race: {@code close()} may run on one thread while another is inside
 * {@code cancel()}/{@code isCancelled()}. The monitor is this handle's own —
 * distinct from the socket a parked control call blocks on, so no deadlock.
 */
public final class RtspCancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<JniRtspCancel>; 0 = closed

    RtspCancelHandle(long handle) { this.handle = handle; }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { ensureOpen(); nCancel(handle); }

    /** True once {@link #cancel()} was called on the backing flag. */
    public synchronized boolean isCancelled() { ensureOpen(); return nIsCancelled(handle); }

    @Override public synchronized void close() { if (handle != 0) { nClose(handle); handle = 0; } }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("RtspCancelHandle is closed");
    }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
