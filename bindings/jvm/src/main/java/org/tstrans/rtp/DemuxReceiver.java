package org.tstrans.rtp;

import java.util.Iterator;
import java.util.NoSuchElementException;
import java.util.function.Consumer;
import org.tstrans.DemuxException;
import org.tstrans.NativeLoader;
import org.tstrans.RtpException;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.DemuxerConfig;

/**
 * Single-call convenience wrapper that owns a {@code Demuxer} + an RTP
 * {@code RtpRecvTransport}. Construct with an {@code rtp://host:port} URL
 * (unicast or multicast); iterate over the emitted {@link DemuxEvent} instances
 * (the same sealed hierarchy {@code org.tstrans.mpegts.Demuxer} produces).
 *
 * <p>Mirrors {@code tstrans.rtp.DemuxReceiver}. Wraps
 * {@code tst_pipeline::DemuxReceiver<tst_rtp::RtpRecvTransport>}.
 *
 * <p><b>Thread safety:</b> a single {@code DemuxReceiver} is intended to be
 * iterated from one thread. There is no {@code cancelHandle()} on the RTP
 * convenience wrapper (matching tst-py's surface), so the sanctioned cross-thread
 * operation is {@link #close()}: to stop an iteration currently <em>parked</em> in
 * {@code next()} waiting for the next datagram, another thread may call
 * {@code close()} — it cancels the in-flight recv first (waking the parked
 * {@code next()} within ~100&nbsp;ms), then frees the receiver.
 *
 * <p>{@code close()} is memory-safe against ANY concurrent native call: it claims
 * the registry id atomically and the leased {@code HandleRegistry} guarantees no
 * use-after-free/double-free — a racing call either runs or throws a clean
 * {@link IllegalStateException}. Note a <em>fresh</em> {@code next()} entered
 * <em>after</em> {@code close()} fired its cancel hook is not woken by that same
 * hook; it observes the closed handle and throws {@link IllegalStateException}
 * (single-iterator expectation). After {@code close()}, further calls throw
 * {@code IllegalStateException}.
 *
 * <p><b>Byte-copy posture (JDK 17):</b> sample payloads and the byte-sink
 * callbacks deliver heap {@code byte[]} copies; a zero-copy path (FFM
 * {@code MemorySegment}) is JDK-22+ only and will be added in a future release.
 *
 * <pre>{@code
 * try (DemuxReceiver rx = DemuxReceiver.fromUrl("rtp://0.0.0.0:5004")) {
 *     for (DemuxEvent e : rx) {
 *         if (e instanceof DemuxEvent.Video v) { ... }
 *     }
 * }
 * }</pre>
 */
public final class DemuxReceiver implements AutoCloseable, Iterable<DemuxEvent> {
    static { NativeLoader.load(); }

    // AtomicLong registry key. close() is a sanctioned cross-thread operation on
    // this class (the watchdog pattern — see the class javadoc): the handle is read
    // by the iterator thread and claimed (getAndSet) by a closing thread. The leased
    // HandleRegistry closes the in-flight free race — a racing native call sees the
    // entry gone and throws IllegalStateException, never UB.
    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    DemuxReceiver(long h) { this.handle.set(h); }

