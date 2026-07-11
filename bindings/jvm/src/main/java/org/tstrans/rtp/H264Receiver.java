package org.tstrans.rtp;

import org.tstrans.NativeHandle;
import org.tstrans.NativeLoader;
import org.tstrans.RtpException;

/**
 * Blocking H.264-over-RTP receiver. Wraps {@code tst_rtp::H264Receiver}.
 * Mirrors tst-py {@code tstrans.rtp.H264Receiver}.
 *
 * <h3>Constructing</h3>
 *
 * <p>{@link #listen(String)} / {@link #listen(String, H264DepayConfig)} — bind a
 * UDP socket to an {@code rtp://host:port?pt=N} URL and return a ready receiver.
 * The {@code ?pt=} query parameter is required (range 1..=127; value 33 is
 * rejected — use {@link DemuxReceiver} for MPEG-TS streams).
 *
 * <h3>Receiving</h3>
 *
 * <p>{@link #recvAu()} blocks until a complete H.264 Access Unit is reassembled
 * or EOS. Returns {@link H264AccessUnit} on success, {@code null} at EOS (clean
 * close or RTSP teardown), or throws {@link RtpException} on transport failure.
 *
 * <h3>Thread safety</h3>
 *
 * <p>A single {@code H264Receiver} must be iterated from one thread (single-iterator
 * contract). The one sanctioned cross-thread operation is {@link #cancelHandle()}:
 * obtain a {@link CancelHandle} BEFORE the potentially-blocking {@link #recvAu()}
 * call, then call {@link CancelHandle#cancel()} from another thread to wake
 * the parked recv within ~100&nbsp;ms. {@link #close()} also fires the cancel
 * handle before freeing the receiver and is safe to call from another thread to
 * stop an iteration currently parked in {@code recvAu()}.
 *
 * <h3>Stats</h3>
 *
 * <p>Three complementary views, mirroring {@code tst_rtp::H264Receiver}:
 * <ul>
 *   <li>{@link #socketStats()} — wire-level throughput (bytes/packets received).
 *       Note: unlike {@link DemuxReceiver#stats()}, this returns {@link SocketStats}
 *       directly (not wrapped in a {@code TransportStats}). The Rust/Python surface
 *       has the same asymmetry — {@code H264Receiver.socket_stats()} returns a bare
 *       {@code SocketStats}; {@code DemuxReceiver.stats()} returns a combined view.
 *   <li>{@link #rtpStats()} — protocol-level anomaly counter (malformed packets).
 *   <li>{@link #depayStats()} — RFC 6184 depacketizer internals (AU counts, seq
 *       gaps, parameter-set updates).
 * </ul>
 *
 * <h3>Lifecycle</h3>
 *
 * <p>{@link #close()} fires the cancel handle and frees the native receiver.
 * Subsequent {@link #recvAu()} throws {@link IllegalStateException}. Idempotent.
 *
 * <pre>{@code
 * try (H264Receiver rx = H264Receiver.listen("rtp://0.0.0.0:5004?pt=96")) {
 *     H264AccessUnit au;
 *     while ((au = rx.recvAu()) != null) {
 *         byte[] annexb = au.annexb();
 *         // ... decode or re-mux ...
 *     }
 * }
 * }</pre>
 */
public final class H264Receiver extends NativeHandle {
    static { NativeLoader.load(); }

    H264Receiver(long h) { setHandle(h); }

