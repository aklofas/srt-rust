package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.63 Tag 63 Sensor Field of View Name — indicates the
 * Motion Imagery sensor's current lens type / FOV preset.
 *
 * <p><b>Spec discrepancy:</b> the item's own definition table (§8.63) caps
 * the KLV range at {@code [0, 7]}, but the Details subsection's worked
 * table — ST 0601.19 §8.63.1 Table 4 — lists a 9th codepoint, {@code 8} =
 * "Continuous Zoom". Modeled per Table 4 (matching
 * {@code tst_core::klv::st0601::SensorFovName}) since it is the more
 * complete of the two spec tables and real-world encoders emit it.
 *
 * <p>Codepoints are 0-8. A wire-unknown codepoint surfaces as a
 * {@code null} typed accessor ({@link UasDatalinkLs#sensorFovName()}
 * returns {@code null}) while the raw code is preserved in
 * {@link UasDatalinkLs#sensorFovNameCode()} — mirrors the
 * {@link SecurityClassification} precedent. Java enum ordinals are NOT the
 * wire codepoints — use {@link #code()} for serialisation.
 */
public enum SensorFovName {
    ULTRANARROW(0),
    NARROW(1),
    MEDIUM(2),
    WIDE(3),
    ULTRAWIDE(4),
    NARROW_MEDIUM(5),
    TWO_X_ULTRANARROW(6),
    FOUR_X_ULTRANARROW(7),
    CONTINUOUS_ZOOM(8);

    private final int code;

    SensorFovName(int code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.63 wire codepoint for this preset. */
    public int code() {
        return code;
    }

    /**
     * Look up a {@code SensorFovName} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-8 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static SensorFovName fromCode(int c) {
        for (SensorFovName v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
