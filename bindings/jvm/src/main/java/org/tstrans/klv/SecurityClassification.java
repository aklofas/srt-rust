package org.tstrans.klv;

import java.util.Optional;

/**
 * ST 0102.12 §6.1.1 Tag 1 Security Classification.
 *
 * <p>Codepoints are 0x01–0x05. Unknown codepoints from the wire surface as
 * a {@code null} typed accessor ({@link SecurityLs#securityClassification()}
 * returns empty) while the raw code is preserved in
 * {@link SecurityLs#securityClassificationCode()}. Java enum ordinals are
 * NOT the wire codepoints — use {@link #code()} for serialisation.
 */
public enum SecurityClassification {
    UNCLASSIFIED(0x01),
    RESTRICTED(0x02),
    CONFIDENTIAL(0x03),
    SECRET(0x04),
    TOP_SECRET(0x05);

    private final int code;

    SecurityClassification(int code) {
        this.code = code;
    }

    /** @return the ST 0102.12 §6.1.1 wire codepoint for this classification. */
    public int code() {
        return code;
    }

    /**
     * Look up a {@code SecurityClassification} by ST 0102.12 wire codepoint.
     *
     * @param c the wire codepoint (0x01–0x05 for known values)
     * @return the matching constant, or {@link Optional#empty()} for unknown codepoints
     */
    public static Optional<SecurityClassification> fromCode(int c) {
        for (SecurityClassification v : values()) {
            if (v.code == c) return Optional.of(v);
        }
        return Optional.empty();
    }
}
