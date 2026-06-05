package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Parsed H.266 / VVC Video Parameter Set.
 * Mirrors {@code tst_core::codec::h266::H266Vps} (and tst-py's
 * {@code tstrans.codec.H266Vps}).
 *
 * <p>Only {@code vpsId}, {@code maxLayers}, and {@code maxSubLayers} are
 * surfaced; Profile/Tier/Level loops, OLS info, and DPB/HRD parameters stay in
 * {@code rawRbsp} for consumers needing deeper info later.
 *
 * @param vpsId       {@code vps_video_parameter_set_id} (H.266 V4 §7.3.2.3)
 * @param maxLayers   {@code vps_max_layers_minus1 + 1}
 * @param maxSubLayers {@code vps_max_sublayers_minus1 + 1}
 * @param rawRbsp     original RBSP bytes (heap {@code ByteBuffer})
 */
public record H266Vps(
        int vpsId,
        int maxLayers,
        int maxSubLayers,
        ByteBuffer rawRbsp) {
}
