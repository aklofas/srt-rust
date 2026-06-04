package org.tstrans.mpegts;

/** H.222.0 V9 §2.12.4.2 Table 2-157 cell_fragment_indication bits. Mirrors {@code tst_core::mpegts::au_cell::CellFragmentIndication}. Set on {@code NonConformant.observedCfi}/{@code treatedAs} when kind == CFI_TOLERATED. */
public enum CellFragmentIndication { MIDDLE, LAST, FIRST, COMPLETE }
