package org.tstrans.rtp;

/**
 * RTSP session RTCP-derived stats snapshot. Mirrors tst-py
 * {@code tstrans.rtp.RtspStats}. RTCP fields are populated only when the session
 * is in PLAY and the server has sent at least one RR/SR; wave C always reports a
 * zeroed snapshot (the counters wire in with the RTCP-over-RTP data plane).
 *
 * <p>Unsigned counters are reinterpreted as signed {@code long}/{@code int}; for
 * the magnitudes seen here they are effectively non-negative.
 *
 * @param rrPacketsReceived   RTCP Receiver Reports received
 * @param srPacketsReceived   RTCP Sender Reports received
 * @param rrPacketsSent       RTCP Receiver Reports sent
 * @param srPacketsSent       RTCP Sender Reports sent
 * @param interarrivalJitterUs interarrival jitter (microseconds, u32)
 * @param fractionLostQ8       fraction lost, 8-bit fixed point (u8)
 */
public record RtspStats(
        long rrPacketsReceived,
        long srPacketsReceived,
        long rrPacketsSent,
        long srPacketsSent,
        long interarrivalJitterUs,
        int fractionLostQ8) {
}
