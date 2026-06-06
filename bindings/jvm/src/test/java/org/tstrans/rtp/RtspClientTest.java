package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

/**
 * Offline surface tests for the RTSP client (org.tstrans.rtp wave C). Mirrors
 * tst-py's {@code tests/test_rtsp_client.py}. No live RTSP server — the live
 * RtspClient↔RtspServer loopback is wave D's capstone.
 */
class RtspClientTest {
    // ---- enums ----
    @Test void transportPrefMembers() {
        assertArrayEquals(
            new TransportPref[]{TransportPref.AUTO, TransportPref.UDP, TransportPref.TCP},
            TransportPref.values());
    }
    @Test void digestAlgorithmMembers() {
        assertArrayEquals(
            new DigestAlgorithm[]{DigestAlgorithm.MD5, DigestAlgorithm.SHA256},
            DigestAlgorithm.values());
    }
    @Test void rtspVersionMembers() {
        assertArrayEquals(
            new RtspVersion[]{RtspVersion.V1_0, RtspVersion.V2_0},
            RtspVersion.values());
    }

    // ---- BasicAuth ----
    @Test void basicAuthExposesUserAndRealm() {
        var a = new BasicAuth("alice", "hunter2");
        assertEquals("alice", a.user());
        assertTrue(a.realm().isEmpty());
        var b = new BasicAuth("alice", "hunter2", "myrealm");
        assertEquals("myrealm", b.realm().orElseThrow());
    }
    @Test void basicAuthToStringRedactsPassword() {
        var a = new BasicAuth("alice", "topsecret");
        var s = a.toString();
        assertTrue(s.contains("alice"));
        assertFalse(s.contains("topsecret"));
        assertTrue(s.contains("<redacted>"));
    }
    @Test void basicAuthHasNoPublicPasswordAccessor() {
        for (var method : BasicAuth.class.getMethods()) {
            assertNotEquals("password", method.getName(),
                "BasicAuth must not expose a public password accessor");
        }
    }

    // ---- DigestAuth ----
    @Test void digestAuthDefaultsToMd5() {
        var a = new DigestAuth("bob", "x");
        assertEquals("bob", a.user());
        assertEquals(DigestAlgorithm.MD5, a.algorithm());
    }
    @Test void digestAuthAcceptsSha256() {
        var a = new DigestAuth("bob", "x", DigestAlgorithm.SHA256);
        assertEquals(DigestAlgorithm.SHA256, a.algorithm());
    }
    @Test void digestAuthToStringRedactsPassword() {
        var a = new DigestAuth("bob", "topsecret", DigestAlgorithm.SHA256);
        var s = a.toString();
        assertTrue(s.contains("bob"));
        assertFalse(s.contains("topsecret"));
        assertTrue(s.contains("<redacted>"));
    }
    @Test void digestAuthHasNoPublicPasswordAccessor() {
        for (var method : DigestAuth.class.getMethods()) {
            assertNotEquals("password", method.getName(),
                "DigestAuth must not expose a public password accessor");
        }
    }

