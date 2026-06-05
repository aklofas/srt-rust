package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.265 / HEVC Picture Parameter Set.
 * Mirrors {@code tst_core::codec::h265::H265Pps} (and tst-py's
 * {@code tstrans.codec.H265Pps}).
 *
 * <p>Only the two id linkage fields are exposed; everything else in the PPS is
 * decoder-internal.
 *
 * @param ppsPicParameterSetId {@code pps_pic_parameter_set_id} (H.265 §7.4.3.3)
 * @param ppsSeqParameterSetId {@code pps_seq_parameter_set_id} linking to an SPS
 * @param rawRbsp              original RBSP bytes (heap {@code ByteBuffer})
 */
public record H265Pps(
        int ppsPicParameterSetId,
        int ppsSeqParameterSetId,
        ByteBuffer rawRbsp) {
}
