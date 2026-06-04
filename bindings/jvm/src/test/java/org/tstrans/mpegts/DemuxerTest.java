package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.nio.file.*;
import org.junit.jupiter.api.Test;

class DemuxerTest {
    private static final Path FIXTURE =
        Path.of(System.getProperty("user.dir"), "..", "..",
                "crates/tst-core/tests/fixtures/audio/mp2.ts").normalize();

    @Test
    void demuxesFixtureToEvents() throws Exception {
        byte[] ts = Files.readAllBytes(FIXTURE);
        int events = 0; boolean sawProgramMap = false;
        try (Demuxer d = new Demuxer()) {
            d.feed(ts);
            d.flush();
            for (DemuxEvent e : d) {
                events++;
                if (e instanceof DemuxEvent.ProgramMap pm) {
                    sawProgramMap = true;
                    assertFalse(pm.elementaryPids().isEmpty(), "expected >=1 elementary stream");
                }
                if (e instanceof DemuxEvent.Sample s) {
                    assertTrue(s.pid() > 0);
                    assertNotNull(s.payload());
                }
            }
        }
        assertTrue(events > 0, "expected demux events");
        assertTrue(sawProgramMap, "expected a ProgramMap event");
    }

    @Test
    void samplePayloadIsRetainableHeapCopy() throws Exception {
        // mp2.ts yields audio Samples (keystone maps Audio -> AUDIO Sample).
        // Sample.payload is a JVM-owned heap copy (not a direct buffer over Rust
        // memory), so it stays valid even after further pulls and close().
        byte[] ts = Files.readAllBytes(FIXTURE);
        java.nio.ByteBuffer retained = null;
        byte[] snapshot = null;
        try (Demuxer d = new Demuxer()) {
            d.feed(ts);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Sample s) {
                    retained = s.payload();
                    assertFalse(retained.isDirect(),
                        "Sample.payload is a copied heap ByteBuffer, safe to retain");
                    assertTrue(retained.remaining() > 0, "expected non-empty Sample payload");
                    snapshot = new byte[retained.remaining()];
                    retained.duplicate().get(snapshot);
                    break;
                }
            }
            // Drain the rest — this would clobber a zero-copy backing store, but
            // the heap copy is independent of demuxer state.
            for (DemuxEvent ignored : d) {
                // intentionally empty
            }
        }
        // Demuxer is now closed; the JVM-owned copy is still readable and intact.
        assertNotNull(retained, "expected at least one Sample event from mp2.ts");
        byte[] afterClose = new byte[retained.remaining()];
        retained.duplicate().get(afterClose);
        assertArrayEquals(snapshot, afterClose,
            "retained heap payload must stay valid after further pulls and close()");
    }

    @Test
    void feedAfterCloseThrows() {
        Demuxer d = new Demuxer();
        d.close();
        assertThrows(IllegalStateException.class, () -> d.feed(new byte[] {0x47}));
    }
}
