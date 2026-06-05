package org.tstrans.codec;

import java.util.Map;

/**
 * All SPS and PPS NAL units parsed from a single access unit.
 * Mirrors {@code tst_core::codec::h264::H264ParameterSets} (and tst-py's
 * {@code tstrans.codec.H264ParameterSets}).
 *
 * <p>Keys are the parameter-set ids ({@code Integer}); the underlying Rust
 * collections are {@code BTreeMap<u8, _>}. The native parser populates the maps
 * incrementally.
 *
 * @param spsById mapping of {@code seq_parameter_set_id → H264Sps}
 * @param ppsById mapping of {@code pic_parameter_set_id → H264Pps}
 */
public record H264ParameterSets(
        Map<Integer, H264Sps> spsById,
        Map<Integer, H264Pps> ppsById) {
}