    /**
     * Bind a receiver to {@code url} with default demux options.
     *
     * @param url {@code rtp://host:port} (unicast or multicast)
     * @return a bound {@code DemuxReceiver}
     * @throws RtpException {@code TRANSPORT} on URL-parse / socket-bind failure
     */
    public static DemuxReceiver fromUrl(String url) throws RtpException {
        long h = nFromUrl(url);
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT,
                "nFromUrl returned 0 without throwing");
        }
        return new DemuxReceiver(h);
    }

    /**
     * Same as {@link #fromUrl(String)} but with an explicit {@link DemuxerConfig}.
     *
     * @param url         {@code rtp://host:port}
     * @param demuxConfig the demuxer configuration
     * @return a bound {@code DemuxReceiver}
     * @throws RtpException as in {@link #fromUrl(String)}
     */
    public static DemuxReceiver fromUrl(String url, DemuxerConfig demuxConfig) throws RtpException {
        long h = nFromUrlWithConfig(
            url,
            demuxConfig.strictMode().ordinal(), demuxConfig.pesCapPerPid(),
            demuxConfig.pesCapTotal(), demuxConfig.cfiTolerance(),
            demuxConfig.av1Carriage().ordinal(), demuxConfig.auCellCapPerPid(),
            demuxConfig.lenientPsiReassembly());
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT,
                "nFromUrlWithConfig returned 0 without throwing");
        }
        return new DemuxReceiver(h);
    }

    /**
     * Iterate the demuxed events. Each {@code hasNext()} blocks on the next
     * {@code recv_event} until an event arrives, the transport drains cleanly
     * (→ end of iteration; rare for connectionless RTP/UDP), or an error occurs.
     * Checked {@link RtpException} / {@link DemuxException} surfaced by the native
     * pull are wrapped in an unchecked {@link RuntimeException} (the
     * {@code Iterator} contract forbids checked exceptions); a byte-sink exception
     * (see {@link #addByteSink}) is re-raised fail-loud and stops iteration.
     *
     * <p>A cross-thread {@link #close()} (the watchdog pattern below) does NOT end
     * iteration via a clean {@code null}/EOF: it cancels the in-flight recv, which
     * surfaces as an {@link RtpException} of kind {@code CANCELLED} wrapped in a
     * {@code RuntimeException}. Catch that to distinguish a deliberate teardown
     * from a real error.
     *
     * <p>Note: RTP/UDP is connectionless — a remote sender closing does NOT end
     * this iteration; it parks on the next datagram. Break out of the loop on a
     * sentinel event, or call {@link #close()} from another thread.
     */
    @Override
    public Iterator<DemuxEvent> iterator() {
        return new Iterator<>() {
            private DemuxEvent peeked;
            private boolean done;
            @Override public boolean hasNext() {
                if (done) return false;
                if (peeked != null) return true;
                try {
                    peeked = nNext(handle.get());
                } catch (RtpException | DemuxException e) {
                    throw new RuntimeException(e);
                }
                if (peeked == null) { done = true; return false; }
                return true;
            }
            @Override public DemuxEvent next() {
                if (!hasNext()) throw new NoSuchElementException();
                DemuxEvent e = peeked; peeked = null; return e;
            }
        };
    }

    /**
     * Register a fan-out callback that receives every 188-byte TS packet — as a
     * fresh {@code byte[]} — BEFORE the demuxer parses it. Sinks fire in
     * registration order; registration is append-only for the receiver's lifetime.
     *
     * <p><b>Fail-loud:</b> if {@code callback} throws, the exception is captured
     * (first error wins; later per-packet errors are dropped) and re-raised from
     * the <em>next</em> iteration step, which then stops iteration.
     *
     * <p><b>Registration timing:</b> register sinks BEFORE iterating, or between
     * {@code next()} calls.
     *
     * <p><b>Re-entrancy:</b> the callback runs on the receiver's own thread inside
     * the recv loop. It MUST NOT re-enter this receiver (no
     * {@code next()}/{@code close()}/{@code stats()} from inside the sink). Keep
     * the callback cheap — a slow sink throttles the receiver.
     *
     * @param callback invoked with a fresh {@code byte[]} per 188-byte TS packet
     * @throws IllegalStateException if the receiver is closed
     */
    public void addByteSink(Consumer<byte[]> callback) {
        ensureOpen();
        nAddByteSink(handle.get(), callback);
    }

    /**
     * Combined {@code (SocketStats, MuxerStats)} snapshot. The socket stats carry
     * the pipeline-tracked recv byte/packet counters; the muxer stats reshape the
     * demux-side counters so callers read the same {@link TransportStats} shape on
     * both {@code MuxSender} and {@code DemuxReceiver}.
     *
     * @return the combined stats snapshot
     * @throws IllegalStateException if the receiver is closed
     */
    public TransportStats stats() {
        ensureOpen();
        return nStats(handle.get());
    }

    /**
     * Close the receiver. Cancels any in-flight {@code next()} first (waking a
     * parked iteration), then frees the underlying RTP socket. May be called from
     * another thread to stop an iteration that is currently <em>parked</em> in
     * {@code next()} (the cancel wakes the parked recv, which releases the inner
     * receiver before it is freed). Do NOT race {@code close()} against any other
     * concurrent call on this receiver (a second {@code next()}, {@code stats()},
     * {@code addByteSink}, etc.) — the single-iterator contract still applies.
     * Idempotent.
     */
    @Override
    public void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    /** Whether the receiver owns a live transport. */
    public boolean isAlive() {
        if (handle.get() == 0) return false;
        return nIsAlive(handle.get());
    }

    private void ensureOpen() {
        if (handle.get() == 0) throw new IllegalStateException("DemuxReceiver is closed");
    }

    // --- Natives ---

    private static native long nFromUrl(String url) throws RtpException;
    private static native long nFromUrlWithConfig(String url, int strict, long pesCapPerPid,
        long pesCapTotal, boolean cfi, int av1, long auCellCap, boolean lenientPsi)
        throws RtpException;
    private static native DemuxEvent nNext(long handle) throws RtpException, DemuxException;
    private static native void nAddByteSink(long handle, Consumer<byte[]> callback);
    private static native TransportStats nStats(long handle);
    private static native void nClose(long handle);
    private static native boolean nIsAlive(long handle);
}
