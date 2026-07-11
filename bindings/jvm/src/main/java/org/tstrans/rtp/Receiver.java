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

    Receiver(long h) { setHandle(h); }

    /** Bind a receiver to {@code url} with the default {@code pktSize}. */
    public static Receiver fromUrl(String url) throws RtpException {
        return fromUrl(url, Sender.DEFAULT_PKT_SIZE);
    }

    /**
     * Bind a receiver to {@code url}.
     *
     * @param url     {@code rtp://host:port} (unicast or multicast)
     * @param pktSize expected send-side packet size; has no effect on the receive
     *                buffer, which always accepts any legal datagram; must be
     *                &ge; 0
     * @throws RtpException {@code TRANSPORT} on URL-parse / bind failure
     * @throws IllegalArgumentException if {@code pktSize} is negative
     */
    public static Receiver fromUrl(String url, int pktSize) throws RtpException {
        if (pktSize < 0) throw new IllegalArgumentException("pktSize must be >= 0: " + pktSize);
        long h = nFromUrl(url, pktSize);
        if (h == 0) {
            throw new RtpException(RtpException.Kind.TRANSPORT, "nFromUrl returned 0 without throwing");
        }
        return new Receiver(h);
    }

    /**
     * Receive one TS payload chunk. Blocks until a packet arrives or a cancel fires.
     *
     * @throws IllegalStateException if the receiver is closed
     * @throws RtpException {@code CANCELLED} if a cancel fired; {@code TRANSPORT} otherwise
     */
    public byte[] recv() throws RtpException {
        ensureOpen("Receiver is closed");
        return nRecv(peekHandle());
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

    /** Close the receiver. Idempotent. */
    @Override public void close() { super.close(); }

    @Override protected void nativeClose(long h) { nClose(h); }

    private static native long   nFromUrl(String url, int pktSize) throws RtpException;
    private static native byte[] nRecv(long handle) throws RtpException;
    private static native SocketStats nSocketStats(long handle);
    private static native long   nCancelHandle(long handle);
    private static native void   nClose(long handle);
}
