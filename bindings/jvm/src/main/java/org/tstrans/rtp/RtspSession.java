package org.tstrans.rtp;

import org.tstrans.NativeHandle;
import org.tstrans.NativeLoader;
import org.tstrans.RtspException;
import org.tstrans.mpegts.DemuxerConfig;

/**
 * A live RTSP session (server in PLAY state). Drives the remaining control-plane
 * ({@link #pause()} / {@link #play()} / {@link #teardown()}) and consumes the RTP
 * data plane via {@link #intoDemuxReceiver()}. Mirrors tst-py
 * {@code tstrans.rtp.RtspSession}.
 *
 * <p><b>Thread safety:</b> the control methods are single-threaded — call them
 * from one thread. The one sanctioned cross-thread operation is {@link
 * RtspCancelHandle#cancel() cancel} on a {@link RtspCancelHandle} obtained from
 * {@link #cancelHandle()} BEFORE issuing a (potentially blocking) control call;
 * flipping it wakes a parked {@code pause}/{@code play}/{@code teardown} (which
 * then throws {@link RtspException}). {@link #close()} performs a best-effort
 * teardown; the cross-thread interrupt routes through {@link RtspCancelHandle}
 * (exactly like srt {@code Sender}/{@code Receiver}), not through {@code close()}.
 *
 * <p>The native handle is an {@link java.util.concurrent.atomic.AtomicLong}
 * registry key. {@code close()} claims it atomically with {@code getAndSet(0)};
 * the leased {@code HandleRegistry} guarantees no use-after-free/double-free for
 * any native call concurrent with {@code close()} — a racing call either runs or
 * throws a clean {@link IllegalStateException}.
 */
public final class RtspSession extends NativeHandle {
    static { NativeLoader.load(); }

    RtspSession(long h) { setHandle(h); }

    /** Send PAUSE. Server stops emitting RTP; the session stays valid for {@link #play()}. */
    public void pause() throws RtspException { ensureOpen("RtspSession is closed"); nPause(peekHandle()); }

    /** Send PLAY (resume after {@link #pause()}). */
    public void play() throws RtspException { ensureOpen("RtspSession is closed"); nPlay(peekHandle()); }

    /**
     * Send TEARDOWN. Closes the server session; subsequent {@code pause}/{@code play}
     * raise {@link RtspException}. Idempotent on the wrapper (a second call is a no-op).
     */
    public void teardown() throws RtspException { ensureOpen("RtspSession is closed"); nTeardown(peekHandle()); }

    /**
     * Obtain a cross-thread cancel handle. Obtain it BEFORE a blocking control call;
     * flipping {@link RtspCancelHandle#cancel()} from another thread breaks that call
     * out of blocking I/O.
     *
     * @throws IllegalStateException if the session is closed or already torn down
     */
    public RtspCancelHandle cancelHandle() {
        ensureOpen("RtspSession is closed");
        long h = nCancelHandle(peekHandle());
        if (h == 0) throw new IllegalStateException("RtspSession is torn down");
        return new RtspCancelHandle(h);
    }

    /**
     * RTCP-derived stats snapshot. Wave C always returns a zeroed snapshot — the
     * counters wire in with the RTP data plane. Mirrors tst-py's wave-A behavior.
     *
     * @throws IllegalStateException if the session is closed
     */
    public RtspStats stats() {
        ensureOpen("RtspSession is closed");
        return new RtspStats(0L, 0L, 0L, 0L, 0L, 0);
    }

    /** Consume the data plane with default demux options. See {@link #intoDemuxReceiver(DemuxerConfig)}. */
    public DemuxReceiver intoDemuxReceiver() throws RtspException {
        ensureOpen("RtspSession is closed");
        long h = nIntoDemuxReceiver(peekHandle(), false, 0, 0L, 0L, false, 0, 0L, false);
        if (h == 0) {
            throw new RtspException(RtspException.Kind.PROTOCOL,
                "nIntoDemuxReceiver returned 0 without throwing");
        }
        return new DemuxReceiver(h);
    }

    /**
     * Consume the session's RTP data plane and return a {@link DemuxReceiver} over
     * the demuxed {@code DemuxEvent} stream. The control methods remain usable
     * afterward (only the internal data-plane transport is consumed). Calling this
     * twice raises {@link RtspException} of kind {@code PROTOCOL}.
     *
     * @param demuxConfig demuxer configuration (must not be null; use
     *     {@link #intoDemuxReceiver()} for defaults)
     */
    public DemuxReceiver intoDemuxReceiver(DemuxerConfig demuxConfig) throws RtspException {
        ensureOpen("RtspSession is closed");
        long h = nIntoDemuxReceiver(peekHandle(), true,
            demuxConfig.strictMode().ordinal(), demuxConfig.pesCapPerPid(),
            demuxConfig.pesCapTotal(), demuxConfig.cfiTolerance(),
            demuxConfig.av1Carriage().ordinal(), demuxConfig.auCellCapPerPid(),
            demuxConfig.lenientPsiReassembly());
        if (h == 0) {
            throw new RtspException(RtspException.Kind.PROTOCOL,
                "nIntoDemuxReceiver returned 0 without throwing");
        }
        return new DemuxReceiver(h);
    }

    /** Whether {@link #teardown()} (or {@link #close()}) has fired. */
    public boolean isTornDown() {
        if (peekHandle() == 0) return true;
        return nIsTornDown(peekHandle());
    }

    /** Best-effort teardown, then free the native session. Idempotent. */
    @Override public void close() { super.close(); }

    @Override protected void nativeClose(long h) { nClose(h); }

    private static native void nPause(long handle) throws RtspException;
    private static native void nPlay(long handle) throws RtspException;
    private static native void nTeardown(long handle) throws RtspException;
    private static native long nCancelHandle(long handle);
    private static native long nIntoDemuxReceiver(long handle, boolean withConfig,
        int strict, long pesCapPerPid, long pesCapTotal, boolean cfi, int av1,
        long auCellCap, boolean lenientPsi) throws RtspException;
    private static native boolean nIsTornDown(long handle);
    private static native void nClose(long handle);
}
