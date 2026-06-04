package org.tstrans.scenarios;

import static org.junit.jupiter.api.Assertions.*;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.Test;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.Demuxer;

/**
 * Java adapter for the cross-binding scenario harness (WS-5), mirroring the
 * Python in-process adapter at {@code bindings/python/tests/test_scenarios.py}.
 *
 * <p>It reads the SHARED committed fixtures under
 * {@code crates/tst-integration/tests/fixtures/scenarios/h264-st0601-mp/}
 * ({@code input.ts} + {@code golden.json}) — the same artifacts the Rust and
 * Python adapters consume — demuxes the bytes through the JNI
 * {@link org.tstrans.mpegts.Demuxer} keystone, and asserts the result against
 * the committed golden.
 *
 * <h2>What this test compares (the KEYSTONE SUBSET)</h2>
 * <ul>
 *   <li><b>video.pid</b> — the elementary-stream PID of the video sample.</li>
 *   <li><b>video.pts</b> — the 90&nbsp;kHz presentation timestamp.</li>
 *   <li><b>video.payload_sha256</b> — SHA-256 of the concatenated NAL RBSP
 *       payload bytes. The JNI keystone derives the {@code Sample.payload}
 *       bytes identically to the Rust/Python normalisers (concatenate every
 *       {@code NalUnit.payload}, Annex-B start codes already stripped by the
 *       demuxer — see {@code sample_bytes()} in {@code bindings/jvm/src/mpegts/mod.rs}
 *       and {@code video_payload_bytes()} in the Rust normaliser), so the digest
 *       matches byte-for-byte. This is the strong cross-binding proof.</li>
 * </ul>
 *
 * <h2>What is DEFERRED (not yet reproducible by the keystone)</h2>
 * <ul>
 *   <li>the {@code klv} core event (pid 4145) — the keystone SKIPS
 *       {@code DemuxEvent::Metadata}; KLV typing lands in the completion wave.</li>
 *   <li>video {@code stream_type} / {@code key} / {@code program} — the keystone
 *       {@code Sample} record does not expose the PMT stream_type byte, the
 *       random-access flag, or the program number.</li>
 * </ul>
 * Full golden parity is a completion-wave concern; per the surface-port plan the
 * keystone compares only the supported subset.
 */
class ScenarioReproductionTest {

    private static final String SCENARIO_ID = "h264-st0601-mp";

    /** Workspace-relative shared scenario dir; resolved from Gradle's user.dir (bindings/jvm). */
    private static Path scenarioDir() {
        return Path.of(
                System.getProperty("user.dir"), "..", "..",
                "crates/tst-integration/tests/fixtures/scenarios", SCENARIO_ID)
            .normalize();
    }

