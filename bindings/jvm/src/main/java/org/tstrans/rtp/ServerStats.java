package org.tstrans.rtp;

/**
 * Aggregate {@link RtspServer} stats snapshot. Mirrors tst-py
 * {@code tstrans.rtp.ServerStats}.
 *
 * @param activeSessions      live accepted-and-not-closed client sessions
 * @param totalRtpPacketsSent cumulative RTP packets across all peers + mounts
 * @param totalRtpBytesSent   cumulative RTP bytes across all peers + mounts
 * @param mounts              number of registered mounts
 */
public record ServerStats(
    long activeSessions,
    long totalRtpPacketsSent,
    long totalRtpBytesSent,
    long mounts) {}
