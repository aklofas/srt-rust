package org.tstrans.rtp;

/**
 * Frozen wire-level statistics snapshot for an RTP {@link Sender} / {@link Receiver}.
 * Mirror of {@code tst_core::transport::SocketStats} and tst-py
 * {@code tstrans.rtp.SocketStats}.
 *
 * <p><b>Unsigned reinterpretation:</b> Rust's counters are unsigned; Java has no
 * unsigned integer types, so each value is the reinterpreted bit pattern as a
 * signed {@code long}. For the magnitudes seen here (byte/packet counts) the
 * values are effectively non-negative.
 *
 * <p>This is a distinct type from {@link org.tstrans.srt.SocketStats} (same
 * field set, different transport). {@code RtpTransport} populates the send-side
 * counters; {@code RtpRecvTransport} the receive-side. RTCP-derived fields
 * ({@code rttUs}, {@code packetsLost*}) stay zero until RTCP ingest is wired.
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
        long recvBufferPackets) {
}
