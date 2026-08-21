package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;
import org.tstrans.RtpException;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.VideoCodec;

/**
 * Unit tests for the public {@link DemuxReceiver#recvEvent()} checked-exception
 * receive method (task D3) — the non-iterator counterpart to {@link
 * DemuxReceiver#iterator()}. The live cross-binding round-trip + byte-sink
 * fixture lives in {@link RtpMuxDemuxLoopbackTest}; this file is scoped to
 * {@code recvEvent()}'s own contract: a persistent {@code ?recv_timeout=}
 * deadline must surface as a directly catchable checked {@link RtpException}
 * (unlike {@link DemuxReceiver#iterator()}, which wraps it in
 * {@link RuntimeException}), and the receiver must stay usable afterward.
 *
 * <p>RTP/UDP is connectionless, so — unlike the srt {@code DemuxReceiver} —
 * there is no cheap clean-EOS fixture (a remote sender closing does not end
 * an rtp iteration; see the class javadoc). This file does not attempt one.
 */
class DemuxReceiverTest {

    private static boolean isLinux() {
        return System.getProperty("os.name", "").toLowerCase().contains("linux");
    }

    private static byte[] syntheticH264Idr() {
        byte[] buf = new byte[20];
        buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
        buf[4] = 0x65;
        for (int i = 0; i < 15; i++) buf[5 + i] = (byte) (0xA5 ^ i);
        return buf;
    }

    private static MuxerConfig roundtripConfig() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
    }

    private static int freeUdpPort() throws Exception {
        try (DatagramSocket s = new DatagramSocket(new InetSocketAddress("127.0.0.1", 0))) {
            return s.getLocalPort();
        }
    }

    /**
     * Persistent {@code ?recv_timeout=200} on a quiet receiver: {@code recvEvent()}
     * must throw {@code RtpException(TIMEOUT)} as a directly catchable checked
     * exception (assertThrows, no {@code RuntimeException} wrapper — that's
     * iterator()-only). The receiver must then stay usable: a real
     * {@code MuxSender} pushes a small H.264 burst over RTP/UDP to the same
     * port, and the SAME receiver's next {@code recvEvent()} call resumes and
     * delivers a real {@link DemuxEvent}.
     */
    @Test
    @Timeout(20)
    void recvEventPersistentTimeoutRaisesCheckedTimeoutThenDeliversRealEvent() throws Exception {
        assumeTrue(isLinux(),
            "RTP live-socket loopback gated to Linux (same as #![cfg(target_os = \"linux\")] in Rust)");

        // Bind, retrying a few times if the discovered port's RTCP companion
        // (port+1) happens to be taken — mirrors DemuxReceiverCloseRaceTest.
        DemuxReceiver rx = null;
        int port = -1;
        for (int attempt = 0; attempt < 8 && rx == null; attempt++) {
            int candidate = freeUdpPort();
            try {
                rx = DemuxReceiver.fromUrl("rtp://127.0.0.1:" + candidate + "?recv_timeout=200");
                port = candidate;
            } catch (RtpException bindCollision) {
                // port+1 was taken; try a different ephemeral port.
            }
        }
        assertNotNull(rx, "could not bind an rtp DemuxReceiver after 8 attempts");

        try (DemuxReceiver receiver = rx) {
            // Phase 1: nothing has been sent. The persistent ?recv_timeout=200
            // deadline must expire as a checked, directly catchable RtpException.
            RtpException ex = assertThrows(RtpException.class, receiver::recvEvent);
            assertEquals(RtpException.Kind.TIMEOUT, ex.kind());

            // Phase 2: resumable — push a real muxed TS stream (PAT/PMT + a
            // handful of H.264 IDRs at distinct PTS values, so the demuxer
            // closes the first video PES well before the sender's final
            // close()-triggered flush) and confirm the SAME receiver's
            // recvEvent() call delivers a real event.
            try (MuxSender tx = MuxSender.fromUrl("rtp://127.0.0.1:" + port, roundtripConfig())) {
                for (int i = 0; i < 8; i++) {
                    tx.sendVideo(syntheticH264Idr(), i * 3000L, true);
                }
                Thread.sleep(300);
            }

            DemuxEvent event = receiver.recvEvent();
            assertNotNull(event,
                "recvEvent() must resume and deliver a real event after a prior TIMEOUT");
        }
    }

    /**
     * {@code lastSeenMicros(pid)} (task D7): {@code null} before any event has
     * arrived, {@code null} for an unrecognized PID, and a positive Unix-epoch
     * microsecond count for the configured video PID once at least one event
     * has been demuxed. Reuses the same bind-retry + {@code MuxSender} burst
     * fixture as {@link #recvEventPersistentTimeoutRaisesCheckedTimeoutThenDeliversRealEvent}.
     */
    @Test
    @Timeout(20)
    void lastSeenMicrosTracksLiveStreamAndNullsUnknownPid() throws Exception {
        assumeTrue(isLinux(),
            "RTP live-socket loopback gated to Linux (same as #![cfg(target_os = \"linux\")] in Rust)");

        DemuxReceiver rx = null;
        int port = -1;
        for (int attempt = 0; attempt < 8 && rx == null; attempt++) {
            int candidate = freeUdpPort();
            try {
                rx = DemuxReceiver.fromUrl("rtp://127.0.0.1:" + candidate);
                port = candidate;
            } catch (RtpException bindCollision) {
                // port+1 (the RTCP companion) was taken; try a different ephemeral port.
            }
        }
        assertNotNull(rx, "could not bind an rtp DemuxReceiver after 8 attempts");

        try (DemuxReceiver receiver = rx) {
            // Before any data has arrived, both a real (never-seen) PID and an
            // unrecognized PID must read null.
            assertNull(receiver.lastSeenMicros(0x1011),
                "lastSeenMicros must be null before any event has been demuxed");
            assertNull(receiver.lastSeenMicros(0x1FFF),
                "lastSeenMicros must be null for an unrecognized PID");

            try (MuxSender tx = MuxSender.fromUrl("rtp://127.0.0.1:" + port, roundtripConfig())) {
                for (int i = 0; i < 8; i++) {
                    tx.sendVideo(syntheticH264Idr(), i * 3000L, true);
                }
                Thread.sleep(300);
            }

            // The first demuxed event is typically a ProgramMap (PAT/PMT), not a
            // Video sample — pull until a Video event on the configured PID
            // arrives (bounded so a real regression fails instead of hanging).
            DemuxEvent.Video videoEvent = null;
            for (int i = 0; i < 20 && videoEvent == null; i++) {
                DemuxEvent event = receiver.recvEvent();
                assertNotNull(event, "expected at least one demuxed event");
                if (event instanceof DemuxEvent.Video v) {
                    videoEvent = v;
                }
            }
            assertNotNull(videoEvent, "expected a Video event on the configured PID");

            Long seen = receiver.lastSeenMicros(0x1011);
            assertNotNull(seen, "lastSeenMicros must be non-null for the video PID after delivery");
            assertTrue(seen > 0, "lastSeenMicros must be a positive Unix-epoch microsecond count");

            assertNull(receiver.lastSeenMicros(0x1FFF),
                "lastSeenMicros must stay null for a PID that was never carried");
        }
    }
}
