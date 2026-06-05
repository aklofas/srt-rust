package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.266 / VVC Picture Parameter Set.
 * Mirrors {@code tst_core::codec::h266::H266Pps} (and tst-py's
 * {@code tstrans.codec.H266Pps}).
 *
 * <p>Only the two id linkage fields are exposed; everything else in the PPS is
 * decoder-internal and stays in {@code rawRbsp}.
 *
 * @param ppsId   {@code pps_pic_parameter_set_id} (H.266 V4 §7.3.2.5)
 * @param spsId   {@code pps_seq_parameter_set_id} linking to an SPS
 * @param rawRbsp original RBSP bytes (heap {@code ByteBuffer})
 */
public record H266Pps(
        int ppsId,
        int spsId,
        ByteBuffer rawRbsp) {
}
