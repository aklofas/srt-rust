package org.tstrans.srt;

/**
 * Frozen wire-level statistics snapshot. Mirror of
 * {@code tst_core::transport::SocketStats} / tst-py {@code tstrans.srt.SocketStats}.
 * All counters are widened to {@code long} (Java has no unsigned). For
 * SRT-specific extras (estimated bandwidth, symmetric loss split) use
 * {@link SrtStats}.
 */
public record SocketStats(
        long rttUs,
        long sendBandwidthBps,
        long recvBandwidthBps,
        long linkBandwidthBps,
        long bytesSent,
        long packetsSent,
        long bytesReceived,
        long packetsReceived,
        long bytesLostRecv,
        long packetsLostRecv,
        long packetsLostSend,
        long packetsRetransmitted,
        long packetsDroppedSend,
        long packetsDroppedRecv,
        long sendBufferPackets,
        long recvBufferPackets) {}
