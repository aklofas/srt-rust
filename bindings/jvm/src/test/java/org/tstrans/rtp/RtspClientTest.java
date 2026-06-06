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
}
