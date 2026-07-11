package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;

import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import org.junit.jupiter.api.Test;
import org.tstrans.RtpException;

/**
 * Unit tests for {@link H264Receiver}.
 *
 * <p>Tests are offline — no live RTSP server. The loopback test hand-builds a
 * single-NALU RFC 3550 + RFC 6184 packet (same bytes as the Rust and Python
 * loopback tests) and sends it to a locally-bound receiver via
 * {@link DatagramSocket}.
 *
 * <h2>Packet layout (RFC 3550 §5.1 + RFC 6184 §5.6)</h2>
 * <pre>
 *   Byte 0:    0x80  (V=2, P=0, X=0, CC=0)
 *   Byte 1:    0x80 | 96  (M=1, PT=96)
 *   Bytes 2-3: seq=1 (big-endian)
 *   Bytes 4-7: ts=0x00002328 = 9000 ticks (big-endian)
 *   Bytes 8-11: ssrc=9 (big-endian)
 *   Byte 12:   0x65 (NALU type 5 = IDR)
 *   Bytes 13+: payload bytes 0xAB, 0xCD
 * </pre>
 * Expected AU: annexb = [0,0,0,1, 0x65, 0xAB, 0xCD]; pts = 0 (zero-based first
 * AU); keyFrame = true; rtpTimestamp = 9000.
 */
class H264ReceiverTest {

    /** Hand-built RFC 3550 + RFC 6184 single-NALU IDR packet. */
    private static final byte[] IDR_PACKET = {
        (byte)0x80,           // V=2, P=0, X=0, CC=0
        (byte)(0x80 | 96),   // M=1, PT=96
        0, 1,                 // seq=1
        0, 0, 0x23, 0x28,    // ts=9000
        0, 0, 0, 9,          // ssrc=9
        0x65,                 // NALU type 5 = IDR slice
        (byte)0xAB, (byte)0xCD  // payload bytes
    };

    // ── Test 1: UDP loopback — single IDR packet ──────────────────────────────

    /**
     * Bind an H264Receiver on an ephemeral port, push one canned IDR packet via
     * DatagramSocket, verify all AU fields.
     *
     * <p>Mirrors the Rust test {@code h264_receiver_udp_loopback_single_au} and the
     * Python test in {@code test_rtp.py}. The port-discovery pattern (bind-to-:0,
     * read port via localAddr()) is required: {@code rtp://127.0.0.1:0} hands port
     * selection to the OS and we discover the actual port from the receiver.
     */
    @Test
    void udpLoopbackSingleIdrPacket() throws Exception {
        try (H264Receiver rx = H264Receiver.listen("rtp://127.0.0.1:0?pt=96")) {
            // Discover ephemeral port — localAddr() returns "host:port".
            String addrStr = rx.localAddr();
            assertNotNull(addrStr, "localAddr() must be non-null for UDP receiver");
            int port = Integer.parseInt(addrStr.substring(addrStr.lastIndexOf(':') + 1));
            assertTrue(port > 0, "bound port must be positive");

            // Push the canned packet from a throwaway DatagramSocket.
            try (DatagramSocket tx = new DatagramSocket(
                    new InetSocketAddress("127.0.0.1", 0))) {
                DatagramPacket pkt = new DatagramPacket(IDR_PACKET, IDR_PACKET.length,
                    new InetSocketAddress("127.0.0.1", port));
                tx.send(pkt);
            }

            // Receive — blocks until packet arrives; returns null at EOS.
            H264AccessUnit au = rx.recvAu();
            assertNotNull(au, "first recvAu() must return an AU");

            // Annex B framing: [0,0,0,1] start code prepended to 0x65, 0xAB, 0xCD.
            byte[] expectedAnnexb = {0, 0, 0, 1, 0x65, (byte)0xAB, (byte)0xCD};
            assertArrayEquals(expectedAnnexb, au.annexb(),
                "annexb must have 4-byte start code + NALU bytes");

            // PTS: zero-based at the first AU (anchor = first RTP ts = 9000;
            // pts = 9000 - 9000 = 0 ticks).
            assertEquals(0L, au.pts(), "pts must be zero-based at first AU");

            assertTrue(au.keyFrame(), "IDR NALU (type 5) must set keyFrame=true");

            // RTP timestamp from the packet header (unsigned 32-bit → long).
            assertEquals(9000L, au.rtpTimestamp(), "rtpTimestamp must equal RTP ts field");

            // toString must not throw.
            assertNotNull(au.toString());
        }
    }

