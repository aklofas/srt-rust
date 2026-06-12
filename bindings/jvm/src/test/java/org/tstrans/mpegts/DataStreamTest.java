package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
import org.tstrans.MuxException;

/**
 * Open-time Rust validation matrix for data streams. Every config carries a
 * video stream alongside the data stream(s): data streams are PCR-ineligible
 * (like subtitles), so a data-only program is rejected up front as a
 * no-PCR-eligible-stream config — which would mask the specific validation
 * each test targets (mirrors the core's own {@code three_data_cfg} fixture).
 */
class DataStreamTest {

    @Test
    void typedStreamTypeRejectedAtOpen() {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0x1B, true) // 0x1B = H.264
            .build();
        MuxException e = assertThrows(MuxException.class, () -> new Muxer(cfg).close());
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
    }

    @Test
    void klvaMasqueradeOn0x06RejectedAtOpen() {
        // 0x06 + KLVA registration descriptor classifies as KLV -> rejected
        byte[] klva = {(byte) 0x05, 4, 'K', 'L', 'V', 'A'};
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0x06, true)
            .streamDescriptorsForData(0, new byte[][] {klva})
            .build();
        MuxException e = assertThrows(MuxException.class, () -> new Muxer(cfg).close());
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
    }

    @Test
    void bare0x06OpensAsData() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0x06, true)
            .build();
        try (Muxer m = new Muxer(cfg)) { /* opens fine */ }
    }

    @Test
    void seventeenDataStreamsRejectedAtOpen() {
        MuxerConfig.Builder b = MuxerConfig.builder().addVideo(0x1011, VideoCodec.H264);
        for (int i = 0; i < 17; i++) b.addData(0x0100 + i, 0xF0, false);
        MuxException e = assertThrows(MuxException.class, () -> new Muxer(b.build()).close());
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
    }

    @Test
    void descriptorsAttributeToTheRightStreamAcrossMultipleDataStreams() throws Exception {
        // KLVA registration on the 0xF0 stream is benign (0xF0 never classifies);
        // a mis-attribution that lands it on the 0x06 stream would trip the
        // masquerade rejection — so a clean open proves per-stream attribution.
        byte[] klva = {(byte) 0x05, 4, 'K', 'L', 'V', 'A'};
        byte[] benign = {(byte) 0x05, 4, 'A', 'R', 'S', 'X'};
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0xF0, true)
            .addData(0x0101, 0x06, true)
            .streamDescriptorsForData(0, new byte[][] {klva})
            .streamDescriptorsForData(1, new byte[][] {benign})
            .build();
        try (Muxer m = new Muxer(cfg)) { /* opens fine — attribution correct */ }

        // And the converse: swapped attribution must reject.
        MuxerConfig swapped = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0xF0, true)
            .addData(0x0101, 0x06, true)
            .streamDescriptorsForData(0, new byte[][] {benign})
            .streamDescriptorsForData(1, new byte[][] {klva})
            .build();
        MuxException e = assertThrows(MuxException.class, () -> new Muxer(swapped).close());
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
    }

    @Test
    void pushDataShorthandErrorMatrix() throws Exception {
        // zero data streams -> INVALID_USAGE
        try (Muxer m = new Muxer(MuxerConfig.builder().addVideo(0x1011, VideoCodec.H264).build())) {
            MuxException e = assertThrows(MuxException.class, () -> m.pushData(new byte[] {1}, 0L));
            assertEquals(MuxException.Kind.INVALID_USAGE, e.kind());
        }
        // two data streams -> ambiguous INVALID_USAGE; pushDataTo resolves it
        MuxerConfig two = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0xF0, true).addData(0x0101, 0xF1, true).build();
        try (Muxer m = new Muxer(two)) {
            MuxException e = assertThrows(MuxException.class, () -> m.pushData(new byte[] {1}, 0L));
            assertEquals(MuxException.Kind.INVALID_USAGE, e.kind());
            assertEquals(2, m.dataHandles().size());
            m.pushDataTo(m.dataHandles().get(1), new byte[] {1, 2, 3}, 90_000L); // no throw
        }
    }

    @Test
    void oversizedPayloadIsInputMalformed() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264).addData(0x0100, 0xF0, true).build();
        try (Muxer m = new Muxer(cfg)) {
            MuxException e = assertThrows(MuxException.class,
                () -> m.pushData(new byte[70_000], 0L));
            assertEquals(MuxException.Kind.INPUT_MALFORMED, e.kind());
        }
    }

    @Test
    void forgedHandleIsInvalidUsage() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264).addData(0x0100, 0xF0, true).build();
        try (Muxer m = new Muxer(cfg)) {
            MuxException e = assertThrows(MuxException.class,
                () -> m.pushDataTo(DataStreamHandle.fromRaw(0x7FFF_FFFFL), new byte[] {1}, 0L));
            assertEquals(MuxException.Kind.INVALID_USAGE, e.kind());
        }
    }

    @Test
    void handleAccessorsAndClosedContract() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264).addData(0x0100, 0xF0, true).build();
        Muxer m = new Muxer(cfg);
        assertEquals(1, m.dataHandles().size());
        assertTrue(m.dataStreamHandle(0).isPresent());
        assertTrue(m.dataStreamHandle(1).isEmpty());
        m.close();
        assertThrows(IllegalStateException.class, m::dataHandles);
        assertThrows(IllegalStateException.class, () -> m.pushData(new byte[] {1}, 0L));
    }

    @Test
    void malformedDescriptorTlvRejectedAtOpen() {
        // length byte claims 10, only 2 payload bytes follow -> blob walk fails
        byte[] bad = {(byte) 0x05, 10, 'A', 'B'};
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0xF0, true)
            .streamDescriptorsForData(0, new byte[][] {bad})
            .build();
        MuxException e = assertThrows(MuxException.class, () -> new Muxer(cfg).close());
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
    }
}