    /**
     * Bind to {@code url} with default {@link H264DepayConfig}.
     *
     * @param url {@code rtp://host:port?pt=N} where {@code N} is the dynamic
     *     payload type (1..=127; 33 is rejected — use {@link DemuxReceiver} for MPEG-TS)
     * @return a bound {@code H264Receiver}
     * @throws RtpException {@code TRANSPORT} on URL parse failure, missing
     *     {@code ?pt=}, or socket bind error
     */
    public static H264Receiver listen(String url) throws RtpException {
        long h = nListen(url);
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT,
                "nListen returned 0 without throwing");
        }
        return new H264Receiver(h);
    }

    /**
     * Bind to {@code url} with an explicit {@link H264DepayConfig}.
     *
     * <p>The {@code ?pt=} value in the URL overrides {@code config.payloadType()}.
     *
     * @param url    {@code rtp://host:port?pt=N}
     * @param config depacketizer configuration
     * @return a bound {@code H264Receiver}
     * @throws RtpException as in {@link #listen(String)}
     */
    public static H264Receiver listen(String url, H264DepayConfig config) throws RtpException {
        // Build initialParameterSets as a flat byte[][] for the native.
        // Each element is one raw NALU (type 7 or 8).
        java.util.List<byte[]> psList = config.initialParameterSets();
        byte[][] psArr = psList.toArray(new byte[0][]);
        long h = nListenWithConfig(
            url,
            config.payloadType(),
            config.parameterSetInjection().ordinal(),
            psArr,
            config.maxAuBytes()
        );
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT,
                "nListenWithConfig returned 0 without throwing");
        }
        return new H264Receiver(h);
    }

    /**
     * Receive the next reassembled H.264 Access Unit.
     *
     * <p>Blocks until a packet arrives or EOS / error. The native call parks the
     * calling thread on kernel I/O; there is no GIL to release on the JVM side.
     *
     * @return the next {@link H264AccessUnit}, or {@code null} at EOS (clean close
     *     or RTSP teardown — caller should exit the recv loop)
     * @throws RtpException {@code CANCELLED} if the cancel handle was fired
     *     explicitly; {@code TRANSPORT} on a hard I/O error
     * @throws IllegalStateException if the receiver is already closed
     */
    public H264AccessUnit recvAu() throws RtpException {
        ensureOpen("H264Receiver is closed");
        return nRecvAu(peekHandle());
    }

    /**
     * RFC 6184 depacketizer counters (AU counts, seq gaps, parameter-set updates).
     *
     * @throws IllegalStateException if the receiver is closed
     */
    public H264DepayStats depayStats() {
        ensureOpen("H264Receiver is closed");
        return nDepayStats(peekHandle());
    }

    /**
     * RTP protocol-level statistics (malformed packet counter).
     *
     * @throws IllegalStateException if the receiver is closed
     */
    public RtpStats rtpStats() {
        ensureOpen("H264Receiver is closed");
        return nRtpStats(peekHandle());
    }

    /**
     * Wire-level throughput statistics (bytes / packets received).
     *
     * <p><b>Asymmetry note:</b> this method returns {@link SocketStats} directly,
     * matching {@code tst_rtp::H264Receiver::socket_stats()} and tst-py's
     * {@code H264Receiver.socket_stats()}. The {@link DemuxReceiver} wrapper
     * returns {@link TransportStats} (a combined view). The difference reflects
     * the Rust API shape, not a binding inconsistency.
     *
     * @throws IllegalStateException if the receiver is closed
     */
    public SocketStats socketStats() {
        ensureOpen("H264Receiver is closed");
        return nSocketStats(peekHandle());
    }

    /**
     * Local address the UDP socket is bound to, as {@code "host:port"}. Returns
     * {@code null} for the TCP-interleaved (RTSP) path where no UDP socket exists.
     *
     * <p>Tests use this to discover the ephemeral port when the URL specifies
     * {@code :0}.
     *
     * @throws IllegalStateException if the receiver is closed
     */
    public String localAddr() {
        ensureOpen("H264Receiver is closed");
        return nLocalAddr(peekHandle());
    }

    /**
     * Obtain a cross-thread cancel handle. Obtain it BEFORE a blocking
     * {@link #recvAu()} call; calling {@link CancelHandle#cancel()} from another
     * thread wakes the parked recv within ~100&nbsp;ms, causing it to return
     * {@code null} (EOS) rather than throwing.
     *
     * @throws IllegalStateException if the receiver is closed
     */
    public CancelHandle cancelHandle() {
        ensureOpen("H264Receiver is closed");
        long h = nCancelHandle(peekHandle());
        if (h == 0) throw new IllegalStateException("H264Receiver: cancelHandle native returned 0");
        return new CancelHandle(h);
    }

    /**
     * Cancel any in-progress {@link #recvAu()} and free the underlying receiver.
     * Idempotent. Safe to call from another thread to stop a parked recv.
     *
     * <p><b>May block briefly:</b> the cancel hook fires first (without taking
     * the native resource lock), then the close waits for the woken
     * {@code recvAu()} to release that lock. The parked recv polls its cancel
     * flag at ~100&nbsp;ms granularity, so a cross-thread {@code close()} can
     * take up to ~100&nbsp;ms (plus one packet-processing interval) to return.
     *
     * <p>For receivers created via {@link RtspSession#intoH264Receiver()}, this
     * also issues a best-effort RTSP TEARDOWN — the receiver owns the session's
     * control plane (the session wrapper was consumed at conversion).
     */
    @Override public void close() { super.close(); }

    @Override protected void nativeClose(long h) { nClose(h); }

    // --- Natives ---

    private static native long nListen(String url) throws RtpException;
    private static native long nListenWithConfig(String url, int payloadType,
        int parameterSetInjection, byte[][] initialParameterSets,
        long maxAuBytes) throws RtpException;
    private static native H264AccessUnit nRecvAu(long handle) throws RtpException;
    private static native H264DepayStats nDepayStats(long handle);
    private static native RtpStats nRtpStats(long handle);
    private static native SocketStats nSocketStats(long handle);
    private static native String nLocalAddr(long handle);
    private static native long nCancelHandle(long handle);
    private static native void nClose(long handle);
}
