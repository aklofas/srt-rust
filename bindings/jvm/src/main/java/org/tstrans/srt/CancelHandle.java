package org.tstrans.srt;

import org.tstrans.NativeHandle;

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
public final class CancelHandle extends NativeHandle {
    static { org.tstrans.NativeLoader.load(); }

    CancelHandle(long h) { setHandle(h); }

    /** Signal cancellation. Idempotent. */
    public synchronized void cancel() { nCancel(requireOpen("CancelHandle is closed")); }

    /** True once {@link #cancel()} was called on this handle (advisory). */
    public synchronized boolean isCancelled() {
        return nIsCancelled(requireOpen("CancelHandle is closed"));
    }

    // Preserve synchronized semantics for the cancel/isCancelled/close coordination contract.
    @Override public synchronized void close() { super.close(); }

    @Override protected void nativeClose(long h) { nClose(h); }

    private static native void nCancel(long handle);
    private static native boolean nIsCancelled(long handle);
    private static native void nClose(long handle);
}
