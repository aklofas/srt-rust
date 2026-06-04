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
    }
}
