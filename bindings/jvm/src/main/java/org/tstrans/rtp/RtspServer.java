package org.tstrans.rtp;

import org.tstrans.MuxException;
import org.tstrans.NativeLoader;
import org.tstrans.RtspException;
import org.tstrans.mpegts.MuxerConfig;

/**
 * Sync RTSP server. Construct via {@link #start(RtspServerConfig)}. Mirrors tst-py
 * {@code tstrans.rtp.RtspServer}. The underlying {@code tst_rtp::RtspServer} owns a
 * tokio Runtime for its lifetime, held inside the native box — there is no
 * JNI-side async handling.
 *
 * <p><b>Closing:</b> {@link #close()} performs a graceful stop (RFC 7826 §13.5.1
 * Notice 5402 server-initiated teardown of active sessions) then frees the native
 * server. Use try-with-resources. {@link #stop(long)} is the explicit graceful
 * shutdown; {@link #cancelHandle()} returns a cross-thread hard-cancel.
 */
public final class RtspServer implements AutoCloseable {
    static { NativeLoader.load(); }

    private long handle; // Box<tst_rtp::RtspServer>; 0 = closed

    RtspServer(long handle) { this.handle = handle; }

    /**
     * Build, bind, and start a server from {@code config}.
     *
     * @throws RtspException {@code SERVER} on bind/start failure, {@code PROTOCOL}
     *     on bind-URL parse failure, {@code TLS} if a TLS PEM field is set (TLS is
     *     forward-compat — not wired in this build)
     * @throws IllegalArgumentException if {@code config.auth()} is set without a realm
     */
    public static RtspServer start(RtspServerConfig config) throws RtspException {
        int authScheme = -1;
        String realm = null, user = null, password = null;
        Object auth = config.auth().orElse(null);
        if (auth instanceof BasicAuth b) {
            authScheme = 0;
            realm = b.realm().orElseThrow(() -> new IllegalArgumentException(
                "server-side BasicAuth requires a realm"));
            user = b.user();
            password = b.password();
        } else if (auth instanceof DigestAuth d) {
            authScheme = (d.algorithm() == DigestAlgorithm.SHA256) ? 2 : 1;
            realm = d.realm().orElseThrow(() -> new IllegalArgumentException(
                "server-side DigestAuth requires a realm"));
            user = d.user();
            password = d.password();
        }
        long h = nStart(
            config.bindAddr(),
            config.maxSessions(), config.sessionTimeoutSecs(),
            config.fanoutCapacity(), config.gracefulShutdownDrainMs(),
            authScheme, realm, user, password,
            config.tlsCertPem().isPresent(), config.tlsKeyPem().isPresent());
        if (h == 0) {
            throw new RtspException(RtspException.Kind.SERVER,
                "nStart returned 0 without throwing");
        }
        return new RtspServer(h);
    }

    /** Aggregate server stats snapshot. @throws IllegalStateException if closed. */
    public ServerStats stats() { ensureOpen(); return nStats(handle); }

    /** Bound listener address as {@code "ip:port"}, or {@code null} before bind. */
    public String localAddr() { ensureOpen(); return nLocalAddr(handle); }

    /**
     * Graceful shutdown — fires the Notice 5402 path on each active session, waits
     * the builder's drain window. Idempotent. {@code drainMs} is accepted for API
     * stability but the configured {@code gracefulShutdownDrainMs} governs the wait.
     *
     * @throws RtspException {@code SERVER} if the server was never started
     */
    public void stop(long drainMs) throws RtspException { ensureOpen(); nStop(handle, drainMs); }

    /** {@link #stop(long)} with the default drain hint (1000). */
    public void stop() throws RtspException { stop(1000L); }

