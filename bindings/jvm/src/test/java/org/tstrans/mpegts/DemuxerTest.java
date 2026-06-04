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
    void samplePayloadIsDirectZeroCopyBuffer() throws Exception {
        // mp2.ts yields audio Samples (keystone maps Audio -> AUDIO Sample).
        byte[] ts = Files.readAllBytes(FIXTURE);
        boolean sawSample = false;
        try (Demuxer d = new Demuxer()) {
            d.feed(ts);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Sample s) {
                    // Inspect the payload WHILE this sample is current — the
                    // direct buffer is only valid until the subsequent
                    // nextEvent() pull overwrites the native backing storage
                    // (spec §5.4).
                    assertTrue(s.payload().isDirect(),
                        "Sample.payload must be a zero-copy DIRECT ByteBuffer");
                    assertTrue(s.payload().remaining() > 0,
                        "expected non-empty Sample payload");
                    sawSample = true;
                    break;
                }
            }
        }
        assertTrue(sawSample, "expected at least one Sample event from mp2.ts");
    }

    @Test
    void feedAfterCloseThrows() {
        Demuxer d = new Demuxer();
        d.close();
        assertThrows(IllegalStateException.class, () -> d.feed(new byte[] {0x47}));
    }
}
