package org.tstrans.klv;

/**
 * MISB ST 0806.4 Table 8-4 Tag 1 top-2-bit data-type code, derived from
 * {@link RvtUserData#numericIdRaw()} — see {@link RvtUserData#dataType()}.
 *
 * <p>Fully enumerated (a 2-bit field has exactly these 4 values); unlike
 * {@link RvtPoiType}/{@link RvtAoiType} there is no wire-unknown catch-all.
 */
public enum RvtUserDataType {
    STRINGS(0),
    INT(1),
    UINT(2),
    EXPERIMENTAL(3);

    private final int code;

    RvtUserDataType(int code) {
        this.code = code;
    }

    /** @return the ST 0806.4 Table 8-4 top-2-bit codepoint (0-3). */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code RvtUserDataType} by its 2-bit codepoint.
     *
     * @param c the codepoint, {@code 0..=3}
     * @return the matching constant
     * @throws IllegalArgumentException if {@code c} is outside {@code 0..=3}
     */
    public static RvtUserDataType fromCode(int c) {
        for (RvtUserDataType v : values()) {
            if (v.code == c) return v;
        }
        throw new IllegalArgumentException("RvtUserDataType code must be 0..=3, got " + c);
    }
}
