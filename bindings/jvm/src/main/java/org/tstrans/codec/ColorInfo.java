package org.tstrans.codec;

/**
 * VUI / video signal type metadata. Decoded per ITU-T H.273 / ISO/IEC 23091-2.
 * Mirrors {@code tstrans.codec.ColorInfo}.
 *
 * <p>This is the binding-exposed subset: the Rust {@code ColorInfo} additionally
 * carries {@code chroma_loc} and {@code sample_aspect_ratio}, but — matching
 * tst-py's {@code ColorInfoPy} — they are intentionally omitted here.
 *
 * @param primaries colour primaries
 * @param transfer  transfer characteristics
 * @param matrix    matrix coefficients
 * @param fullRange {@code false} = limited range, {@code true} = full range
 */
public record ColorInfo(
        ColourPrimaries primaries,
        TransferCharacteristics transfer,
        MatrixCoefficients matrix,
        boolean fullRange) {
}
