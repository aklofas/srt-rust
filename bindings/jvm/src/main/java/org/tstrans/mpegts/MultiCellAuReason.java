package org.tstrans.mpegts;

/** Why a multi-cell AU reassembly attempt did not yield a sample. Mirrors {@code tst_core::mpegts::demux::event::MultiCellAuReason}. Set on {@code NonConformant.multiCellAuReason} when kind == MULTI_CELL_AU. */
public enum MultiCellAuReason { ORPHAN, SEQUENCE_GAP, CONCURRENT_FIRST, OVERFLOW, OVERFLOW_TOTAL, TOO_MANY_PIDS }
