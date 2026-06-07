package org.tstrans.srt;

import java.util.Iterator;
import java.util.NoSuchElementException;
import org.tstrans.NativeLoader;
import org.tstrans.SrtException;

/**
 * Bound SRT listener. Returned by {@link Builder#listen()}. Iterate to consume
 * accepted {@link Socket}s, or call {@link #accept(Integer)} for explicit control.
 *
 * <p>Iteration ends cleanly when {@code cancelHandle().cancel()} is called from
 * another thread — the underlying {@code AcceptError::ListenerClosed} maps to
 * end-of-iteration ({@link Iterator#hasNext()} returns {@code false}). Other accept
 * failures propagate as {@link SrtException} wrapped in {@link RuntimeException}.
 *
 * <pre>{@code
 * try (Listener listener = new Builder("srt://:9000").listener().listen()) {
 *     for (Socket sock : listener) {
 *         try (sock) {
 *             Receiver r = sock.intoReceiver();
 *             // handle r on a new thread …
 *         }
 *     }
 * }
 * }</pre>
 */
public final class Listener implements AutoCloseable, Iterable<Socket> {

    static { NativeLoader.load(); }

    /**
     * Box&lt;JniListener&gt; pointer; 0 = closed.
     *
     * <p>{@code volatile} because {@link #close()} may be called from a different
     * thread than the one parked in {@link #accept(Integer)} / iterating, and the
     * close-vs-accept handoff relies on a consistent view of this field across
     * threads. See {@link #close()} for the threading contract.
     */
    private volatile long handle;

    /** Package-private: constructed by Builder only. */
    Listener(long handle) {
        this.handle = handle;
    }

    /**
     * Block until an incoming peer completes the SRT handshake, then return the
     * accepted {@link Socket}.
     *
     * @param timeoutMs block at most this many milliseconds; {@code null} blocks indefinitely.
     * @throws SrtException with {@code Kind.TIMEOUT} if the timeout expires;
     *                      with {@code Kind.CLOSED} if the listener was cancelled;
     *                      with {@code Kind.ACCEPT_FAILED} for other accept errors.
     */
    public Socket accept(Integer timeoutMs) throws SrtException {
        ensureOpen();
        return new Socket(nAccept(handle, timeoutMs == null ? -1L : (long) timeoutMs));
    }

    /**
     * Return a cancel handle for this listener. Calling {@link CancelHandle#cancel()} from
     * any thread wakes any thread parked in {@link #accept(Integer)} — the parked call
     * returns {@code SrtException(Kind.CLOSED)}. The iterator converts that to clean
     * end-of-iteration.
     */
    public CancelHandle cancelHandle() {
        ensureOpen();
        return new CancelHandle(nCancelHandle(handle));
    }

    /**
     * Local bound address as a {@link HostPort} record. Useful when the URL
     * requested port 0 (kernel-assigned).
     *
     * @throws SrtException if the handle is invalid ({@code IO}).
     */
    public HostPort localAddr() throws SrtException {
        ensureOpen();
        return nLocalAddr(handle);
    }

    /** {@code true} while this listener still owns the native handle. */
    public boolean isAlive() {
        return handle != 0;
    }

    /**
     * Close the listener and free the native handle. Idempotent.
     *
     * <p><b>Threading contract.</b> A {@link Listener} has a single owner: the thread
     * that calls {@link #accept(Integer)} / iterates is the owner. {@code close()} is
     * intended to be called by that owner, or while no {@code accept()} is in flight.
     *
     * <p>{@code close()} may also be called concurrently with a thread parked in
     * {@code accept()} <em>only to terminate it</em>: it wakes the parked accept (which
     * returns {@code SrtException(Kind.CLOSED)}) and frees the native allocation only
     * once that parked accept has unwound — so it is memory-safe against a parked
     * accept. It is <b>not</b> safe to race {@code close()} against a <em>fresh</em>
     * {@code accept()} / iterator {@code next()} entry on another thread; that violates
     * the single-owner contract.
     *
     * <p>For a purely cross-thread wake that does not itself free the listener — e.g.
     * to stop an iterator loop from a control thread — prefer
     * {@link #cancelHandle()}{@code .cancel()}, which the iterator converts to clean
     * end-of-iteration. {@code cancelHandle().cancel()} never frees the listener; the
     * owning thread still calls {@code close()} (typically via try-with-resources) to
     * release the native handle.
     */
    @Override
    public void close() {
        if (handle != 0) {
            nClose(handle);
            handle = 0;
        }
    }

    /**
     * Return an iterator over accepted {@link Socket}s.
     *
     * <p>Each call to {@link Iterator#next()} blocks indefinitely until a peer
     * connects. The iterator terminates cleanly (returns {@code false} from
     * {@link Iterator#hasNext()}) when the listener is cancelled via
     * {@link CancelHandle#cancel()} — the underlying {@code CLOSED} exception is
     * caught and converted to end-of-iteration. Any other {@link SrtException} is
     * rethrown wrapped in {@link RuntimeException}, matching the idiom used by
     * {@link org.tstrans.mpegts.Demuxer#iterator()}.
     */
    @Override
    public Iterator<Socket> iterator() {
        return new Iterator<>() {
            // One-slot look-ahead: null means "not yet fetched"; a non-null Socket
            // is the next item to deliver; `done` means iteration has ended.
            private Socket next = null;
            private boolean done = false;

            private void advance() {
                if (next != null || done) return;
                try {
                    next = accept(null);
                } catch (SrtException e) {
                    if (e.kind() == SrtException.Kind.CLOSED) {
                        done = true;
                    } else {
                        throw new RuntimeException(e);
                    }
                }
            }

            @Override
            public boolean hasNext() {
                advance();
                return next != null;
            }

            @Override
            public Socket next() {
                advance();
                if (next == null) throw new NoSuchElementException();
                Socket s = next;
                next = null;
                return s;
            }
        };
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("Listener is closed");
    }

    // --- Natives ---

    /**
     * Block for up to {@code timeoutMs} ms (negative = indefinitely) and return
     * a Box&lt;Socket&gt; handle on success; throws SrtException and returns 0 on error.
     */
    private static native long nAccept(long handle, long timeoutMs) throws SrtException;

    /** Return a Box&lt;JniCancel&gt; handle wrapping the listener's cancel handle. */
    private static native long nCancelHandle(long handle);

    private static native HostPort nLocalAddr(long handle) throws SrtException;

    private static native void nClose(long handle);
}
