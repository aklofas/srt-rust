package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.138 Table 17 Payload Type enumeration.
 *
 * <p>A wire-unknown codepoint surfaces as a {@code null} typed accessor
 * ({@link PayloadRecord#payloadType()} returns {@code null}) while the raw
 * code is preserved in {@link PayloadRecord#payloadTypeCode()} — mirrors the
 * {@link IcingDetected} precedent, but widened to {@code long}: unlike
 * {@code IcingDetected}'s narrow wire byte, the Rust
 * {@code PayloadType::Other} catch-all carries the type code's full BER-OID
 * {@code u64} range, so {@link #code()}/{@link #fromCode(long)} use
 * {@code long} rather than {@code int}. Java enum ordinals are NOT the wire
 * codepoints — use {@link #code()} for serialisation.
 */
public enum PayloadType {
    ELECTRO_OPTICAL(0),
    LIDAR(1),
    RADAR(2),
    SIGINT(3),
    SAR(4);

    private final long code;

    PayloadType(long code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.138 Table 17 wire codepoint for this type. */
    public long code() {
        return code;
    }

    /**
     * Look up a {@code PayloadType} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-4 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static PayloadType fromCode(long c) {
        for (PayloadType v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