    // ---- RtspClientConfig ----
    @Test void configDefaults() {
        var cfg = RtspClientConfig.of("rtsp://127.0.0.1:8554/live");
        assertEquals("rtsp://127.0.0.1:8554/live", cfg.url());
        assertTrue(cfg.auth().isEmpty());
        assertEquals(TransportPref.AUTO, cfg.transportPref());
        assertTrue(cfg.rtcp());
        assertTrue(cfg.tlsRootCertsPem().isEmpty());
        assertTrue(cfg.keepalive());
        assertEquals(RtspVersion.V1_0, cfg.rtspVersion());
    }
    @Test void configAcceptsBasicAuth() {
        var a = new BasicAuth("alice", "x");
        var cfg = RtspClientConfig.builder("rtsp://h/p").auth(a).build();
        assertSame(a, cfg.auth().orElseThrow());
    }
    @Test void configAcceptsDigestAuth() {
        var a = new DigestAuth("bob", "x", DigestAlgorithm.SHA256);
        var cfg = RtspClientConfig.builder("rtsp://h/p").auth(a).build();
        assertSame(a, cfg.auth().orElseThrow());
    }
    @Test void configRejectsEmptyUrl() {
        var ex = assertThrows(IllegalArgumentException.class,
            () -> RtspClientConfig.of(""));
        assertTrue(ex.getMessage().contains("url must not be empty"));
    }
    @Test void configRejectsArbitraryAuth() {
        var ex = assertThrows(IllegalArgumentException.class,
            () -> RtspClientConfig.builder("rtsp://h/p").auth("alice:hunter2"));
        assertTrue(ex.getMessage().contains("auth must be"));
    }
    @Test void configAcceptsNullAuthExplicitly() {
        var cfg = RtspClientConfig.builder("rtsp://h/p").auth(null).build();
        assertTrue(cfg.auth().isEmpty());
    }
    @Test void configAcceptsTransportPref() {
        var cfg = RtspClientConfig.builder("rtsp://h/p")
            .transportPref(TransportPref.TCP).build();
        assertEquals(TransportPref.TCP, cfg.transportPref());
    }
    @Test void configAcceptsTlsPemBytes() {
        byte[] pem = "-----BEGIN CERTIFICATE-----\nMIID\n-----END CERTIFICATE-----\n"
            .getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        var cfg = RtspClientConfig.builder("rtsps://h/p").tlsRootCertsPem(pem).build();
        assertArrayEquals(pem, cfg.tlsRootCertsPem().orElseThrow());
    }
    @Test void configToStringRedactsPemAndAuth() {
        var a = new BasicAuth("alice", "topsecret");
        var cfg = RtspClientConfig.builder("rtsp://h/p")
            .auth(a).tlsRootCertsPem("PEMBYTES".getBytes(java.nio.charset.StandardCharsets.US_ASCII)).build();
        var s = cfg.toString();
        assertFalse(s.contains("topsecret"));
        assertFalse(s.contains("PEMBYTES"));
        assertTrue(s.contains("<auth>"));
        assertTrue(s.contains("<bytes>"));
    }
    @Test void configTlsPemDefensivelyCopiedOnRead() {
        byte[] pem = "CERT".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        var cfg = RtspClientConfig.builder("rtsps://h/p").tlsRootCertsPem(pem).build();
        byte[] got = cfg.tlsRootCertsPem().orElseThrow();
        got[0] = 0x00;                                                 // mutate the returned copy
        assertArrayEquals(pem, cfg.tlsRootCertsPem().orElseThrow());   // internal state unchanged
    }

    // ---- RtspCancelHandle shape ----
    @Test void rtspCancelHandleImplementsAutoCloseable() {
        assertTrue(AutoCloseable.class.isAssignableFrom(RtspCancelHandle.class));
        assertDoesNotThrow(() -> RtspCancelHandle.class.getMethod("cancel"));
        assertDoesNotThrow(() -> RtspCancelHandle.class.getMethod("isCancelled"));
        assertDoesNotThrow(() -> RtspCancelHandle.class.getMethod("close"));
    }

    // ---- RtspStats ----
    @Test void rtspStatsComponentsRoundTrip() {
        var s = new RtspStats(1L, 2L, 3L, 4L, 5L, 6);
        assertEquals(1L, s.rrPacketsReceived());
        assertEquals(2L, s.srPacketsReceived());
        assertEquals(3L, s.rrPacketsSent());
        assertEquals(4L, s.srPacketsSent());
        assertEquals(5L, s.interarrivalJitterUs());
        assertEquals(6, s.fractionLostQ8());
    }
    @Test void rtspStatsDefaultIsZero() {
        var s = new RtspStats(0L, 0L, 0L, 0L, 0L, 0);
        assertEquals(0L, s.rrPacketsReceived());
        assertEquals(0, s.fractionLostQ8());
    }
}
