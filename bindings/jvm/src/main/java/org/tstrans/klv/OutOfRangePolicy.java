package org.tstrans.klv;

/**
 * Controls how {@link Klv#encodeUasDatalink(UasDatalinkLs, OutOfRangePolicy)}
 * handles field values that fall outside their ST 0601 mapped range.
 *
 * <p>ST 0601.19 §7.5 defines an "Out of Range" special value (INT_MIN of the
 * underlying signed integer type) for a subset of ranged tags. According to
 * the spec this sentinel is defined for Tags 6, 7, 50, 51, 52, 79, 80, and
 * 90–93 — all of which are encodable as typed fields on a
 * {@link UasDatalinkLs}. All other tags, and any non-finite input, always
 * produce {@link org.tstrans.KlvEncodeException} regardless of policy.
 *
 * <p>Mirrors tst-py's {@code OutOfRangePolicy} and the Rust
 * {@code tst_core::klv::st0601::OutOfRangePolicy}.
 */
public enum OutOfRangePolicy {

    /**
     * Reject out-of-range values with {@link org.tstrans.KlvEncodeException}
     * (kind {@code OUT_OF_RANGE}). This is the default used by the 1-arg
     * {@link Klv#encodeUasDatalink(UasDatalinkLs)} overload.
     */
    ERROR,

    /**
     * Emit the tag's spec-defined Out-of-Range special value
     * ({@code 0x8000} / {@code 0x80000000} for 2-/4-byte signed mappings)
     * instead of throwing. Applies only to the tags whose INT_MIN sentinel
     * means "Out of Range" per ST 0601 (Tags 6, 7, 50, 51, 52, 79, 80,
     * 90–93 — all encodable {@link UasDatalinkLs} fields); all other tags
     * and non-finite inputs still throw.
     */
    INDICATOR,
}
