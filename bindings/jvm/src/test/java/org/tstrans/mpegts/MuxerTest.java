package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;

import org.junit.jupiter.api.Test;
import org.tstrans.MuxException;

class MuxerTest {

    /**
     * Minimal H.264 IDR access unit in Annex-B framing: 4-byte start code +
     * IDR NAL header (0x65) + deterministic filler. Mirrors the Rust
     * {@code synthetic_h264_idr()} shape (enough to mux; not byte-locked here —
     * the byte-exact proof lives in MuxRoundtripScenarioTest).
     */
    private static byte[] syntheticH264Idr() {
        byte[] buf = new byte[4 + 1 + 15];
        buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01; // Annex-B start code
        buf[4] = 0x65;                                               // IDR NAL header
        for (int i = 0; i < 15; i++) {
            buf[5 + i] = (byte) (0xA5 ^ i);
        }
        return buf;
    }

    @Test
    void pushVideoThenDrainProducesTsPackets() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();

        int total = 0;
        boolean firstPacketSeen = false;
        byte[] out = new byte[8192];
        try (Muxer m = new Muxer(cfg)) {
            m.pushVideo(syntheticH264Idr(), /*pts=*/ 0L, /*keyFrame=*/ true);
            int n;
            while ((n = m.pull(out)) > 0) {
                assertEquals(0, n % 188, "pull must return a multiple of 188");
                if (!firstPacketSeen) {
                    assertEquals((byte) 0x47, out[0], "first TS packet must start with sync byte 0x47");
                    firstPacketSeen = true;
                }
                total += n;
            }
        }
        assertTrue(total > 0, "expected muxed TS bytes");
        assertEquals(0, total % 188, "total muxed bytes must be a multiple of 188");
    }

    @Test
    void pushAfterCloseThrows() throws Exception {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x1011, VideoCodec.H264)
            .build();
        Muxer m = new Muxer(cfg);
        m.close();
        assertThrows(IllegalStateException.class,
            () -> m.pushVideo(syntheticH264Idr(), 0L, true));
    }

    @Test
    void invalidConfigThrowsMuxException() {
        // pmt_pid colliding with the video PID is rejected by Muxer::new (Rust-side
        // validation) -> MuxException(CONFIG_INVALID). Proves the error path crosses JNI.
        MuxerConfig cfg = MuxerConfig.builder()
            .pmtPid(0x1011)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
        MuxException ex = assertThrows(MuxException.class, () -> new Muxer(cfg));
        assertEquals(MuxException.Kind.CONFIG_INVALID, ex.kind());
    }
}
