package org.tstrans.rtp;

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
public final class Receiver implements AutoCloseable {
    static { NativeLoader.load(); }

    private long handle; // Box<JniRtpReceiver>; 0 = closed

    Receiver(long handle) { this.handle = handle; }

    /** Bind a receiver to {@code url} with the default recv scratch size. */
    public static Receiver fromUrl(String url) throws RtpException {
        return fromUrl(url, Sender.DEFAULT_PKT_SIZE);
    }

    /**
     * Bind a receiver to {@code url}.
     *
     * @param url     {@code rtp://host:port} (unicast or multicast)
     * @param pktSize recv scratch buffer size; must be &ge; 0
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
        ensureOpen();
        return nRecv(handle);
    }

    /** Snapshot of wire-level statistics (never null in normal operation). */
    public SocketStats socketStats() {
        ensureOpen();
        return nSocketStats(handle);
    }

    /**
     * Return a shareable cancel handle. Calling {@link CancelHandle#cancel()}
     * wakes a thread parked in {@link #recv}; that call throws
     * {@code RtpException(CANCELLED)}.
     */
    public CancelHandle cancelHandle() {
        ensureOpen();
        return new CancelHandle(nCancelHandle(handle));
    }

    /** Close the receiver. Idempotent. */
    @Override
    public void close() {
        if (handle != 0) {
            nClose(handle);
            handle = 0;
        }
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("Receiver is closed");
    }

    private static native long   nFromUrl(String url, int pktSize) throws RtpException;
    private static native byte[] nRecv(long handle) throws RtpException;
    private static native SocketStats nSocketStats(long handle);
    private static native long   nCancelHandle(long handle);
    private static native void   nClose(long handle);
}
