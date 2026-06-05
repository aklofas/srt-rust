package org.tstrans.codec;

import java.util.List;

/**
 * Aggregate of all typed structs extracted from a sequence of AV1 OBUs.
 * Mirrors {@code tst_core::codec::av1::Av1ObuStream} (and tst-py's
 * {@code tstrans.codec.Av1ObuStream}).
 *
 * <p>Build a {@link List} of {@link Obu} objects, then call
 * {@link Codec#parseAv1ObuStream(List)} to populate the three fields.
 * Partial-success-tolerant: OBUs that fail to parse accumulate in
 * {@link #unparseable} rather than aborting the walk — the walk itself never
 * throws.
 *
 * @param sequenceHeaders all successfully parsed Sequence Header OBUs, in
 *                        encounter order
 * @param frameHeaders    all successfully parsed Frame Header OBUs, in
 *                        encounter order
 * @param unparseable     each OBU that failed to parse (or a Frame Header
 *                        arriving before any Sequence Header)
 */
public record Av1ObuStream(
        List<Av1SequenceHeader> sequenceHeaders,
        List<Av1FrameHeaderLight> frameHeaders,
        List<UnparseableObu> unparseable) {

    /**
     * One OBU that failed to parse. Mirrors the Rust
     * {@code (u8, CodecParseError)} pair; tst-py exposes it as an
     * {@code (obu_type, error_message)} tuple, modelled here with the error
     * rendered to its display string.
     *
     * @param obuType the 4-bit {@code obu_type} of the failed OBU
     * @param error   the parse-error display message
     */
    public record UnparseableObu(int obuType, String error) {
    }
}
