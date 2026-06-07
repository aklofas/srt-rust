package org.tstrans.mpegts;

/** Frozen demuxer-counter snapshot. The 6 scalar fields of
 *  {@code tst_core::mpegts::demux::DemuxerStats} (the per-stream map is not
 *  surfaced, matching tst-py). Counters widened to {@code long}. */
public record DemuxerStats(long programMapsSeen, long pmtVersionsSeen, long discontinuities,
        long nonconformant, long programsSeen, long subtitleStreamsSeen) {}
