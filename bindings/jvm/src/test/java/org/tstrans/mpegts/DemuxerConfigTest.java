package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.file.*;
import org.junit.jupiter.api.Test;
import org.tstrans.DemuxException;

class DemuxerConfigTest {
    // A valid PAT + a PMT with a deliberately-corrupted CRC-32 (564 bytes). Under
    // StrictMode.FULL the demuxer escalates the PsiChecksumMismatch to a hard
    // StrictRejection (→ JNI code STRICT_REJECTION); under the default StrictMode.OFF
    // it tolerates the mismatch (no throw, surfaces a NonConformant event). The
    // sibling `strict-rejection` fixture is NOT used here: it is garbage bytes that
    // hit DemuxError::Unrecoverable (→ INTERNAL) regardless of strictness, so it
    // does not exercise the strict-MODE knob.
    private static final Path FIXTURE =
        Path.of(System.getProperty("user.dir"), "..", "..",
                "crates/tst-integration/tests/fixtures/scenarios/malformed-psi-strict/input.bin")
            .normalize();

    @Test
    void strictFullRejectsNonConformantInput() throws Exception {
        byte[] bytes = Files.readAllBytes(FIXTURE);
        DemuxerConfig cfg = DemuxerConfig.builder().strictMode(StrictMode.FULL).build();
        try (Demuxer d = new Demuxer(cfg)) {
            // The PMT CRC mismatch is detected inside feed()'s packet loop, so the
            // StrictRejection surfaces on feed(), not flush().
            DemuxException ex = assertThrows(DemuxException.class, () -> d.feed(bytes));
            assertEquals(DemuxException.Kind.STRICT_REJECTION, ex.kind(),
                "StrictMode.FULL must reject the corrupted-PMT-CRC fixture with STRICT_REJECTION");
        }
    }

    @Test
    void defaultConfigDoesNotReject() throws Exception {
        // Control: the SAME bytes under the default config (StrictMode.OFF) must
        // NOT throw — proving the strict knob is what changed the outcome. The
        // mismatch is tolerated and surfaced as a NonConformant event instead.
        byte[] bytes = Files.readAllBytes(FIXTURE);
        try (Demuxer d = new Demuxer()) {
            assertDoesNotThrow(() -> {
                d.feed(bytes);
                d.flush();
            });
            boolean sawNonConformant = false;
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.NonConformant) {
                    sawNonConformant = true;
                }
            }
            assertTrue(sawNonConformant,
                "default config tolerates the mismatch and surfaces a NonConformant event");
        }
    }

    @Test
    void negativeCapsAreRejected() {
        // 0 = "use the Rust default" sentinel; a negative would be silently
        // coerced to the default by the JNI bridge, so the builder rejects it.
        assertThrows(IllegalArgumentException.class,
            () -> DemuxerConfig.builder().pesCapPerPid(-1));
        assertThrows(IllegalArgumentException.class,
            () -> DemuxerConfig.builder().pesCapTotal(-1));
        assertThrows(IllegalArgumentException.class,
            () -> DemuxerConfig.builder().auCellCapPerPid(-1));
        assertThrows(IllegalArgumentException.class,
            () -> DemuxerConfig.builder().syncBufCap(-1));
    }

    @Test
    void syncBufCapPermitsWholeFileFeed() throws Exception {
        // 5 MiB of valid TS in one feed: default config raises DemuxException
        // with a message naming sync_buf_cap; raised cap accepts it.
        byte[] pkt = new byte[188];
        pkt[0] = 0x47;
        pkt[1] = 0x1f;
        pkt[2] = (byte) 0xff;
        pkt[3] = 0x10;
        java.util.Arrays.fill(pkt, 4, 188, (byte) 0xff);
        int count = (5 * 1024 * 1024) / 188 + 1;
        byte[] data = new byte[count * 188];
        for (int i = 0; i < count; i++) {
            System.arraycopy(pkt, 0, data, i * 188, 188);
        }

        // Default config: feed of 5 MiB throws DemuxException with SYNC_LOSS kind.
        try (Demuxer d = new Demuxer()) {
            DemuxException ex = assertThrows(DemuxException.class, () -> d.feed(data));
            assertTrue(ex.getMessage().contains("sync_buf_cap"),
                "error message must mention sync_buf_cap; got: " + ex.getMessage());
        }

        // Raised cap: same feed must not throw.
        DemuxerConfig cfg = DemuxerConfig.builder()
            .syncBufCap(16L * 1024 * 1024)
            .build();
        try (Demuxer d = new Demuxer(cfg)) {
            assertDoesNotThrow(() -> d.feed(data));
        }
    }
}
