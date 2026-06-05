package org.tstrans.srt;

import org.tstrans.mpegts.MuxerStats;

/**
 * Combined stats snapshot returned by {@code MuxSender.stats()} and
 * {@code DemuxReceiver.stats()}. Pairs the SRT wire-level
 * {@link SocketStats} with the inner {@link MuxerStats}.
 *
 * <p>This record is the Java stand-in for tst-py's {@code (SocketStats,
 * MuxerStats)} tuple — the same role {@link HostPort} plays for the
 * {@code (host, port)} tuple in the low-level surface.
 *
 * @param socketStats the SRT transport's wire-level counters
 * @param muxerStats  the inner muxer's program / packet totals
 */
public record TransportStats(SocketStats socketStats, MuxerStats muxerStats) {}
