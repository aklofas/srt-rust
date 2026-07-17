package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.77 Tag 77 Operational Mode — indicates the mode of
 * operations of the event portrayed in the Motion Imagery, per the §8.77.1
 * Table 5 enumeration.
 *
 * <p>Spec code {@code 0} is named "Other" in Table 5; this binding (like
 * the Rust {@code tst_core::klv::st0601::OperationalMode}) names it
 * {@link #OTHER_MODE} to avoid confusion with an unknown-codepoint result.
 *
 * <p>Codepoints are 0-5. A wire-unknown codepoint surfaces as a
 * {@code null} typed accessor ({@link UasDatalinkLs#operationalMode()}
 * returns {@code null}) while the raw code is preserved in
 * {@link UasDatalinkLs#operationalModeCode()} — mirrors the
 * {@link SecurityClassification} precedent. Java enum ordinals are NOT the
 * wire codepoints — use {@link #code()} for serialisation.
 */
public enum OperationalMode {
    OTHER_MODE(0),
    OPERATIONAL(1),
    TRAINING(2),
    EXERCISE(3),
    MAINTENANCE(4),
    TEST(5);

    private final int code;

    OperationalMode(int code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.77 wire codepoint for this mode. */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code OperationalMode} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-5 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static OperationalMode fromCode(int c) {
        for (OperationalMode v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
