package org.tstrans.srt;

import java.util.Iterator;
import java.util.NoSuchElementException;
import org.tstrans.DemuxException;
import org.tstrans.NativeLoader;
import org.tstrans.SrtException;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.DemuxerConfig;

/**
 * Single-call convenience wrapper that owns a managed-reconnect demuxer over an
 * SRT transport. Construct with an SRT URL — {@code mode=listener} (default) OR
 * {@code mode=caller} — and iterate over the emitted {@link DemuxEvent} instances
 * (the same sealed hierarchy {@code org.tstrans.mpegts.Demuxer} produces). On any
 * Broken/Closed event the wrapper re-binds (listener) or re-dials (caller) under
 * the configured {@link ReconnectPolicy} and resumes delivering events.
 *
 * <p>Mirrors {@code tstrans.srt.ManagedDemuxReceiver}. Wraps
 * {@code tst_pipeline::ManagedDemuxReceiver<SrtTransport>}.
 *
 * <p><b>Reconnect discontinuity:</b> after each transport reconnect the inner
 * receiver emits exactly one {@link DemuxEvent.ReconnectDiscontinuity} before any
 * post-reconnect events. Consumers should drop per-stream caches on receipt and
 * rebuild from the next {@link DemuxEvent.ProgramMap}.
 *
 * <p><b>Mode:</b> unlike the plain {@code DemuxReceiver} (listener only), this
 * wrapper accepts {@code mode=caller} too — in caller mode it dials the peer on
 * each (re)connect; in listener mode it re-binds and re-accepts.
 *
 * <pre>{@code
 * try (ManagedDemuxReceiver rx = ManagedDemuxReceiver.fromUrl("srt://:7000?mode=listener")) {
 *     for (DemuxEvent e : rx) {
 *         if (e instanceof DemuxEvent.ReconnectDiscontinuity) {
 *             // drop caches; rebuild on the next ProgramMap
 *         } else if (e instanceof DemuxEvent.Video v) { ... }
 *     }
 * }
 * }</pre>
 *
 * <p><b>Thread safety:</b> a single {@code ManagedDemuxReceiver} is NOT
 * thread-safe and is deliberately NOT {@code synchronized}. Iterate from one
 * thread; the only sanctioned cross-thread operation is {@link #cancelHandle()}'s
 * {@code cancel()}, which wakes a thread parked in iteration.
 *
 * <p><b>Stats:</b> both {@link #socketStats()} and {@link #srtStats()} return a
 * {@link SocketStats} — {@code srtStats()} returns the SAME value as
 * {@code socketStats()} (a documented drift mirroring tst-py, where
 * {@code ManagedRecvTransport} exposes no separate SRT-rich shape) and does NOT
 * throw. There is NO combined {@code stats()} on this wrapper.
 * {@link #reconnectAttempts()} counts every reconnect-factory invocation since
 * construction (an ATTEMPT counter).
 *
 * <p><b>Closing:</b> use try-with-resources or call {@link #close()} explicitly.
 * After close, further calls throw {@code IllegalStateException}.
 *
 * <p><b>Byte-copy posture (JDK 17):</b> sample payloads deliver heap
 * {@code byte[]} copies; a zero-copy path (FFM {@code MemorySegment}) is JDK-22+
 * only and will be added in a future release.
 */
public final class ManagedDemuxReceiver implements AutoCloseable, Iterable<DemuxEvent> {
    static { NativeLoader.load(); }

    private long handle; // Box<JniManagedDemuxReceiver>; 0 = closed

    /** Package-private constructor from a native handle. */
    ManagedDemuxReceiver(long handle) { this.handle = handle; }

    /**
     * Bind (or connect) a managed receiver on the given SRT URL with the default
     * {@link ReconnectPolicy} and default demux options.
     *
     * @param url {@code srt://[host]:port?mode=listener|caller[&key=value&...]}
     * @return a connected {@code ManagedDemuxReceiver}
     * @throws SrtException {@code CONFIG_INVALID} if the URL is malformed;
     *     {@code CONNECT_FAILED} on initial bind/connect failure
     */
    public static ManagedDemuxReceiver fromUrl(String url) throws SrtException {
        return fromUrl(url, (ReconnectPolicy) null);
    }

    /**
     * Bind (or connect) a managed receiver on the given SRT URL with default
     * demux options.
     *
     * @param url    {@code srt://[host]:port?mode=listener|caller[&...]}
     * @param policy reconnect tuning; {@code null} applies the defaults
     * @return a connected {@code ManagedDemuxReceiver}
     * @throws SrtException {@code CONFIG_INVALID} if the URL is malformed;
     *     {@code CONNECT_FAILED} on initial bind/connect failure
     */
    public static ManagedDemuxReceiver fromUrl(String url, ReconnectPolicy policy)
            throws SrtException {
        PolicyArgs p = PolicyArgs.from(policy);
        long h = nFromUrl(
            url,
            p.maxAttemptsPresent(), p.maxAttempts(),
            p.backoffKind(), p.backoffBaseMs(), p.backoffMaxMs(),
            p.gapBufferCapacity(), p.overflowPolicy());
        if (h == 0) {
            throw new SrtException(SrtException.Kind.IO, "nFromUrl returned 0 without throwing");
        }
        return new ManagedDemuxReceiver(h);
    }

    /**
     * Bind (or connect) a managed receiver on the given SRT URL with an explicit
     * {@link DemuxerConfig} and the default {@link ReconnectPolicy}.
     *
     * @param url         {@code srt://[host]:port?mode=listener|caller[&...]}
     * @param demuxConfig the demuxer configuration
     * @return a connected {@code ManagedDemuxReceiver}
     * @throws SrtException as in {@link #fromUrl(String)}
     */
    public static ManagedDemuxReceiver fromUrl(String url, DemuxerConfig demuxConfig)
            throws SrtException {
        return fromUrl(url, demuxConfig, null);
    }

