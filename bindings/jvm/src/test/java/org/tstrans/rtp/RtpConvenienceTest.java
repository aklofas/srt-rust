package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;

import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import org.junit.jupiter.api.Test;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.VideoCodec;

/**
 * Standalone (non-live) unit tests for the RTP convenience wrappers. The live
 * MuxSender→RTP→DemuxReceiver cross-binding parity proof is in
 * {@link RtpMuxDemuxLoopbackTest}; this class covers construction, closed-state,
 * stats shape, and the push path.
 *
 * <p>Each {@code MuxSender} targets a port held open by a throwaway
 * {@link DatagramSocket} peer for the duration of the push. This is required on
 * Linux: an {@code rtp://} sender uses a <em>connected</em> UDP socket, and a
 * connected send to a port with no listener fails with {@code ECONNREFUSED}
 * (the kernel surfaces the ICMP port-unreachable). With a bound peer the send
 * succeeds, so the push and the resulting muxer stats are genuinely exercised.
 */
class RtpConvenienceTest {

    private static MuxerConfig videoConfig() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
    }

    private static byte[] idr() {
        byte[] b = new byte[20];
        b[0] = 0; b[1] = 0; b[2] = 0; b[3] = 1; b[4] = 0x65;
        for (int i = 0; i < 15; i++) b[5 + i] = (byte) (0xA5 ^ i);
        return b;
    }

    /** Bind a throwaway UDP peer on 127.0.0.1 and return it (caller closes). */
    private static DatagramSocket peer() throws Exception {
        return new DatagramSocket(new InetSocketAddress("127.0.0.1", 0));
    }

    /**
     * Discover a free UDP port by binding a throwaway socket to :0 and releasing
     * it. Avoids fixed-port flakiness on shared CI runners. (A receiver binds the
     * port, and the rtp transport auto-binds an RTCP companion on {@code port+1},
     * so a literal {@code :0} URL is unusable here — the OS would put RTCP on the
     * privileged port 1; this yields a concrete high ephemeral port instead.)
     */
    private static int freeUdpPort() throws Exception {
        try (DatagramSocket s = new DatagramSocket(new InetSocketAddress("127.0.0.1", 0))) {
            return s.getLocalPort();
        }
    }

    @Test
    void muxSenderConstructsPushesAndReportsStats() throws Exception {
        try (DatagramSocket peer = peer()) {
            String url = "rtp://127.0.0.1:" + peer.getLocalPort();
            try (MuxSender s = MuxSender.fromUrl(url, videoConfig())) {
                assertTrue(s.isAlive());
                s.pushVideo(idr(), 0L, true);
                s.pushVideo(idr(), 3000L, true);

                TransportStats st = s.stats();
                assertNotNull(st);
                assertNotNull(st.socketStats());
                assertNotNull(st.muxerStats());
                // The muxer emitted at least the PAT/PMT + video PES packets.
                assertTrue(st.muxerStats().tsPacketsEmitted() > 0,
                    "muxer should have emitted TS packets after two video pushes");

                assertTrue(s.videoHandle().isPresent());
                assertTrue(s.klvHandle().isEmpty());
                assertTrue(s.dataHandle().isEmpty());
            }
        }
    }

    @Test
    void muxSenderWithExplicitPktSize() throws Exception {
        try (DatagramSocket peer = peer()) {
            String url = "rtp://127.0.0.1:" + peer.getLocalPort();
            try (MuxSender s = MuxSender.fromUrl(url, videoConfig(), 188)) {
                assertTrue(s.isAlive());
                s.pushVideo(idr(), 0L, true);
            }
        }
    }

    @Test
    void muxSenderRejectsNegativePktSize() {
        assertThrows(IllegalArgumentException.class,
            () -> MuxSender.fromUrl("rtp://127.0.0.1:5006", videoConfig(), -1));
    }

    @Test
    void muxSenderClosedStateThrows() throws Exception {
        MuxSender s = MuxSender.fromUrl("rtp://127.0.0.1:5007", videoConfig());
        s.close();
        assertFalse(s.isAlive());
        assertThrows(IllegalStateException.class, () -> s.pushVideo(idr(), 0L, true));
        assertThrows(IllegalStateException.class, s::stats);
        s.close(); // idempotent
    }

    @Test
    void muxSenderRejectsBadUrl() {
        assertThrows(org.tstrans.RtpException.class,
            () -> MuxSender.fromUrl("not-a-url", videoConfig()));
    }

    // ── DemuxReceiver standalone (non-live) tests ──────────────────────────

    @Test
    void demuxReceiverConstructsAndReportsClosedState() throws Exception {
        DemuxReceiver rx = DemuxReceiver.fromUrl("rtp://127.0.0.1:" + freeUdpPort());
        assertTrue(rx.isAlive());
        // No sender → stats are zeroed but the call succeeds.
        TransportStats st = rx.stats();
        assertNotNull(st.socketStats());
        assertNotNull(st.muxerStats());
        // addByteSink before iterating is allowed.
        rx.addByteSink(pkt -> { /* no-op */ });
        rx.close();
        assertFalse(rx.isAlive());
        assertThrows(IllegalStateException.class, () -> rx.addByteSink(pkt -> {}));
        assertThrows(IllegalStateException.class, rx::stats);
        rx.close(); // idempotent
    }

    @Test
    void demuxReceiverRejectsBadUrl() {
        assertThrows(org.tstrans.RtpException.class,
            () -> DemuxReceiver.fromUrl("not-a-url"));
    }

    @Test
    void demuxReceiverWithConfig() throws Exception {
        org.tstrans.mpegts.DemuxerConfig cfg =
            org.tstrans.mpegts.DemuxerConfig.builder().build();
        try (DemuxReceiver rx = DemuxReceiver.fromUrl("rtp://127.0.0.1:" + freeUdpPort(), cfg)) {
            assertTrue(rx.isAlive());
        }
    }
}