    // ── Test 2: close-then-recvAu → IllegalStateException ─────────────────────

    /**
     * After close(), recvAu() must throw {@link IllegalStateException}.
     * Mirrors DemuxReceiver's closed-state contract.
     */
    @Test
    void closePreventsFurtherRecv() throws Exception {
        H264Receiver rx = H264Receiver.listen("rtp://127.0.0.1:0?pt=96");
        rx.close();
        assertThrows(IllegalStateException.class, rx::recvAu,
            "recvAu() on closed receiver must throw IllegalStateException");
        // Idempotent close.
        rx.close();
    }

    // ── Test 3: config defaults observable from Java ──────────────────────────

    @Test
    void configDefaults() {
        H264DepayConfig cfg = H264DepayConfig.defaults();
        assertEquals(96, cfg.payloadType(), "default payloadType must be 96");
        assertEquals(ParameterSetInjection.BEFORE_IDR, cfg.parameterSetInjection(),
            "default parameterSetInjection must be BEFORE_IDR");
        assertTrue(cfg.initialParameterSets().isEmpty(),
            "default initialParameterSets must be empty");
        assertEquals(8 * 1024 * 1024L, cfg.maxAuBytes(),
            "default maxAuBytes must be 8 MiB (8388608)");
    }

    // ── Test 4: listen with explicit config ──────────────────────────────────

    @Test
    void listenWithConfig() throws Exception {
        H264DepayConfig cfg = H264DepayConfig.builder()
            .parameterSetInjection(ParameterSetInjection.NONE)
            .maxAuBytes(4 * 1024 * 1024L)
            .build();
        try (H264Receiver rx = H264Receiver.listen("rtp://127.0.0.1:0?pt=96", cfg)) {
            assertNotNull(rx.localAddr());
        }
    }

    // ── Test 5: DepayStats and RtpStats constructible ─────────────────────────

    /**
     * depayStats() and rtpStats() must return non-null snapshots with zero counters
     * before any AU has been received.
     */
    @Test
    void statsZeroedBeforeReceive() throws Exception {
        try (H264Receiver rx = H264Receiver.listen("rtp://127.0.0.1:0?pt=96")) {
            H264DepayStats depay = rx.depayStats();
            assertNotNull(depay);
            assertEquals(0L, depay.ausEmitted());
            assertEquals(0L, depay.seqGaps());

            RtpStats rtp = rx.rtpStats();
            assertNotNull(rtp);
            assertEquals(0L, rtp.malformedPackets());

            SocketStats sock = rx.socketStats();
            assertNotNull(sock);
            assertEquals(0L, sock.packetsReceived());
        }
    }

    // ── Test 6: cancelHandle() is usable ─────────────────────────────────────

    @Test
    void cancelHandleUsable() throws Exception {
        try (H264Receiver rx = H264Receiver.listen("rtp://127.0.0.1:0?pt=96")) {
            CancelHandle ch = rx.cancelHandle();
            assertNotNull(ch);
            // cancel() must not throw.
            ch.cancel();
            ch.close();
        }
    }

    // ── Test 7: listen without ?pt= must throw RtpException ──────────────────

    @Test
    void listenWithoutPtThrows() {
        assertThrows(RtpException.class,
            () -> H264Receiver.listen("rtp://127.0.0.1:0"),
            "listen without ?pt= must throw RtpException");
    }

    // ── Test 8: RTSP session consumed-handle double-use ───────────────────────
    // NOTE: This test requires a real RTSP server and is marked @Test but will
    // only exercise the locally-testable path (session misuse without a server).
    // Full RTSP consume-path is exercised in RtspServerClientLoopbackTest.
    //
    // We test that an RtspSession created by connect() (not connectH264()) raises
    // RtspException(PROTOCOL) from intoH264Receiver() — the MP2T-vs-H264 guard.
    // This test cannot run without a server, so we skip it here. The RTSP loopback
    // test (RtspServerClientLoopbackTest) covers the session lifecycle end-to-end.

    // ── Test 9: ParameterSetInjection enum ordinals ──────────────────────────

    @Test
    void parameterSetInjectionOrdinals() {
        // Ordinal stability: the JNI layer passes ordinals to Rust.
        assertEquals(0, ParameterSetInjection.NONE.ordinal());
        assertEquals(1, ParameterSetInjection.BEFORE_IDR.ordinal());
    }
}
