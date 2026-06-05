package org.tstrans.codec;

import java.util.Map;

/**
 * All VPS, SPS, and PPS NAL units parsed from a single slice.
 * Mirrors {@code tst_core::codec::h265::H265ParameterSets} (and tst-py's
 * {@code tstrans.codec.H265ParameterSets}).
 *
 * <p>Keys are the parameter-set ids ({@code Integer}); the underlying Rust
 * collections are {@code BTreeMap<u8, _>}. The native parser populates the maps
 * incrementally and is partial-success-tolerant.
 *
 * @param vpsById mapping of {@code vps_video_parameter_set_id → H265Vps}
 * @param spsById mapping of {@code sps_seq_parameter_set_id → H265Sps}
 * @param ppsById mapping of {@code pps_pic_parameter_set_id → H265Pps}
 */
public record H265ParameterSets(
        Map<Integer, H265Vps> vpsById,
        Map<Integer, H265Sps> spsById,
        Map<Integer, H265Pps> ppsById) {
}
