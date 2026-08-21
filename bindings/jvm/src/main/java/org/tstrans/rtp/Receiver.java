package org.tstrans.rtp;

import org.tstrans.NativeHandle;
import org.tstrans.NativeLoader;
import org.tstrans.RtpException;

/**
 * RTP receiver — wraps {@code tst_rtp::RtpRecvTransport}. Binds to {@code url}
 * (literal IP:port); for multicast URLs the group is joined automatically.
 * {@link #recv()} returns the TS payload of one datagram (12-byte RTP header
 * already stripped) and blocks until a packet arrives or a cancel fires.
 *
 * <p><b>Thread safety:</b> a single {@code Receiver} is NOT thread-safe; use one
 * per thread. Cross-thread stop = {@link #cancelHandle()}{@code .cancel()}.
 *
 * <p>Mirrors {@code tstrans.rtp.Receiver} in tst-py.
 */
public final class Receiver extends NativeHandle {
    static { NativeLoader.load(); }

    // Populated by nativeClose from nClose's close-time snapshot (see
    // endReason()/endDetail()'s javadoc for why: once closed, peekHandle()
    // is 0 and there is no handle left to pass to a live-getter native).
    private volatile StreamEndReason closedEndReason;
    private volatile String closedEndDetail;

    Receiver(long h) { setHandle(h); }

    /** Bind a receiver to {@code url} ({@code rtp://host:port}, unicast or
     *  multicast). The receive buffer sizes itself to the transport's
     *  deliverable ceiling; {@code ?pkt_size=} on a receiver URL is rejected.
     *
     * @throws RtpException {@code TRANSPORT} on URL-parse / bind failure
     */
    public static Receiver fromUrl(String url) throws RtpException {
        long h = nFromUrl(url);
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT, "nFromUrl returned 0 without throwing");
        }
        return new Receiver(h);
    }

    /**
     * Receive one TS payload chunk. Blocks until a packet arrives or a cancel fires.
     *
     * @throws IllegalStateException if the receiver is closed
     * @throws RtpException {@code CANCELLED} if a cancel fired; {@code TRANSPORT} otherwise;
     *     {@code TIMEOUT} if a configured persistent recv deadline (the
     *     {@code ?recv_timeout=<ms>} URL knob) expires
     * @see #recv(Integer) for a per-call deadline instead of (or on top of) a
     *     persistent one
     */
    public byte[] recv() throws RtpException {
        ensureOpen("Receiver is closed");
        return nRecv(peekHandle());
    }

    /**
     * Receive one TS payload chunk, bounded by a per-call deadline.
     *
     * @param timeoutMs milliseconds to wait for a packet; {@code null} blocks
     *     indefinitely, identically to {@link #recv()} (any persistent
     *     deadline armed by the {@code ?recv_timeout=<ms>} URL knob still
     *     applies in that case). A non-null value overrides the persistent
     *     deadline for this one call.
     * @throws IllegalStateException if the receiver is closed
     * @throws RtpException {@code CANCELLED} if a cancel fired; {@code TRANSPORT}
     *     otherwise; {@code TIMEOUT} if {@code timeoutMs} elapses, or (when
     *     {@code timeoutMs} is {@code null}) a configured persistent recv
     *     deadline expires
     */
    public byte[] recv(Integer timeoutMs) throws RtpException {
        ensureOpen("Receiver is closed");
        return nRecvTimeout(peekHandle(), timeoutMs == null ? -1L : (long) timeoutMs);
    }

    /** Snapshot of wire-level statistics (never null in normal operation). */
    public SocketStats socketStats() {
        ensureOpen("Receiver is closed");
        return nSocketStats(peekHandle());
    }

    /**
     * Return a shareable cancel handle. Calling {@link CancelHandle#cancel()}
     * wakes a thread parked in {@link #recv}; that call throws
     * {@code RtpException(CANCELLED)}.
     */
    public CancelHandle cancelHandle() {
        ensureOpen("Receiver is closed");
        return new CancelHandle(nCancelHandle(peekHandle()));
    }

    /**
     * Why the receive session ended, or {@code null} if it hasn't ended yet
     * (or ended through a path this arc doesn't instrument). Still readable
     * after {@link #close()} — the close path snapshots the reason before
     * the underlying native resource is freed.
     */
    public StreamEndReason endReason() {
        long h = peekHandle();
        if (h == 0) return closedEndReason;
        return StreamEndReason.fromWireOrdinal(nEndReason(h));
    }

    /**
     * Free-text detail for {@link #endReason()} — the message carried by
     * {@code KEEPALIVE_FAILED} / {@code TRANSPORT_FAILED} /
     * {@code PROTOCOL_ERROR}; {@code null} for every other reason (including
     * "hasn't ended yet"). Still readable after {@link #close()}.
     */
    public String endDetail() {
        long h = peekHandle();
        if (h == 0) return closedEndDetail;
        return nEndDetail(h);
    }

    /** Close the receiver. Idempotent. */
    @Override public void close() { super.close(); }

    @Override protected void nativeClose(long h) {
        Object[] snapshot = nClose(h);
        closedEndReason = StreamEndReason.fromWireOrdinal((Integer) snapshot[0]);
        closedEndDetail = (String) snapshot[1];
    }

    private static native long   nFromUrl(String url) throws RtpException;
    private static native byte[] nRecv(long handle) throws RtpException;
    private static native byte[] nRecvTimeout(long handle, long timeoutMs) throws RtpException;
    private static native SocketStats nSocketStats(long handle);
    private static native long   nCancelHandle(long handle);
    private static native int    nEndReason(long handle);
    private static native String nEndDetail(long handle);
    private static native Object[] nClose(long handle);
}
