package org.tstrans.rtp;

/**
 * Per-mount stats snapshot. Mirrors tst-py {@code tstrans.rtp.MountStats}.
 *
 * @param bytesPushed         cumulative TS bytes pushed through this mount's fanout
 * @param packetsPushed       cumulative RTP-sized chunks broadcast through this mount
 * @param peerCount           live subscriber count on the broadcast channel
 * @param framesDroppedTotal  sum of per-peer dropped-frame counters from lagging peers
 */
public record MountStats(
    long bytesPushed,
    long packetsPushed,
    long peerCount,
    long framesDroppedTotal) {}
