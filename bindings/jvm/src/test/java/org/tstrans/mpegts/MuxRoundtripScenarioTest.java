package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;

import java.io.ByteArrayOutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import org.junit.jupiter.api.Test;

/**
 * Cross-binding byte-exactness proof for the offline {@link Muxer}: replicate the
 * shared {@code video-roundtrip} mux recipe (single source of truth =
 * {@code crates/tst-integration/src/scenarios/mod.rs::video_roundtrip_ts_bytes()})
 * and assert the SHA-256 of the JNI muxer's TS output equals the committed
 * golden's {@code extensions.output_sha256}. The muxer is integer-only/
 * deterministic, so the digest matches the Rust + Python bindings byte-for-byte.
 */
class MuxRoundtripScenarioTest {

    private static final String SCENARIO_ID = "video-roundtrip";

    private static Path scenarioDir() {
        return Path.of(System.getProperty("user.dir"), "..", "..",
                "crates/tst-integration/tests/fixtures/scenarios", SCENARIO_ID).normalize();
    }

    /** Mirror of the Rust {@code synthetic_h264_idr()}: Annex-B start code + IDR header + filler. */
    private static byte[] syntheticH264Idr() {
        byte[] buf = new byte[20];
        buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
        buf[4] = 0x65;
        for (int i = 0; i < 15; i++) {
            buf[5 + i] = (byte) (0xA5 ^ i);
        }
        return buf;
    }

    @Test
    void reproducesVideoRoundtripBytesExactly() throws Exception {
        Path goldenPath = scenarioDir().resolve("golden.json");
        assertTrue(Files.isRegularFile(goldenPath),
            "shared golden missing (expected committed fixture): " + goldenPath);
        String goldenJson = Files.readString(goldenPath, StandardCharsets.UTF_8);
        // Sanity backstop: this is the committed golden digest
        // (2aa000852931462b875ec9b2548dd8bf5846fef36e5a1084d149cdd636d09a24).
        String expectedSha = extractString(goldenJson, "output_sha256");

        byte[] tsOut = muxAndDrain();
        assertEquals(expectedSha, sha256Hex(tsOut),
            "JNI muxer output must be byte-identical to the Rust/Python video-roundtrip golden");
    }

    /** Replicate video_roundtrip_ts_bytes(): config + one push_video + drain. */
    private static byte[] muxAndDrain() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
        ByteArrayOutputStream acc = new ByteArrayOutputStream();
        byte[] out = new byte[8192];
        try (Muxer m = new Muxer(cfg)) {
            m.pushVideo(syntheticH264Idr(), /*pts=*/ 0L, /*keyFrame=*/ true);
            int n;
            while ((n = m.pull(out)) > 0) {
                acc.write(out, 0, n);
            }
        }
        return acc.toByteArray();
    }

    private static String sha256Hex(byte[] bytes) throws Exception {
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
        StringBuilder sb = new StringBuilder(digest.length * 2);
        for (byte b : digest) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16));
            sb.append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    /** Read a string-valued JSON field {@code "key": "<value>"} (minimal; junit-only classpath). */
    private static String extractString(String json, String key) {
        String needle = "\"" + key + "\"";
        int k = json.indexOf(needle);
        assertTrue(k >= 0, "golden missing field \"" + key + "\": " + json);
        int firstQuote = json.indexOf('"', json.indexOf(':', k + needle.length()) + 1);
        assertTrue(firstQuote >= 0, "malformed golden field \"" + key + "\"");
        int lastQuote = json.indexOf('"', firstQuote + 1);
        assertTrue(lastQuote > firstQuote, "unterminated golden string field \"" + key + "\"");
        return json.substring(firstQuote + 1, lastQuote);
    }
}
