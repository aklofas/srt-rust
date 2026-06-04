package org.tstrans.klv;

import java.util.Optional;

/**
 * ST 0102.12 §6.1.12 Tag 12 Object Country Coding Method.
 *
 * <p>Codepoints differ from Tag 2 ({@link ClassifyingCountryCodingMethod}) —
 * the spec is non-contiguous and jumps to 0x40 for {@code GENC_ADMIN_SUB}.
 * For example, ISO-3166 Numeric is 0x03 here but 0x05 on Tag 2. Always use
 * {@link #code()} for wire encoding; never rely on ordinal.
 *
 * <p>{@code OMITTED_VALUE_08} through {@code OMITTED_VALUE_0C} are
 * spec-reserved slots that exist on the wire but are rejected by strict-mode
 * decode.
 */
public enum ObjectCountryCodingMethod {
    ISO_3166_TWO_LETTER(0x01),
    ISO_3166_THREE_LETTER(0x02),
    ISO_3166_NUMERIC(0x03),       // 0x03 here vs 0x05 on Tag 2
    FIPS_104_TWO_LETTER(0x04),    // 0x04 here vs 0x03 on Tag 2
    FIPS_104_FOUR_LETTER(0x05),   // 0x05 here vs 0x04 on Tag 2
    STANAG_1059_TWO_LETTER(0x06),
    STANAG_1059_THREE_LETTER(0x07),
    OMITTED_VALUE_08(0x08),
    OMITTED_VALUE_09(0x09),
    OMITTED_VALUE_0A(0x0A),
    OMITTED_VALUE_0B(0x0B),
    OMITTED_VALUE_0C(0x0C),
    GENC_TWO_LETTER(0x0D),
    GENC_THREE_LETTER(0x0E),
    GENC_NUMERIC(0x0F),
    GENC_ADMIN_SUB(0x40);         // non-contiguous jump — 0x10..=0x3F are unknown

    private final int code;

    ObjectCountryCodingMethod(int code) {
        this.code = code;
    }

    /** @return the ST 0102.12 §6.1.12 wire codepoint for this coding method. */
    public int code() {
        return code;
    }

    /**
     * Look up an {@code ObjectCountryCodingMethod} by ST 0102.12 Tag 12 wire codepoint.
     *
     * @param c the wire codepoint (0x01–0x0F, 0x40 for known values)
     * @return the matching constant, or {@link Optional#empty()} for unknown codepoints
     */
    public static Optional<ObjectCountryCodingMethod> fromCode(int c) {
        for (ObjectCountryCodingMethod v : values()) {
            if (v.code == c) return Optional.of(v);
        }
        return Optional.empty();
    }
}
