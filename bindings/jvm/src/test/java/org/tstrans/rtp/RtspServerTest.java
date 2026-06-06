package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;
import org.tstrans.RtspException;

class RtspServerTest {

    @Test @Timeout(15)
    void startBindsAndReportsLocalAddr() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            String addr = s.localAddr();
            assertNotNull(addr, "localAddr should be populated after start()");
            assertTrue(addr.startsWith("127.0.0.1:"));
            ServerStats st = s.stats();
            assertEquals(0L, st.activeSessions());
            assertEquals(0L, st.mounts());
        }
    }

    @Test @Timeout(15)
    void cancelHandleToggles() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            try (RtspServerCancelHandle h = s.cancelHandle()) {
                assertFalse(h.isCancelled());
                h.cancel();
                assertTrue(h.isCancelled());
            }
        }
    }

    @Test @Timeout(15)
    void stopIsIdempotent() throws Exception {
        RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"));
        s.stop();
        s.stop(); // no-op second time
        s.close();
    }

    @Test @Timeout(15)
    void tlsFieldsRaiseTlsAtStart() {
        RtspServerConfig cfg = RtspServerConfig.builder()
            .bindAddr("127.0.0.1:0")
            .tlsCertPem(new byte[]{1}).tlsKeyPem(new byte[]{2})
            .build();
        RtspException ex = assertThrows(RtspException.class, () -> RtspServer.start(cfg));
        assertEquals(RtspException.Kind.TLS, ex.kind());
    }

    @Test @Timeout(15)
    void authWithoutRealmRejected() {
        RtspServerConfig cfg = RtspServerConfig.builder()
            .bindAddr("127.0.0.1:0")
            .auth(new BasicAuth("u", "p")) // no realm
            .build();
        assertThrows(IllegalArgumentException.class, () -> RtspServer.start(cfg));
    }

    @Test @Timeout(15)
    void startWithBasicAuthSucceeds() throws Exception {
        RtspServerConfig cfg = RtspServerConfig.builder()
            .bindAddr("127.0.0.1:0")
            .auth(new BasicAuth("admin", "secret", "tst"))
            .build();
        try (RtspServer s = RtspServer.start(cfg)) {
            assertNotNull(s.localAddr());
        }
    }

    @Test @Timeout(15)
    void startWithDigestAuthSucceeds() throws Exception {
        RtspServerConfig cfg = RtspServerConfig.builder()
            .bindAddr("127.0.0.1:0")
            .auth(new DigestAuth("admin", "secret", DigestAlgorithm.SHA256, "tst"))
            .build();
        try (RtspServer s = RtspServer.start(cfg)) {
            assertNotNull(s.localAddr());
        }
    }

    @Test @Timeout(15)
    void usingAfterCloseThrows() throws Exception {
        RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"));
        s.close();
        assertThrows(IllegalStateException.class, s::stats);
    }
}
