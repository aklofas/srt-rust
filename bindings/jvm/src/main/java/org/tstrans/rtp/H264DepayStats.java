package org.tstrans.rtp;

/**
 * RFC 6184 depacketizer counters. Frozen snapshot returned by
 * {@link H264Receiver#depayStats()}.
 *
 * <p>Mirrors tst-py {@code tstrans.rtp.H264DepayStats} and
 * {@code tst_rtp::H264DepayStats}. All counters are cumulative since
 * {@link H264Receiver#listen(String)} (or since the RTSP session started).
 * Counter values are unsigned; Java has no unsigned integer types, so each
 * value is the reinterpreted bit pattern as a signed {@code long}.
 */
public record H264DepayStats(
        /** Number of complete, unpoisoned AUs emitted. */
        long ausEmitted,
        /**
         * Number of AUs discarded due to poisoning (seq gaps, F-bit, etc.).
         * Includes AUs dropped for exceeding {@code maxAuBytes}
         * (those are also counted in {@link #ausDroppedOversize}).
         */
        long ausDropped,
        /**
         * Number of AUs dropped specifically because their accumulated buffer
         * exceeded {@link H264DepayConfig#maxAuBytes()}. Each oversize drop is
         * also counted in {@link #ausDropped}.
         */
        long ausDroppedOversize,
        /** Number of RTP packets discarded (empty, reserved, interleaved types). */
        long packetsDiscarded,
        /** Number of NALUs discarded (F-bit set, open FU at AU completion, etc.). */
        long nalusDiscarded,
        /** Number of sequence-number gaps detected. */
        long seqGaps,
        /** Number of duplicate sequence numbers detected. */
        long duplicatePackets,
        /** Number of times cached parameter sets were updated (in-band SPS/PPS changed). */
        long parameterSetUpdates,
        /** Number of SSRC changes (source restarts) detected. */
        long ssrcChanges
) {}
