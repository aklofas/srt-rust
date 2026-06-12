package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

class MuxerFileSinkTest {

    /**
     * Minimal H.264 IDR access unit in Annex-B framing, copied verbatim from
     * {@code MuxRoundtripScenarioTest.syntheticH264Idr()} — the same shape used
     * across all cross-binding mux tests.
     */
    private static byte[] syntheticH264Idr() {
        byte[] buf = new byte[20];
        buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
        buf[4] = 0x65;
        for (int i = 0; i < 15; i++) {
            buf[5 + i] = (byte) (0xA5 ^ i);
        }
        return buf;
    }

    private static final byte[] IDR = syntheticH264Idr();

    private static MuxerConfig videoConfig() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
    }

    @Test
    void writeFileRoundTripsThroughParseFile(@TempDir Path tmp) throws Exception {
        Path out = tmp.resolve("out.ts");
        try (Muxer m = new Muxer(videoConfig());
             MuxerFileSink sink = m.writeFile(out)) {
            sink.pushVideo(IDR, 0L, true);
        }
        assertTrue(Files.size(out) > 0, "writeFile must produce a non-empty .ts");
        long videoEvents;
        try (var s = org.tstrans.io.Io.parseFile(out)) {
            videoEvents = s.filter(e -> e instanceof DemuxEvent.Video).count();
        }
        assertTrue(videoEvents >= 1, "round-tripped file must demux back to >=1 Video event");
    }

    @Test
    void atomicCommitPromotes(@TempDir Path tmp) throws Exception {
        Path out = tmp.resolve("atomic.ts");
        try (Muxer m = new Muxer(videoConfig());
             MuxerFileSink sink = m.writeFile(out, true)) {
            sink.pushVideo(IDR, 0L, true);
            sink.commit();
        }
        assertTrue(Files.isRegularFile(out), "committed atomic write must appear at dest");
        assertTrue(Files.size(out) > 0);
        try (var dir = Files.list(tmp)) {
            assertTrue(dir.noneMatch(p -> p.getFileName().toString().contains(".partial")),
                "no .partial temp may remain after a committed atomic write");
        }
    }

    @Test
    void atomicNoCommitDiscards(@TempDir Path tmp) throws Exception {
        Path out = tmp.resolve("discard.ts");
        try (Muxer m = new Muxer(videoConfig());
             MuxerFileSink sink = m.writeFile(out, true)) {
            sink.pushVideo(IDR, 0L, true);
            // no commit()
        }
        assertFalse(Files.exists(out),
            "atomic write without commit() must NOT appear at the destination");
        try (var dir = Files.list(tmp)) {
            assertTrue(dir.findAny().isEmpty(), "no temp file may remain after a discarded atomic write");
        }
    }

    @Test
    void muxerReusableAfterSink(@TempDir Path tmp) throws Exception {
        try (Muxer m = new Muxer(videoConfig())) {
            try (MuxerFileSink sink = m.writeFile(tmp.resolve("a.ts"))) {
                sink.pushVideo(IDR, 0L, true);
            }
            assertDoesNotThrow(() -> m.pendingPackets());
        }
    }

    @Test
    void fileSinkMirrorsMuxerPushFamily() {
        // W3 gap-class guard: a drain-proxy missing a push method causes silent
        // BufferFull on the bare muxer — pin the sink's push* surface to the
        // muxer's structurally so a new Muxer.push* can't ship without its
        // MuxerFileSink twin.
        java.util.function.Function<Class<?>, java.util.Set<String>> pushNames = c ->
            java.util.Arrays.stream(c.getDeclaredMethods())
                .filter(m -> java.lang.reflect.Modifier.isPublic(m.getModifiers())
                    && m.getName().startsWith("push"))
                .map(java.lang.reflect.Method::getName)
                .collect(java.util.stream.Collectors.toSet());
        assertEquals(pushNames.apply(Muxer.class), pushNames.apply(MuxerFileSink.class),
            "MuxerFileSink must mirror every Muxer push* method (W3 gap-class guard)");
    }
}
