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
                    assertEquals(0x1000, pm.pmtPid(), "mp2.ts PAT declares PMT at 0x1000");
                }
                if (e instanceof DemuxEvent.Audio a) {
                    assertTrue(a.stream().pid() > 0);
                    // mp2.ts is a clean MP2 stream → typed frame list, no
                    // bytes-fallback, no parse error.
                    assertNotNull(a.payload());
                    assertNull(a.rawPayload(), "clean MP2 parse has no rawPayload");
                    assertNull(a.codecParseError(), "clean MP2 parse has no codecParseError");
                }
            }
        }
        assertTrue(events > 0, "expected demux events");
        assertTrue(sawProgramMap, "expected a ProgramMap event");
    }

    @Test
    void samplePayloadIsRetainableHeapCopy() throws Exception {
        // mp2.ts yields DemuxEvent.Audio events with typed Mpeg2AudioFrame
        // payloads. Each frame's payload is a JVM-owned heap copy (not a direct
        // buffer over Rust memory), so it stays valid even after further pulls
        // and close().
        byte[] ts = Files.readAllBytes(FIXTURE);
        java.nio.ByteBuffer retained = null;
        byte[] snapshot = null;
        try (Demuxer d = new Demuxer()) {
            d.feed(ts);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Audio a && !a.payload().isEmpty()) {
                    org.tstrans.codec.Mpeg2AudioFrame frame =
                        (org.tstrans.codec.Mpeg2AudioFrame) a.payload().get(0);
                    retained = frame.payload();
                    assertFalse(retained.isDirect(),
                        "frame payload is a copied heap ByteBuffer, safe to retain");
                    assertTrue(retained.remaining() > 0, "expected non-empty frame payload");
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
        assertNotNull(retained, "expected at least one Audio frame from mp2.ts");
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
