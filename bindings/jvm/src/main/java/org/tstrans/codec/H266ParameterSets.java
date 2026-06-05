package org.tstrans.codec;

import java.util.List;

/**
 * All VPS, SPS, and PPS NAL units parsed from a single slice.
 * Mirrors {@code tst_core::codec::h266::H266ParameterSets} (and tst-py's
 * {@code tstrans.codec.H266ParameterSets}).
 *
 * <p>Unlike the H.265 version (which uses dict-by-id maps), H.266 parameter
 * sets are stored as ordered lists — the underlying Rust collections are
 * {@code Vec<_>}, not {@code BTreeMap<u8, _>}. Use {@code vpses.get(i).vpsId()}
 * etc. to look up by id. The native parser populates the lists incrementally
 * and is partial-success-tolerant.
 *
 * @param vpses parsed VPS units, ordered by {@code vps_id}
 * @param spses parsed SPS units, ordered by {@code sps_id}
 * @param ppses parsed PPS units, ordered by {@code pps_id}
 */
public record H266ParameterSets(
        List<H266Vps> vpses,
        List<H266Sps> spses,
        List<H266Pps> ppses) {
}
