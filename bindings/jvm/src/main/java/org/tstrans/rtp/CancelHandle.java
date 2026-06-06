package org.tstrans.rtp;

/**
 * Cross-thread cancel handle for an RTP {@link Sender} / {@link Receiver}.
 * {@link #cancel()} wakes a thread parked in {@code send}/{@code recv} within
 * ~100 ms; that call then throws {@link org.tstrans.RtpException} with kind
 * {@code CANCELLED}. Mirrors tst-py {@code tstrans.rtp.CancelHandle} — which
 * exposes only {@code cancel()} (no {@code isCancelled}).
 *
 * <p>{@link #cancel()} and {@link #close()} are {@code synchronized} on this
 * instance to guard the cross-thread close/cancel race: {@code close()} may run
 * on one thread while another is still inside {@code cancel()}.
 */
public final class CancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<JniRtpCancel>; 0 = closed

    CancelHandle(long handle) { this.handle = handle; }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { ensureOpen(); nCancel(handle); }

    @Override public synchronized void close() { if (handle != 0) { nClose(handle); handle = 0; } }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("CancelHandle is closed");
    }

    private static native void nCancel(long handle);
    private static native void nClose(long handle);
}
