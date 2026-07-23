package org.tstrans.klv;

/**
 * MISB ST 0806.4 Table 8-3 Tag 6 — Area of Interest Type.
 *
 * <p>Shares codes 1/2/4 with {@link RvtPoiType}; code 3 is "Reserved" here
 * rather than "Target". Wire-unknown codepoints surface via
 * {@link RvtAoi#aoiTypeCode()} with a {@code null} {@link RvtAoi#aoiType()}
 * typed accessor — mirrors the {@link IcingDetected} precedent. Java enum
 * ordinals are NOT the wire codepoints — use {@link #code()} for serialisation.
 */
public enum RvtAoiType {
    FRIENDLY(1),
    HOSTILE(2),
    RESERVED(3),
    UNKNOWN(4);

    private final int code;

    RvtAoiType(int code) {
        this.code = code;
    }

    /** @return the ST 0806.4 Table 8-3 wire codepoint for this state. */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code RvtAoiType} by ST 0806.4 wire codepoint.
     *
     * @param c the wire codepoint
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static RvtAoiType fromCode(int c) {
        for (RvtAoiType v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