    /**
     * Bind (or connect) a managed receiver on the given SRT URL with an explicit
     * {@link DemuxerConfig} and reconnect policy.
     *
     * @param url         {@code srt://[host]:port?mode=listener|caller[&...]}
     * @param demuxConfig the demuxer configuration
     * @param policy      reconnect tuning; {@code null} applies the defaults
     * @return a connected {@code ManagedDemuxReceiver}
     * @throws SrtException as in {@link #fromUrl(String)}
     */
    public static ManagedDemuxReceiver fromUrl(String url, DemuxerConfig demuxConfig, ReconnectPolicy policy)
            throws SrtException {
        PolicyArgs p = PolicyArgs.from(policy);
        long h = nFromUrlWithConfig(
            url,
            p.maxAttemptsPresent(), p.maxAttempts(),
            p.backoffKind(), p.backoffBaseMs(), p.backoffMaxMs(),
            p.gapBufferCapacity(), p.overflowPolicy(),
            demuxConfig.strictMode().ordinal(), demuxConfig.pesCapPerPid(),
            demuxConfig.pesCapTotal(), demuxConfig.cfiTolerance(),
            demuxConfig.av1Carriage().ordinal(), demuxConfig.auCellCapPerPid(),
            demuxConfig.lenientPsiReassembly());
        if (h == 0) {
            throw new SrtException(SrtException.Kind.IO,
                "nFromUrlWithConfig returned 0 without throwing");
        }
        return new ManagedDemuxReceiver(h);
    }

    /**
     * Iterate the demuxed events. Each {@code hasNext()} blocks on the next
     * {@code recv_event} until an event arrives, the transport closes cleanly
     * (clean EOF → iteration ends), or an error occurs. Checked
     * {@link SrtException} / {@link DemuxException} surfaced by the native pull
     * are wrapped in an unchecked {@link RuntimeException} (the {@code Iterator}
     * contract forbids checked exceptions). The stream emits exactly one
     * {@link DemuxEvent.ReconnectDiscontinuity} after each transport reconnect.
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
                    peeked = nNext(handle);
                } catch (SrtException | DemuxException e) {
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
     * Return a shareable cancel handle. Calling {@link CancelHandle#cancel()}
     * wakes a thread parked in iteration; that pull then ends or throws.
     *
     * @return a new {@link CancelHandle}
     * @throws IllegalStateException if the receiver is closed
     */
    public CancelHandle cancelHandle() {
        ensureOpen();
        long ch = nCancelHandle(handle);
        return new CancelHandle(ch);
    }

    /**
     * Scheme-neutral 16-field wire stats snapshot from the current inner
     * transport (zeros while mid-reconnect).
     *
     * @return the wire-stats snapshot (never null in normal operation)
     * @throws IllegalStateException if the receiver is closed
     */
    public SocketStats socketStats() {
        ensureOpen();
        return nSocketStats(handle);
    }

    /**
     * SRT-specific stats — <b>returns the same {@link SocketStats} view as
     * {@link #socketStats()}</b> (documented drift; {@code ManagedRecvTransport}
     * exposes no separate SRT-rich shape). Unlike {@code ManagedSender}/
     * {@code ManagedReceiver} this does NOT throw, and the return type is
     * {@code SocketStats}, not {@code SrtStats}.
     *
     * @return the wire-stats snapshot (same as {@link #socketStats()})
     * @throws IllegalStateException if the receiver is closed
     */
    public SocketStats srtStats() {
        ensureOpen();
        return nSrtStats(handle);
    }

    /**
     * Total number of times the reconnect factory has been invoked since
     * construction. {@code 0} means the initial connect is still live; rising
     * values mean the inner SRT transport has been rebuilt (or a rebuild attempt
     * failed and was retried).
     *
     * @return the reconnect-attempt counter
     * @throws IllegalStateException if the receiver is closed
     */
    public long reconnectAttempts() {
        ensureOpen();
        return nReconnectAttempts(handle);
    }

    /**
     * Close the receiver. Closes the underlying libsrt socket and stops further
     * reconnects. Idempotent — subsequent calls are no-ops.
     */
    @Override
    public void close() {
        if (handle != 0) {
            nClose(handle);
            handle = 0;
        }
    }

    /**
     * Return {@code true} while the receiver owns a live transport.
     *
     * @return liveness state of the underlying SRT socket
     */
    public boolean isAlive() {
        if (handle == 0) return false;
        return nIsAlive(handle);
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("ManagedDemuxReceiver is closed");
    }

    // --- Natives ---

    private static native long nFromUrl(String url,
        boolean maxAttemptsPresent, int maxAttempts,
        int backoffKind, long backoffBaseMs, long backoffMaxMs,
        int gapBufferCapacity, int overflowPolicy) throws SrtException;
    private static native long nFromUrlWithConfig(String url,
        boolean maxAttemptsPresent, int maxAttempts,
        int backoffKind, long backoffBaseMs, long backoffMaxMs,
        int gapBufferCapacity, int overflowPolicy,
        int strict, long pesCapPerPid, long pesCapTotal, boolean cfi,
        int av1, long auCellCap, boolean lenientPsi) throws SrtException;
    private static native DemuxEvent nNext(long handle) throws SrtException, DemuxException;
    private static native long nCancelHandle(long handle);
    private static native SocketStats nSocketStats(long handle);
    private static native SocketStats nSrtStats(long handle);
    private static native long nReconnectAttempts(long handle);
    private static native void nClose(long handle);
    private static native boolean nIsAlive(long handle);
}
