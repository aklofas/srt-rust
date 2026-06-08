package org.tstrans.rtp;

/**
 * Cross-thread cancel handle for an RTP {@link Sender} / {@link Receiver}.
 * {@link #cancel()} wakes a thread parked in {@code send}/{@code recv} within
 * ~100 ms; that call then throws {@link org.tstrans.RtpException} with kind
 * {@code CANCELLED}. Mirrors tst-py {@code tstrans.rtp.CancelHandle} — which
 * exposes only {@code cancel()} (no {@code isCancelled}).
 *
 * <p>The native handle is an {@link java.util.concurrent.atomic.AtomicLong}
 * registry key; {@link #close()} claims it atomically with {@code getAndSet(0)},
 * and the leased {@code HandleRegistry} guarantees no use-after-free or
 * double-free for any native call concurrent with {@code close()} — a
 * use-after-close is a clean {@link IllegalStateException}, never UB.
 */
public final class CancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    CancelHandle(long h) { this.handle.set(h); }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { nCancel(ensureOpen()); }

    @Override public synchronized void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    private long ensureOpen() {
        long h = handle.get();
        if (h == 0) throw new IllegalStateException("CancelHandle is closed");
        return h;
    }

    private static native void nCancel(long handle);
    private static native void nClose(long handle);
}
