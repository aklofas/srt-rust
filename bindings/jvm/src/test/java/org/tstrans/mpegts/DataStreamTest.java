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
