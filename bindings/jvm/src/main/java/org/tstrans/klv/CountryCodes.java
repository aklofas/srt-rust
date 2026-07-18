package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.122 Item 122 — Country Codes metadata about the
 * platform's operation and manufacture. Per §8.122.1, a wire length-0 value
 * means "unknown" ({@code null} here), not a distinct empty string.
 *
 * @param codingMethod country-code coding method — an enumeration from
 *                     MISB ST 0102 Table 2 Item 12
 * @param overflight   country of overflight code, or {@code null} if unknown
 * @param operator     country of operator code, or {@code null} if absent/unknown
 * @param manufacture  country of manufacture code, or {@code null} if absent/unknown
 */
public record CountryCodes(long codingMethod, String overflight, String operator, String manufacture) {}
