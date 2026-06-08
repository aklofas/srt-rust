package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;
import org.tstrans.RtspException;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.VideoCodec;

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

    /**
     * The mount factories lease the server registry entry (twice on the native side). A server
     * closed before the factory runs must surface a clean {@link IllegalStateException} rather
     * than touching freed memory — the registry makes that deterministic.
     */
    @Test @Timeout(15)
    void addMountAfterCloseThrows() throws Exception {
        RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"));
        s.close();
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000).addVideo(0x1011, VideoCodec.H264).build();
        assertThrows(IllegalStateException.class, () -> s.addUnicastMount("/live", cfg));
    }

    @Test @Timeout(15)
    void addUnicastMountPushAndStats() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            MuxerConfig cfg = MuxerConfig.builder()
                .programNumber(1).pmtPid(0x1000).addVideo(0x1011, VideoCodec.H264).build();
            try (MountHandle m = s.addUnicastMount("/live", cfg)) {
                assertEquals("/live", m.mountPath());
                assertEquals("unicast", m.mountKind());
                assertEquals(1, s.stats().mounts());
                assertTrue(m.videoHandle().isPresent());
                assertEquals(1, m.videoHandles().size());
                MountStats before = m.stats();
                m.pushVideo(idr(), 0L, true);
                m.flush();
                assertTrue(m.stats().bytesPushed() > before.bytesPushed());
                m.resetStats();
                assertEquals(0L, m.stats().bytesPushed());
            }
        }
    }

    @Test @Timeout(15)
    void addMulticastMountKind() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            MuxerConfig cfg = MuxerConfig.builder()
                .programNumber(1).pmtPid(0x1000).addVideo(0x1011, VideoCodec.H264).build();
            try (MountHandle m = s.addMulticastMount("/mc", "239.0.0.1", 5004, cfg)) {
                assertEquals("multicast", m.mountKind());
            }
        }
    }

    @Test @Timeout(15)
    void invalidMountPathThrowsMount() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            MuxerConfig cfg = MuxerConfig.builder()
                .programNumber(1).pmtPid(0x1000).addVideo(0x1011, VideoCodec.H264).build();
            RtspException ex = assertThrows(RtspException.class,
                () -> s.addUnicastMount("live", cfg));  // no leading slash
            assertEquals(RtspException.Kind.MOUNT, ex.kind());
        }
    }

    @Test @Timeout(15)
    void duplicateMountThrowsMount() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"))) {
            MuxerConfig cfg = MuxerConfig.builder()
                .programNumber(1).pmtPid(0x1000).addVideo(0x1011, VideoCodec.H264).build();
            try (MountHandle ignored = s.addUnicastMount("/live", cfg)) {
                RtspException ex = assertThrows(RtspException.class,
                    () -> s.addUnicastMount("/live", cfg));
                assertEquals(RtspException.Kind.MOUNT, ex.kind());
            }
        }
    }

    private static byte[] idr() {
        byte[] b = new byte[20];
        b[0]=0; b[1]=0; b[2]=0; b[3]=1; b[4]=0x65;
        for (int i=0;i<15;i++) b[5+i]=(byte)(0xA5 ^ i);
        return b;
    }
}
