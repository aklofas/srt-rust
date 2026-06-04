package org.tstrans.klv;

import java.util.Optional;

/**
 * ST 0102.12 §6.1.2 Tag 2 Classifying Country / Releasing Instructions
 * Country Coding Method.
 *
 * <p>Tag 2 and Tag 12 ({@link ObjectCountryCodingMethod}) use <em>different</em>
 * codepoints for the same logical coding method — e.g. ISO-3166 Numeric is
 * 0x05 here but 0x03 on Tag 12. Always use {@link #code()} for wire encoding;
 * never rely on ordinal.
 *
 * <p>{@code OMITTED_VALUE_08} and {@code OMITTED_VALUE_09} are
 * spec-reserved slots that exist on the wire but are rejected by strict-mode
 * decode.
 */
public enum ClassifyingCountryCodingMethod {
    ISO_3166_TWO_LETTER(0x01),
    ISO_3166_THREE_LETTER(0x02),
    FIPS_104_TWO_LETTER(0x03),
    FIPS_104_FOUR_LETTER(0x04),
    ISO_3166_NUMERIC(0x05),
    STANAG_1059_TWO_LETTER(0x06),
    STANAG_1059_THREE_LETTER(0x07),
    OMITTED_VALUE_08(0x08),
    OMITTED_VALUE_09(0x09),
    FIPS_104_MIXED(0x0A),
    ISO_3166_MIXED(0x0B),
    STANAG_1059_MIXED(0x0C),
    GENC_TWO_LETTER(0x0D),
    GENC_THREE_LETTER(0x0E),
    GENC_NUMERIC(0x0F),
    GENC_MIXED(0x10);

    private final int code;

    ClassifyingCountryCodingMethod(int code) {
        this.code = code;
    }

    /** @return the ST 0102.12 §6.1.2 wire codepoint for this coding method. */
    public int code() {
        return code;
    }

    /**
     * Look up a {@code ClassifyingCountryCodingMethod} by ST 0102.12 Tag 2 wire codepoint.
     *
     * @param c the wire codepoint (0x01–0x10 for known values)
     * @return the matching constant, or {@link Optional#empty()} for unknown codepoints
     */
    public static Optional<ClassifyingCountryCodingMethod> fromCode(int c) {
        for (ClassifyingCountryCodingMethod v : values()) {
            if (v.code == c) return Optional.of(v);
        }
        return Optional.empty();
    }
}
