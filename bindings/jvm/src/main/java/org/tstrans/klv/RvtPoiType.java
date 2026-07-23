package org.tstrans.klv;

/**
 * MISB ST 0806.4 Table 8-2 Tag 5 — Point of Interest Type.
 *
 * <p>Wire-unknown codepoints surface via {@link RvtPoi#poiTypeCode()} with a
 * {@code null} {@link RvtPoi#poiType()} typed accessor — mirrors the
 * {@link IcingDetected} precedent. Java enum ordinals are NOT the wire
 * codepoints — use {@link #code()} for serialisation.
 */
public enum RvtPoiType {
    FRIENDLY(1),
    HOSTILE(2),
    TARGET(3),
    UNKNOWN(4);

    private final int code;

    RvtPoiType(int code) {
        this.code = code;
    }

    /** @return the ST 0806.4 Table 8-2 wire codepoint for this state. */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code RvtPoiType} by ST 0806.4 wire codepoint.
     *
     * @param c the wire codepoint
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static RvtPoiType fromCode(int c) {
        for (RvtPoiType v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
