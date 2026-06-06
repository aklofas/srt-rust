package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.*;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

import java.io.ByteArrayOutputStream;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import org.junit.jupiter.api.Test;
import org.tstrans.RtpException;
import org.tstrans.codec.NalUnit;
import org.tstrans.codec.VideoUnit;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.Demuxer;

/**
 * Cross-binding loopback parity test for the JVM RTP transport surface.
 *
 * <p>Reproduces the {@code h264-st0601-mp} committed scenario over a real
 * RTP-over-UDP socket pair (loopback on an ephemeral port), then demuxes the
 * received bytes and asserts the same {@code video.payload_sha256} the committed
 * golden carries — the same hash the Rust and Python cross-binding adapters
 * assert. The JVM analogue of tst-py's RTP loopback integration test.
 *
 * <h2>What this proves</h2>
 * <ul>
 *   <li><b>RTP is byte-faithful.</b> The scenario {@code input.ts} (752 bytes,
 *       4 × 188) is below the 1316-byte packet size, so a single
 *       {@link Sender#send(byte[])} produces one RTP datagram and a single
 *       {@link Receiver#recv()} returns all 752 bytes (verified against
 *       {@code crates/tst-rtp/tests/rtp/loopback_unicast.rs}: one
 *       {@code send_bytes} → one datagram → one {@code recv_bytes}). The
 *       received bytes must equal {@code input.ts} exactly.</li>
 *   <li><b>Demux is parity-correct.</b> The RTP-received bytes demux to the same
 *       {@code video.payload_sha256} as the committed golden.</li>
 * </ul>
 *
 * <h2>Topology + safety</h2>
 * UDP is connectionless — there is no SRT-style connect/close linger, so the
 * discover-then-rebind free-port pattern is safe here. The blocking
 * {@link Receiver#recv()} loop runs on a daemon thread feeding a
 * {@link CompletableFuture}; a watchdog cancels a pre-obtained
 * {@link CancelHandle} after a ceiling so {@code recv()} can never wedge the
 * gating macOS/Windows runners. Linux-gated (real sockets), mirroring the Rust
 * {@code #![cfg(target_os = "linux")]} gate on the Rust live-socket tests.
 */
class RtpLoopbackScenarioTest {

    private static final String SCENARIO_ID = "h264-st0601-mp";
    private static final int TIMEOUT_SEC = 15;

    private static boolean isLinux() {
        return System.getProperty("os.name", "").toLowerCase().contains("linux");
    }

    /** Workspace-relative shared scenario dir; resolved from Gradle's user.dir (bindings/jvm). */
    private static Path scenarioDir() {
        return Path.of(
                System.getProperty("user.dir"), "..", "..",
                "crates/tst-integration/tests/fixtures/scenarios", SCENARIO_ID)
            .normalize();
    }

    /** Bind a throwaway UDP socket to :0, read the kernel-picked port, release it. */
    private static int freeUdpPort() throws Exception {
        try (DatagramSocket s = new DatagramSocket(new InetSocketAddress("127.0.0.1", 0))) {
            return s.getLocalPort();
        }
    }

