package org.tstrans.srt;

/**
 * Cancel handle for a {@link Sender} / {@link Receiver} / {@link Listener}.
 * {@link #cancel()} wakes a thread parked in {@code sendBytes}/{@code recvBytes}/
 * {@code accept} within a few ms; that call then throws
 * {@link org.tstrans.SrtException} with kind {@code BROKEN} or {@code CLOSED}.
 * Mirrors tst-py {@code tstrans.srt.CancelHandle}: {@link #isCancelled()} is a
 * per-handle observation flag; all clones forward {@code cancel()} into the same
 * shared native target.
 */
public final class CancelHandle implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<JniCancel>; 0 = closed

    CancelHandle(long handle) { this.handle = handle; }

    /** Signal cancellation. Idempotent. */
    public void cancel() { ensureOpen(); nCancel(handle); }

    /** True once {@link #cancel()} was called on this handle (advisory). */
    public boolean isCancelled() { ensureOpen(); return nIsCancelled(handle); }

    @Override public void close() { if (handle != 0) { nClose(handle); handle = 0; } }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("CancelHandle is closed");
    }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
