package org.tstrans.mpegts;
/** Continuity / PES-reassembly discontinuity classification. Mirrors {@code tst_core::mpegts::demux::event::DiscontinuityKind} (collapsed to its variant tag, as tst-py's DiscontinuityKindTag does). */
public enum DiscontinuityKind { CONTINUITY_JUMP, PES_OVERSIZE, PES_TOTAL_OVERSIZE, ADAPTATION_FIELD_FLAG }
