package org.tstrans.srt;

/**
 * Cancel handle for a {@link Sender} / {@link Receiver} / {@link Listener}.
 * {@link #cancel()} wakes a thread parked in {@code sendBytes}/{@code recvBytes}/
 * {@code accept} within a few ms; that call then throws
 * {@link org.tstrans.SrtException} with kind {@code BROKEN} or {@code CLOSED}.
 * Mirrors tst-py {@code tstrans.srt.CancelHandle}: {@link #isCancelled()} is a
 * per-handle observation flag; all clones forward {@code cancel()} into the same
 * shared native target.
 *
 * <p>The native handle is an {@link java.util.concurrent.atomic.AtomicLong}
 * registry key; {@link #close()} claims it atomically with {@code getAndSet(0)},
 * and the leased {@code HandleRegistry} guarantees no use-after-free or
 * double-free for any native call concurrent with {@code close()} — a
 * use-after-close is a clean {@link IllegalStateException}, never UB. The methods
 * remain {@code synchronized} only to keep the per-handle {@code isCancelled()}
 * observation flag consistent.
 */
public final class CancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    CancelHandle(long h) { this.handle.set(h); }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { nCancel(ensureOpen()); }

    /** True once {@link #cancel()} was called on this handle (advisory). */
    public synchronized boolean isCancelled() { return nIsCancelled(ensureOpen()); }

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
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
