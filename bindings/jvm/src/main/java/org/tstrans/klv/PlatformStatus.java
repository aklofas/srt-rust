package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.125 Tag 125 Platform Status — operational status of
 * the platform.
 *
 * <p>Codepoints are 0-12. A wire-unknown codepoint surfaces as a
 * {@code null} typed accessor ({@link UasDatalinkLs#platformStatus()}
 * returns {@code null}) while the raw code is preserved in
 * {@link UasDatalinkLs#platformStatusCode()} — mirrors the
 * {@link IcingDetected} precedent. Java enum ordinals are NOT the wire
 * codepoints — use {@link #code()} for serialisation.
 */
public enum PlatformStatus {
    ACTIVE(0),
    PRE_FLIGHT(1),
    PRE_FLIGHT_TAXIING(2),
    RUN_UP(3),
    TAKE_OFF(4),
    INGRESS(5),
    MANUAL_OPERATION(6),
    AUTOMATED_ORBIT(7),
    TRANSITIONING(8),
    EGRESS(9),
    LANDING(10),
    LANDED_TAXIING(11),
    LANDED_PARKED(12);

    private final int code;

    PlatformStatus(int code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.125 wire codepoint for this state. */
    public int code() {
        return code;
    }

    /**
     * Look up a {@code PlatformStatus} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-12 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static PlatformStatus fromCode(int c) {
        for (PlatformStatus v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