    /**
     * Register a unicast mount under {@code path}. The returned {@link MountHandle}
     * is the push surface; it is shareable across producer threads.
     *
     * @throws RtspException {@code MOUNT} for an invalid/duplicate mount path; {@code SERVER}
     *     if the server is stopped
     * @throws MuxException if the muxer rejects {@code programConfig}
     */
    public MountHandle addUnicastMount(String path, MuxerConfig programConfig)
            throws RtspException, MuxException {
        ensureOpen();
        long h = nAddUnicastMount(handle,
            path,
            programConfig.programNumber(), programConfig.pmtPid(), programConfig.pcrPid(),
            programConfig.pcrIntervalMs(), programConfig.psiIntervalMs(),
            programConfig.bufferPackets(), programConfig.av1Carriage().ordinal(),
            programConfig.streamPids(), programConfig.streamKinds(),
            programConfig.streamCodecs(), programConfig.klvStreamTypes(),
            programConfig.klvCarriesPts());
        if (h == 0) {
            throw new RtspException(RtspException.Kind.MOUNT,
                "nAddUnicastMount returned 0 without throwing");
        }
        return new MountHandle(h);
    }

    /** {@link #addMulticastMount(String, String, int, int, String, MuxerConfig)} with ttl=1, no iface. */
    public MountHandle addMulticastMount(String path, String group, int port,
            MuxerConfig programConfig) throws RtspException, MuxException {
        return addMulticastMount(path, group, port, 1, null, programConfig);
    }

    /**
     * Register a multicast mount. {@code group} is a literal multicast IP; {@code ttl}
     * defaults to 1 (link-local); {@code iface} pins the NIC (IPv4 literal / IPv6 iface
     * name), or {@code null}.
     *
     * @throws RtspException {@code MOUNT} for an invalid/duplicate mount path or invalid
     *     group/address; {@code SERVER} if the server is stopped
     * @throws MuxException if the muxer rejects {@code programConfig}
     */
    public MountHandle addMulticastMount(String path, String group, int port, int ttl,
            String iface, MuxerConfig programConfig) throws RtspException, MuxException {
        ensureOpen();
        long h = nAddMulticastMount(handle,
            path, group, port, ttl, iface,
            programConfig.programNumber(), programConfig.pmtPid(), programConfig.pcrPid(),
            programConfig.pcrIntervalMs(), programConfig.psiIntervalMs(),
            programConfig.bufferPackets(), programConfig.av1Carriage().ordinal(),
            programConfig.streamPids(), programConfig.streamKinds(),
            programConfig.streamCodecs(), programConfig.klvStreamTypes(),
            programConfig.klvCarriesPts());
        if (h == 0) {
            throw new RtspException(RtspException.Kind.MOUNT,
                "nAddMulticastMount returned 0 without throwing");
        }
        return new MountHandle(h);
    }

    /** Cross-thread hard-cancel handle. @throws IllegalStateException if closed. */
    public RtspServerCancelHandle cancelHandle() {
        ensureOpen();
        long h = nCancelHandle(handle);
        if (h == 0) throw new IllegalStateException("RtspServer is closed");
        return new RtspServerCancelHandle(h);
    }

    /** Graceful stop (best-effort) then free the native server. Idempotent. */
    @Override
    public void close() {
        if (handle != 0) {
            nClose(handle);
            handle = 0;
        }
    }

    void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("RtspServer is closed");
    }

    private static native long nStart(String bindAddr, long maxSessions, long sessionTimeoutSecs,
        long fanoutCapacity, long gracefulShutdownDrainMs, int authScheme, String authRealm,
        String authUser, String authPassword, boolean hasTlsCert, boolean hasTlsKey)
        throws RtspException;
    private static native ServerStats nStats(long handle);
    private static native String nLocalAddr(long handle);
    private static native void nStop(long handle, long drainMs) throws RtspException;
    private static native long nCancelHandle(long handle);
    private static native void nClose(long handle);
    private static native long nAddUnicastMount(long serverHandle, String path,
        int programNumber, int pmtPid, int pcrPid, int pcrIntervalMs, int psiIntervalMs,
        int bufferPackets, int av1Carriage, int[] streamPids, int[] streamKinds,
        int[] streamCodecs, int[] klvStreamTypes, boolean[] klvCarriesPts)
        throws RtspException, MuxException;
    private static native long nAddMulticastMount(long serverHandle, String path, String group,
        int port, int ttl, String iface, int programNumber, int pmtPid, int pcrPid,
        int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
        int[] streamPids, int[] streamKinds, int[] streamCodecs, int[] klvStreamTypes,
        boolean[] klvCarriesPts) throws RtspException, MuxException;
}
