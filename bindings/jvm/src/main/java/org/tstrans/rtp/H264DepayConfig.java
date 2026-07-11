package org.tstrans.rtp;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Objects;

/**
 * Depacketizer configuration for {@link H264Receiver}. Immutable — construct
 * once via {@link #builder()} and pass to {@code H264Receiver.listen} or
 * {@code RtspClient.connectH264}. Mirrors tst-py {@code tstrans.rtp.H264DepayConfig}.
 *
 * <p>Defaults match {@code tst_rtp::H264DepayConfig::default()}:
 * <ul>
 *   <li>{@code payloadType = 96}
 *   <li>{@code parameterSetInjection = }{@link ParameterSetInjection#BEFORE_IDR}
 *   <li>{@code initialParameterSets = []} (empty list)
 *   <li>{@code maxAuBytes = 8388608} (8 MiB — matches the Rust default)
 * </ul>
 */
public final class H264DepayConfig {
    // Default values mirror tst_rtp::H264DepayConfig::default() exactly.
    // Keep in sync with crates/tst-rtp/src/h264/depacketizer.rs.
    static final int DEFAULT_PAYLOAD_TYPE = 96;
    static final ParameterSetInjection DEFAULT_PARAMETER_SET_INJECTION = ParameterSetInjection.BEFORE_IDR;
    static final long DEFAULT_MAX_AU_BYTES = 8 * 1024 * 1024L; // 8 MiB

    private final int payloadType;
    private final ParameterSetInjection parameterSetInjection;
    private final List<byte[]> initialParameterSets;
    private final long maxAuBytes;

    private H264DepayConfig(Builder b) {
        this.payloadType = b.payloadType;
        this.parameterSetInjection = b.parameterSetInjection;
        // Defensive copy of each entry so callers cannot mutate the list.
        List<byte[]> copy = new ArrayList<>(b.initialParameterSets.size());
        for (byte[] ps : b.initialParameterSets) copy.add(ps.clone());
        this.initialParameterSets = Collections.unmodifiableList(copy);
        this.maxAuBytes = b.maxAuBytes;
    }

    /** All-defaults config. */
    public static H264DepayConfig defaults() { return builder().build(); }

    /** Start building a config. */
    public static Builder builder() { return new Builder(); }

    /** Expected RTP payload type (1..=127; 33 rejected at listen-time). Default 96. */
    public int payloadType() { return payloadType; }

    /**
     * Whether to inject cached SPS/PPS before IDR frames. Default
     * {@link ParameterSetInjection#BEFORE_IDR}.
     */
    public ParameterSetInjection parameterSetInjection() { return parameterSetInjection; }

    /**
     * Out-of-band parameter sets from SDP {@code sprop-parameter-sets}. Each
     * element is one raw NALU (type 7 for SPS, type 8 for PPS). Defensive copies
     * are returned. Default empty list.
     */
    public List<byte[]> initialParameterSets() {
        List<byte[]> result = new ArrayList<>(initialParameterSets.size());
        for (byte[] ps : initialParameterSets) result.add(ps.clone());
        return result;
    }

    /**
     * Maximum combined byte count for a single AU's accumulation buffer.
     * AUs exceeding this limit are discarded. Default 8&nbsp;MiB (matches Rust).
     */
    public long maxAuBytes() { return maxAuBytes; }

    @Override public String toString() {
        return "H264DepayConfig(payloadType=" + payloadType
            + ", parameterSetInjection=" + parameterSetInjection
            + ", initialParameterSets=[" + initialParameterSets.size() + " item(s)]"
            + ", maxAuBytes=" + maxAuBytes + ")";
    }

    /** Builder for {@link H264DepayConfig}. */
    public static final class Builder {
        private int payloadType = DEFAULT_PAYLOAD_TYPE;
        private ParameterSetInjection parameterSetInjection = DEFAULT_PARAMETER_SET_INJECTION;
        private List<byte[]> initialParameterSets = new ArrayList<>();
        private long maxAuBytes = DEFAULT_MAX_AU_BYTES;

        private Builder() {}

        /** Set the expected RTP payload type. Range 1..=127; 33 is rejected at listen-time. */
        public Builder payloadType(int pt) {
            this.payloadType = pt; return this;
        }

        /** Set the parameter-set injection mode. */
        public Builder parameterSetInjection(ParameterSetInjection psi) {
            this.parameterSetInjection = Objects.requireNonNull(psi); return this;
        }

        /**
         * Set the out-of-band parameter sets (raw NALU bytes). Each element is
         * copied defensively. Replaces any previously set list.
         */
        public Builder initialParameterSets(List<byte[]> sets) {
            Objects.requireNonNull(sets);
            this.initialParameterSets = new ArrayList<>(sets);
            return this;
        }

        /** Add a single out-of-band parameter set (raw NALU bytes). Copied defensively. */
        public Builder addInitialParameterSet(byte[] nalu) {
            this.initialParameterSets.add(Objects.requireNonNull(nalu).clone());
            return this;
        }

        /** Set the maximum AU byte count. Must be positive. */
        public Builder maxAuBytes(long maxAuBytes) {
            if (maxAuBytes <= 0) throw new IllegalArgumentException("maxAuBytes must be positive");
            this.maxAuBytes = maxAuBytes; return this;
        }

        public H264DepayConfig build() { return new H264DepayConfig(this); }
    }
}
