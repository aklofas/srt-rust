package org.tstrans.rtp;

import org.tstrans.NativeLoader;
import org.tstrans.RtspException;

/**
 * Static facade for RTSP client connections. {@link #connect(RtspClientConfig)}
 * runs the full OPTIONS / DESCRIBE / SETUP / PLAY exchange against the server and
 * returns a live {@link RtspSession} in PLAY state. Mirrors tst-py
 * {@code tstrans.rtp.RtspClient}.
 *
 * <p><b>TLS is forward-compat only.</b> This binding does not link rustls and does
 * not enable {@code tst-rtp/tls}; connecting to an {@code rtsps://} URL surfaces
 * {@link RtspException} of kind {@code TLS} (the URL scheme drives this — the
 * connect path short-circuits before any I/O). {@code RtspClientConfig.tlsRootCertsPem}
 * is accepted for forward-compat surface parity but is NOT read by {@code connect}
 * (pass-through-only), matching tst-py.
 *
 * <p><b>Pass-through config fields.</b> {@code transportPref} and {@code rtspVersion}
 * are likewise informational/pass-through: the underlying tst-rtp connect derives
 * the transport (from a {@code ?transport=udp|tcp} URL query) and the version (from
 * the {@code rtsp://} vs {@code rtsps://} scheme) from the URL, not from these
 * fields (matching tst-py). They round-trip through the config unchanged.
 */
public final class RtspClient {
    static { NativeLoader.load(); }

    private RtspClient() {}

    /**
     * Connect and drive the control-plane to PLAY.
     *
     * @param config the connection configuration
     * @return a live session in PLAY state
     * @throws RtspException on any control-plane failure (URL parse → {@code PROTOCOL};
     *     {@code rtsps://} on a non-TLS build → {@code TLS}; refused/timeout → {@code IO}/
     *     {@code TIMEOUT}; 401/404 → {@code AUTH_REQUIRED}/{@code NOT_FOUND}; …)
     */
    public static RtspSession connect(RtspClientConfig config) throws RtspException {
        String authUser = null, authPassword = null;
        Object a = config.auth().orElse(null);
        if (a instanceof BasicAuth b) {
            authUser = b.user(); authPassword = b.password();
        } else if (a instanceof DigestAuth d) {
            authUser = d.user(); authPassword = d.password();
        }
        long h = nConnect(config.url(), authUser, authPassword, config.keepalive());
        if (h == 0) {
            throw new RtspException(RtspException.Kind.PROTOCOL,
                "nConnect returned 0 without throwing");
        }
        return new RtspSession(h);
    }

    // authUser/authPassword null when no auth. The algorithm is NOT passed: tst-rtp's
    // challenge handler picks it from the server's WWW-Authenticate header (matches
    // tst-py, which captures DigestAuth.algorithm for introspection only).
    private static native long nConnect(String url, String authUser, String authPassword,
        boolean keepalive) throws RtspException;
}
