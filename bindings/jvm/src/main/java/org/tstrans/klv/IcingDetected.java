package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.34 Tag 34 Icing Detected — flag for icing detected at
 * the aircraft location, sensed by a vibrating-probe ice detector.
 *
 * <p>Codepoints are 0-2. A wire-unknown codepoint surfaces as a {@code null}
 * typed accessor ({@link UasDatalinkLs#icingDetected()} returns {@code null})
 * while the raw code is preserved in {@link UasDatalinkLs#icingDetectedCode()}
 * — mirrors the {@link SecurityClassification} precedent. Java enum ordinals
 * are NOT the wire codepoints — use {@link #code()} for serialisation.
 */
public enum IcingDetected {
    DETECTOR_OFF(0),
    NO_ICING_DETECTED(1),
    ICING_DETECTED(2);

    private final int code;

    IcingDetected(int code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.34 wire codepoint for this state. */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code IcingDetected} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-2 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static IcingDetected fromCode(int c) {
        for (IcingDetected v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
