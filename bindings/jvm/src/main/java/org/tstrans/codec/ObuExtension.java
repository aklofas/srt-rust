package org.tstrans.codec;

/**
 * AV1 OBU extension header (AV1 Bitstream Spec §5.3.3), present when
 * {@code obu_extension_flag = 1}. Mirrors {@code tstrans.codec.ObuExtension}.
 *
 * @param temporalId 3-bit {@code temporal_id}
 * @param spatialId  2-bit {@code spatial_id}
 */
public record ObuExtension(int temporalId, int spatialId) {
}