    @Test
    void reproducesH264St0601MpVideoSubset() throws Exception {
        Path dir = scenarioDir();
        Path inputPath = dir.resolve("input.ts");
        Path goldenPath = dir.resolve("golden.json");

        // Skip-guard: the shared fixtures ARE committed; their absence is a hard
        // failure (the cross-binding contract relies on the single-sourced golden).
        assertTrue(Files.isRegularFile(inputPath),
            "shared scenario input missing (expected committed fixture): " + inputPath);
        assertTrue(Files.isRegularFile(goldenPath),
            "shared scenario golden missing (expected committed fixture): " + goldenPath);

        byte[] tsBytes = Files.readAllBytes(inputPath);
        String goldenJson = Files.readString(goldenPath, StandardCharsets.UTF_8);

        // Extract the single `video` core event's pid, pts, payload_sha256 from
        // the golden. No JSON lib is on the test classpath (build.gradle.kts has
        // only junit) — hand-roll a minimal extraction over the one video object
        // rather than add a dependency just for this.
        GoldenVideo expected = extractVideoEvent(goldenJson);

        // Demux the shared input through the JNI keystone, collecting VIDEO Samples.
        // Sample.payload is a JVM-owned heap copy, so SHA-256-ing it inside the
        // loop (or later) is equally safe.
        List<VideoSample> videoSamples = new ArrayList<>();
        try (Demuxer d = new Demuxer()) {
            d.feed(tsBytes);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Sample s
                        && s.kind() == DemuxEvent.SampleKind.VIDEO) {
                    videoSamples.add(new VideoSample(s.pid(), s.pts(), sha256(s.payload())));
                }
            }
        }

        // Assert the keystone subset: a VIDEO sample exists matching the golden's
        // pid AND pts.
        VideoSample match = videoSamples.stream()
            .filter(v -> v.pid == expected.pid && v.pts == expected.pts)
            .findFirst()
            .orElse(null);
        assertNotNull(match,
            "no VIDEO Sample matched golden pid=" + expected.pid + " pts=" + expected.pts
                + "; observed=" + videoSamples);

        // payload_sha256 cross-binding proof: the JNI keystone's Sample.payload is
        // the concatenated NAL RBSP bytes (same derivation as the Rust/Python
        // golden builders), so the digest matches the committed golden.
        assertEquals(expected.payloadSha256, match.payloadSha256,
            "video payload_sha256 mismatch — the JNI keystone Sample.payload bytes "
                + "must equal the concatenated NAL RBSP bytes the golden hashes");
    }

    /** SHA-256 the readable contents of a ByteBuffer (without disturbing it). */
    private static String sha256(ByteBuffer buf) throws Exception {
        ByteBuffer view = buf.duplicate();
        byte[] bytes = new byte[view.remaining()];
        view.get(bytes);
        byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
        StringBuilder sb = new StringBuilder(digest.length * 2);
        for (byte b : digest) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16));
            sb.append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    private record VideoSample(int pid, long pts, String payloadSha256) {}

    private record GoldenVideo(int pid, long pts, String payloadSha256) {}

    /**
     * Minimal extraction of the single {@code "event":"video"} core object's
     * {@code pid}, {@code pts}, and {@code payload_sha256} fields from the golden
     * JSON text. Deliberately not a general JSON parser — it locates the video
     * object by its {@code "event": "video"} marker and reads the three scalar
     * fields that follow within that object. Fails loudly if any is absent.
     */
    private static GoldenVideo extractVideoEvent(String json) {
        int videoMarker = json.indexOf("\"video\"");
        assertTrue(videoMarker >= 0, "golden has no \"video\" core event: " + json);
        // The object containing "video" starts at the preceding '{'.
        int objStart = json.lastIndexOf('{', videoMarker);
        int objEnd = json.indexOf('}', videoMarker);
        assertTrue(objStart >= 0 && objEnd > objStart,
            "could not bound the video core object in golden");
        String obj = json.substring(objStart, objEnd + 1);

        int pid = (int) extractNumber(obj, "pid");
        long pts = extractNumber(obj, "pts");
        String sha = extractString(obj, "payload_sha256");
        return new GoldenVideo(pid, pts, sha);
    }

    /** Read an integer-valued JSON field {@code "key": <number>} from {@code obj}. */
    private static long extractNumber(String obj, String key) {
        String needle = "\"" + key + "\"";
        int k = obj.indexOf(needle);
        assertTrue(k >= 0, "golden video object missing field \"" + key + "\": " + obj);
        int colon = obj.indexOf(':', k + needle.length());
        assertTrue(colon >= 0, "malformed golden field \"" + key + "\"");
        int i = colon + 1;
        while (i < obj.length() && (obj.charAt(i) == ' ' || obj.charAt(i) == '\t')) {
            i++;
        }
        int start = i;
        while (i < obj.length() && (Character.isDigit(obj.charAt(i)) || obj.charAt(i) == '-')) {
            i++;
        }
        assertTrue(i > start, "golden field \"" + key + "\" is not a number");
        return Long.parseLong(obj.substring(start, i));
    }

    /** Read a string-valued JSON field {@code "key": "<value>"} from {@code obj}. */
    private static String extractString(String obj, String key) {
        String needle = "\"" + key + "\"";
        int k = obj.indexOf(needle);
        assertTrue(k >= 0, "golden video object missing field \"" + key + "\": " + obj);
        int firstQuote = obj.indexOf('"', obj.indexOf(':', k + needle.length()) + 1);
        assertTrue(firstQuote >= 0, "malformed golden field \"" + key + "\"");
        int lastQuote = obj.indexOf('"', firstQuote + 1);
        assertTrue(lastQuote > firstQuote, "unterminated golden string field \"" + key + "\"");
        return obj.substring(firstQuote + 1, lastQuote);
    }
}
