package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.264 / AVC Picture Parameter Set.
 * Mirrors {@code tst_core::codec::h264::H264Pps} (and tst-py's
 * {@code tstrans.codec.H264Pps}).
 *
 * @param picParameterSetId  {@code pic_parameter_set_id} ∈ [0, 255]
 * @param seqParameterSetId  {@code seq_parameter_set_id} linking to an SPS ∈ [0, 31]
 * @param entropyCodingMode  {@code CAVLC} or {@code CABAC}
 * @param rawRbsp            original RBSP bytes (heap {@code ByteBuffer})
 */
public record H264Pps(
        int picParameterSetId,
        int seqParameterSetId,
        EntropyCodingMode entropyCodingMode,
        ByteBuffer rawRbsp) {
}
