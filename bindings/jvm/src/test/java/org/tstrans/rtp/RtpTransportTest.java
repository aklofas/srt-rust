package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;
import org.tstrans.RtpException;

class RtpTransportTest {

    @Test
    void senderConstructsStatsCancelClose() throws Exception {
        // UDP "connect" sets the default destination; no peer is required, so
        // construction succeeds with no receiver listening.
        try (Sender s = Sender.fromUrl("rtp://127.0.0.1:50000")) {
            SocketStats st = s.socketStats();
            assertNotNull(st);
            assertEquals(0L, st.bytesSent());
            try (CancelHandle ch = s.cancelHandle()) {
                ch.cancel(); // idempotent, no throw
            }
        }
    }

    @Test
    void senderClosedThenSendThrowsIllegalState() throws Exception {
        Sender s = Sender.fromUrl("rtp://127.0.0.1:50001");
        s.close();
        s.close(); // idempotent
        assertThrows(IllegalStateException.class, () -> s.send(new byte[] {0x47}));
    }

    @Test
    void senderMalformedUrlThrowsRtpTransport() {
        RtpException ex = assertThrows(RtpException.class, () -> Sender.fromUrl("not-a-url"));
        assertEquals(RtpException.Kind.TRANSPORT, ex.kind());
    }

    @Test
    void senderNegativePktSizeThrowsIllegalArgument() {
        assertThrows(IllegalArgumentException.class,
            () -> Sender.fromUrl("rtp://127.0.0.1:50002", -1, null));
    }

    @Test
    void senderSsrcOutOfRangeThrowsIllegalArgument() {
        assertThrows(IllegalArgumentException.class,
            () -> Sender.fromUrl("rtp://127.0.0.1:50003", 1316, 0x1_0000_0000L));
    }

    @Test
    void receiverConstructsStatsCancelClose() throws Exception {
        // Bind to an ephemeral port; do NOT call recv() (it would block).
        try (Receiver r = Receiver.fromUrl("rtp://127.0.0.1:0")) {
            assertNotNull(r.socketStats());
            try (CancelHandle ch = r.cancelHandle()) {
                ch.cancel();
            }
        }
    }

    @Test
    void receiverClosedThenRecvThrowsIllegalState() throws Exception {
        Receiver r = Receiver.fromUrl("rtp://127.0.0.1:0");
        r.close();
        assertThrows(IllegalStateException.class, r::recv);
    }

    @Test
    void receiverMalformedUrlThrowsRtpTransport() {
        RtpException ex = assertThrows(RtpException.class, () -> Receiver.fromUrl("not-a-url"));
        assertEquals(RtpException.Kind.TRANSPORT, ex.kind());
    }

    @Test
    void receiverRejectsPktSizeQuery() {
        RtpException ex = assertThrows(RtpException.class,
                () -> Receiver.fromUrl("rtp://127.0.0.1:0?pkt_size=1316"));
        assertTrue(ex.getMessage().contains("send-side knob"),
                "teaching text expected, got: " + ex.getMessage());
    }
}