    @Test
    void rtpLoopbackReproducesH264St0601MpGolden() throws Exception {
        assumeTrue(isLinux(), "RTP live-socket loopback gated to Linux (same as #![cfg(target_os = \"linux\")] in Rust)");

        Path dir = scenarioDir();
        Path inputPath  = dir.resolve("input.ts");
        Path goldenPath = dir.resolve("golden.json");
        assertTrue(Files.isRegularFile(inputPath),
            "shared scenario input missing (expected committed fixture): " + inputPath);
        assertTrue(Files.isRegularFile(goldenPath),
            "shared scenario golden missing (expected committed fixture): " + goldenPath);

        byte[] input = Files.readAllBytes(inputPath);
        String expectedSha = extractVideoPayloadSha256(
            Files.readString(goldenPath, StandardCharsets.UTF_8));

        int port = freeUdpPort();

        // Bind the receiver on the main thread (UDP bind does not block), then
        // run the blocking recv loop on a daemon thread feeding a future. The
        // accumulation loop is robust to fragmentation, but the single-datagram
        // path (752 < 1316) completes after the first recv.
        Receiver receiver = Receiver.fromUrl("rtp://127.0.0.1:" + port);
        CancelHandle watchdogHandle = receiver.cancelHandle(); // pre-obtained
        CompletableFuture<byte[]> receivedFuture = new CompletableFuture<>();

        Thread receiverThread = new Thread(() -> {
            try {
                ByteArrayOutputStream buf = new ByteArrayOutputStream(input.length);
                while (buf.size() < input.length) {
                    byte[] chunk;
                    try {
                        chunk = receiver.recv();
                    } catch (RtpException e) {
                        if (e.kind() == RtpException.Kind.CANCELLED) break;
                        receivedFuture.completeExceptionally(e);
                        return;
                    }
                    buf.write(chunk);
                }
                receivedFuture.complete(buf.toByteArray());
            } catch (Exception ex) {
                receivedFuture.completeExceptionally(ex);
            }
        });
        receiverThread.setDaemon(true);
        receiverThread.start();

        // Watchdog: cancel the parked recv after the ceiling so it can never hang.
        Thread watchdog = new Thread(() -> {
            try {
                Thread.sleep(TimeUnit.SECONDS.toMillis(TIMEOUT_SEC - 3));
                watchdogHandle.cancel();
            } catch (InterruptedException ignored) {
            }
        });
        watchdog.setDaemon(true);
        watchdog.start();

        // Sender on the main thread: send the scenario bytes once.
        try (Sender sender = Sender.fromUrl("rtp://127.0.0.1:" + port)) {
            sender.send(input);
            // Give the datagram time to traverse loopback + be drained.
            Thread.sleep(500);
        }

        byte[] received;
        try {
            received = receivedFuture.get(TIMEOUT_SEC, TimeUnit.SECONDS);
        } catch (Exception e) {
            throw new AssertionError("receiver thread failed to complete", e);
        } finally {
            watchdogHandle.cancel();
            receiver.close();
            watchdogHandle.close();
        }

        // RTP must be byte-faithful — the received bytes must equal input.ts.
        assertTrue(received.length >= input.length,
            "RTP loopback: received " + received.length + " bytes, expected >= " + input.length
                + " (the input.ts length); transport appears to have lost data");
        assertArrayEquals(input, Arrays.copyOf(received, input.length),
            "RTP loopback must be byte-faithful over the first " + input.length + " bytes");

        // Demux the received bytes and assert the same golden payload_sha256.
        String actualSha = sha256OfDemuxedVideoUnits(Arrays.copyOf(received, input.length));
        assertEquals(expectedSha, actualSha,
            "RTP-received scenario bytes must demux to the golden payload_sha256 "
                + "(cross-binding parity proof: JVM RTP path ≡ Rust/Python)");

        receiverThread.join(TimeUnit.SECONDS.toMillis(TIMEOUT_SEC));
    }

    // ── helpers (replicated from SrtLoopbackScenarioTest) ───────────────────
    //
    // Private to each test class (same package, but Java doesn't inherit private
    // members across class files). Replicated here rather than adding a shared
    // helper class: keeps the test self-contained and easy to grep in isolation.

    /**
     * Demux the given TS bytes and return the SHA-256 hex digest of the
     * concatenated NAL RBSP payloads from the first H.264 video sample that
     * carries typed {@link NalUnit}s. Mirrors the golden-builder derivation
     * in the Rust and Python adapters.
     */
    private static String sha256OfDemuxedVideoUnits(byte[] tsBytes) throws Exception {
        try (Demuxer d = new Demuxer()) {
            d.feed(tsBytes);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Video v && !v.payload().isEmpty()) {
                    return sha256Units(v.payload());
                }
            }
        }
        throw new AssertionError("no typed Video event found in the demuxed bytes");
    }

    /**
     * SHA-256 of the concatenated typed-unit payload bytes. Mirrors the Rust /
     * Python golden builders: concatenate every {@link NalUnit#payload()} (RBSP,
     * Annex-B start codes already stripped by the demuxer).
     */
    private static String sha256Units(List<VideoUnit> units) throws Exception {
        MessageDigest md = MessageDigest.getInstance("SHA-256");
        for (VideoUnit u : units) {
            NalUnit n = (NalUnit) u;
            ByteBuffer view = n.payload().duplicate();
            byte[] bytes = new byte[view.remaining()];
            view.get(bytes);
            md.update(bytes);
        }
        byte[] digest = md.digest();
        StringBuilder sb = new StringBuilder(digest.length * 2);
        for (byte b : digest) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16));
            sb.append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    /**
     * Minimal extraction of the {@code "event":"video"} core object's
     * {@code payload_sha256} field from the golden JSON text. Mirrors
     * {@code extractVideoPayloadSha256} in
     * {@link org.tstrans.srt.SrtLoopbackScenarioTest}.
     */
    private static String extractVideoPayloadSha256(String json) {
        int videoMarker = json.indexOf("\"video\"");
        assertTrue(videoMarker >= 0, "golden has no \"video\" core event: " + json);
        int objStart = json.lastIndexOf('{', videoMarker);
        int objEnd   = json.indexOf('}', videoMarker);
        assertTrue(objStart >= 0 && objEnd > objStart, "could not bound the video core object");
        String obj = json.substring(objStart, objEnd + 1);
        String needle = "\"payload_sha256\"";
        int k = obj.indexOf(needle);
        assertTrue(k >= 0, "golden video object missing payload_sha256: " + obj);
        int firstQuote = obj.indexOf('"', obj.indexOf(':', k + needle.length()) + 1);
        int lastQuote  = obj.indexOf('"', firstQuote + 1);
        assertTrue(firstQuote >= 0 && lastQuote > firstQuote, "malformed payload_sha256 value");
        return obj.substring(firstQuote + 1, lastQuote);
    }
}
