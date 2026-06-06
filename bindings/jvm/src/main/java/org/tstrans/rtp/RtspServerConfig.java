package org.tstrans.rtp;

import java.util.Optional;

/**
 * Configuration for {@link RtspServer#start(RtspServerConfig)}. Mirrors tst-py
 * {@code tstrans.rtp.RtspServerConfig} (a frozen dataclass). Build via
 * {@link #of(String)} (defaults + bind addr) or {@link #builder()}.
 *
 * <p>{@code tlsCertPem}/{@code tlsKeyPem} are forward-compat: setting either raises
 * {@link org.tstrans.RtspException} of kind {@code TLS} at {@link RtspServer#start}
 * (no rustls in this build). They must be set together (both or neither).
 */
public final class RtspServerConfig {
    private final String bindAddr;
    private final Object auth;            // BasicAuth | DigestAuth | null
    private final int maxSessions;
    private final long sessionTimeoutSecs;
    private final int fanoutCapacity;
    private final long gracefulShutdownDrainMs;
    private final byte[] tlsCertPem;      // nullable; defensively copied
    private final byte[] tlsKeyPem;       // nullable; defensively copied

    private RtspServerConfig(Builder b) {
        this.bindAddr = b.bindAddr;
        this.auth = b.auth;
        this.maxSessions = b.maxSessions;
        this.sessionTimeoutSecs = b.sessionTimeoutSecs;
        this.fanoutCapacity = b.fanoutCapacity;
        this.gracefulShutdownDrainMs = b.gracefulShutdownDrainMs;
        this.tlsCertPem = b.tlsCertPem == null ? null : b.tlsCertPem.clone();
        this.tlsKeyPem = b.tlsKeyPem == null ? null : b.tlsKeyPem.clone();
    }

    /** Config with the default field set, bound to {@code bindAddr}. */
    public static RtspServerConfig of(String bindAddr) {
        return builder().bindAddr(bindAddr).build();
    }

    public static Builder builder() { return new Builder(); }

    public String bindAddr() { return bindAddr; }

    /** @return the auth credential ({@link BasicAuth} or {@link DigestAuth}), if set. */
    public Optional<Object> auth() { return Optional.ofNullable(auth); }

    public int maxSessions() { return maxSessions; }
    public long sessionTimeoutSecs() { return sessionTimeoutSecs; }
    public int fanoutCapacity() { return fanoutCapacity; }
    public long gracefulShutdownDrainMs() { return gracefulShutdownDrainMs; }

    /** @return a defensive copy of the cert PEM, if set. */
    public Optional<byte[]> tlsCertPem() {
        return Optional.ofNullable(tlsCertPem).map(byte[]::clone);
    }

    /** @return a defensive copy of the key PEM, if set. */
    public Optional<byte[]> tlsKeyPem() {
        return Optional.ofNullable(tlsKeyPem).map(byte[]::clone);
    }

    @Override public String toString() {
        return "RtspServerConfig(bindAddr=" + bindAddr
            + ", auth=" + (auth != null ? "<auth>" : "None")
            + ", maxSessions=" + maxSessions
            + ", sessionTimeoutSecs=" + sessionTimeoutSecs
            + ", fanoutCapacity=" + fanoutCapacity
            + ", gracefulShutdownDrainMs=" + gracefulShutdownDrainMs
            + ", tlsCertPem=" + (tlsCertPem != null ? "<bytes>" : "None")
            + ", tlsKeyPem=" + (tlsKeyPem != null ? "<bytes>" : "None") + ")";
    }

    /** Builder for {@link RtspServerConfig}. Defaults match tst-py. */
    public static final class Builder {
        private String bindAddr = "0.0.0.0:8554";
        private Object auth;
        private int maxSessions = 100;
        private long sessionTimeoutSecs = 60;
        private int fanoutCapacity = 256;
        private long gracefulShutdownDrainMs = 2000;
        private byte[] tlsCertPem;
        private byte[] tlsKeyPem;

        private Builder() {}

        public Builder bindAddr(String v) {
            this.bindAddr = java.util.Objects.requireNonNull(v, "bindAddr");
            return this;
        }

        /** @param auth a {@link BasicAuth}, {@link DigestAuth}, or {@code null}. */
        public Builder auth(Object auth) {
            if (auth != null && !(auth instanceof BasicAuth) && !(auth instanceof DigestAuth)) {
                throw new IllegalArgumentException("auth must be BasicAuth, DigestAuth, or null");
            }
            this.auth = auth;
            return this;
        }

        public Builder maxSessions(int v) { this.maxSessions = v; return this; }
        public Builder sessionTimeoutSecs(long v) { this.sessionTimeoutSecs = v; return this; }
        public Builder fanoutCapacity(int v) { this.fanoutCapacity = v; return this; }
        public Builder gracefulShutdownDrainMs(long v) { this.gracefulShutdownDrainMs = v; return this; }

        public Builder tlsCertPem(byte[] pem) {
            this.tlsCertPem = pem == null ? null : pem.clone(); return this;
        }
        public Builder tlsKeyPem(byte[] pem) {
            this.tlsKeyPem = pem == null ? null : pem.clone(); return this;
        }

        public RtspServerConfig build() {
            if (maxSessions <= 0)
                throw new IllegalArgumentException("maxSessions must be > 0; got " + maxSessions);
            if (sessionTimeoutSecs <= 0)
                throw new IllegalArgumentException(
                    "sessionTimeoutSecs must be > 0; got " + sessionTimeoutSecs);
            if (fanoutCapacity <= 0)
                throw new IllegalArgumentException(
                    "fanoutCapacity must be > 0; got " + fanoutCapacity);
            if (gracefulShutdownDrainMs < 0)
                throw new IllegalArgumentException(
                    "gracefulShutdownDrainMs must be >= 0; got " + gracefulShutdownDrainMs);
            boolean certSet = tlsCertPem != null;
            boolean keySet = tlsKeyPem != null;
            if (certSet != keySet)
                throw new IllegalArgumentException(
                    "tlsCertPem and tlsKeyPem must be set together (both or neither)");
            return new RtspServerConfig(this);
        }
    }
}
