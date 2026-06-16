package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import org.junit.jupiter.api.Test;

/**
 * JVM surface tests for AV1 carriage provenance (AV1-01):
 * <ul>
 *   <li>{@link DemuxEvent.Video#av1Carriage()} is populated for AV1 and null for H.264.
 *   <li>{@link Muxer#pushVideoWire} performs a byte-faithful AV1 transmux fixpoint.
 * </ul>
 */
class Av1CarriageTest {

    // Mirrors crates/tst-core/tests/mpegts/av1_remux_fixpoint.rs synth():
    //   obu(2, []) + obu(1, [0x00,0x00,0x01,0xAA]) + obu(3, [0x00,0xFF])
    // OBU wire format: (type << 3) | 0x02, length, body...
    private static byte[] synthAv1Au() {
        return unsigned(
            0x12, 0x00,                          // TD OBU (type 2), empty body
            0x0A, 0x04, 0x00, 0x00, 0x01, 0xAA, // SEQUENCE_HEADER (type 1), 4-byte body
            0x1A, 0x02, 0x00, 0xFF               // FRAME (type 3), 2-byte body
        );
    }

    /** Synthetic Annex-B H.264 IDR (mirrors MuxRoundtripScenarioTest). */
    private static byte[] synthH264Idr() {
        byte[] buf = new byte[20];
        buf[0] = 0x00; buf[1] = 0x00; buf[2] = 0x00; buf[3] = 0x01;
        buf[4] = 0x65; // IDR
        for (int i = 0; i < 15; i++) {
            buf[5 + i] = (byte) (0xA5 ^ i);
        }
        return buf;
    }

    private static byte[] unsigned(int... vals) {
        byte[] out = new byte[vals.length];
        for (int i = 0; i < vals.length; i++) {
            out[i] = (byte) vals[i];
        }
        return out;
    }

    /** Mux one video AU and drain all TS bytes. */
    private static byte[] muxAndDrain(MuxerConfig cfg, byte[] au, long pts) throws Exception {
        ByteArrayOutputStream acc = new ByteArrayOutputStream();
        byte[] out = new byte[8192];
        try (Muxer m = new Muxer(cfg)) {
            m.pushVideo(au, pts, true);
            int n;
            while ((n = m.pull(out)) > 0) {
                acc.write(out, 0, n);
            }
        }
        return acc.toByteArray();
    }

    /** Demux TS bytes and return the first Video event, or null. */
    private static DemuxEvent.Video firstVideo(byte[] ts, Av1CarriageMode mode) throws Exception {
        DemuxerConfig cfg = new DemuxerConfig.Builder().av1Carriage(mode).build();
        try (Demuxer d = new Demuxer(cfg)) {
            d.feed(ts);
            d.flush();
            for (DemuxEvent e : d) {
                if (e instanceof DemuxEvent.Video v) {
                    return v;
                }
            }
        }
        return null;
    }

    @Test
    void av1BindingModeCarriagePopulatedOnVideoEvent() throws Exception {
        // Mux a synthetic AV1 AU in MPEG2_TS_BINDING mode (default).
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.AV1)
            .build();
        byte[] ts = muxAndDrain(cfg, synthAv1Au(), 90_000L);

        DemuxEvent.Video video = firstVideo(ts, Av1CarriageMode.MPEG2_TS_BINDING);
        assertNotNull(video, "expected a Video event from AV1 mux output");
        // The provenance field must reflect the configured carriage mode.
        assertEquals(Av1CarriageMode.MPEG2_TS_BINDING, video.av1Carriage(),
            "AV1 binding-mode Video event must carry MPEG2_TS_BINDING carriage");
    }

    @Test
    void h264VideoEventHasNullCarriage() throws Exception {
        // H.264 samples carry no AV1 carriage — av1Carriage must be null.
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
        byte[] ts = muxAndDrain(cfg, synthH264Idr(), 0L);

        DemuxEvent.Video video = firstVideo(ts, Av1CarriageMode.MPEG2_TS_BINDING);
        assertNotNull(video, "expected a Video event from H.264 mux output");
        assertNull(video.av1Carriage(),
            "H.264 Video event must have null av1Carriage (non-AV1 codec)");
    }

    @Test
    void pushVideoWireIsAv1BindingModeFixpoint() throws Exception {
        // Mux → demux → pushVideoWire remux → re-demux.
        // The re-demuxed raw payload must be byte-equal to the first-demux raw
        // (i.e. the wire push does NOT re-wrap the binding-mode payload).
        MuxerConfig muxCfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.AV1)
            .build();

        // First generation: elementary OBUs → mux → demux.
        byte[] ts1 = muxAndDrain(muxCfg, synthAv1Au(), 90_000L);
        DemuxEvent.Video v1 = firstVideo(ts1, Av1CarriageMode.MPEG2_TS_BINDING);
        assertNotNull(v1, "expected a Video event after first mux");

        ByteBuffer rawBuf = v1.raw();
        assertNotNull(rawBuf, "Video.raw() must be populated for AV1");
        assertFalse(rawBuf.isDirect(), "raw is a JVM-owned heap copy");
        byte[] raw1 = new byte[rawBuf.remaining()];
        rawBuf.duplicate().get(raw1);
        assertTrue(raw1.length > 0, "raw payload must be non-empty (guard against vacuous fixpoint)");

        // Re-mux via pushVideoWire (pass-through, no re-wrap).
        ByteArrayOutputStream acc2 = new ByteArrayOutputStream();
        byte[] pullBuf = new byte[8192];
        try (Muxer m2 = new Muxer(muxCfg)) {
            m2.pushVideoWire(raw1, 90_000L, true);
            int n;
            while ((n = m2.pull(pullBuf)) > 0) {
                acc2.write(pullBuf, 0, n);
            }
        }
        byte[] ts2 = acc2.toByteArray();

        // Re-demux the re-muxed stream.
        DemuxEvent.Video v2 = firstVideo(ts2, Av1CarriageMode.MPEG2_TS_BINDING);
        assertNotNull(v2, "expected a Video event after re-mux");

        byte[] raw2 = new byte[v2.raw().remaining()];
        v2.raw().duplicate().get(raw2);

        // Payload fixpoint: pushVideoWire must not alter the wire bytes.
        assertArrayEquals(raw1, raw2,
            "AV1 binding-mode remux via pushVideoWire must be a payload fixpoint");
    }

    @Test
    void pushVideoWireAmbiguousTargetThrows() throws Exception {
        // Zero video streams → pushVideoWire must throw MuxException (INVALID_USAGE).
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addAudio(0x1012, AudioCodec.MP2)
            .build();
        try (Muxer m = new Muxer(cfg)) {
            org.tstrans.MuxException ex = assertThrows(
                org.tstrans.MuxException.class,
                () -> m.pushVideoWire(new byte[] {0x00}, 0L, false));
            assertEquals(org.tstrans.MuxException.Kind.INVALID_USAGE, ex.kind());
        }
    }
}
