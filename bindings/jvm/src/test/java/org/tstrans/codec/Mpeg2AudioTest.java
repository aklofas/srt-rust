package org.tstrans.codec;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import org.junit.jupiter.api.Test;
import org.tstrans.CodecParseException;

class Mpeg2AudioTest {
    // tst-integration's synthetic_mp2_frame() produces a 20-byte buffer:
    //   FF FD C0 04  +  16 deterministic payload bytes (buf[i] = 0xB0 ^ i).
    // Header bits: sync=0xFFF, version_id=11 (MPEG-1), layer=10 (Layer II),
    // protection=1 (no CRC); bitrate_index=12 → 256 kbps (V1L2 table; the
    // generator's "384kbps" comment is mislabelled — the parser is source of
    // truth), sample_rate_index=00 → 44100 Hz, channel_mode=00 → stereo.
    //
    // DIVERGENCE FROM PLAN: the bare 20-byte buffer parses to
    // Truncated{needed:835} (Layer II V1 256k/44100 → 835-byte frame length),
    // NOT a complete frame. To exercise a successful parse we zero-pad the
    // synthetic header+payload out to the full 835-byte frame length the
    // header declares, so frames() yields one complete frame.
    private static final int MP2_FRAME_LEN = 835;
    private static final byte[] MP2_FRAME = buildMp2Frame();

    private static byte[] buildMp2Frame() {
        byte[] buf = new byte[MP2_FRAME_LEN];
        buf[0] = (byte) 0xFF;
        buf[1] = (byte) 0xFD;
        buf[2] = (byte) 0xC0;
        buf[3] = (byte) 0x04;
        for (int i = 4; i < 20; i++) {
            buf[i] = (byte) (0xB0 ^ i);
        }
        // bytes [20..835) remain zero (body padding)
        return buf;
    }

    private static byte[] unsigned(int... vals) {
        byte[] out = new byte[vals.length];
        for (int i = 0; i < vals.length; i++) {
            out[i] = (byte) vals[i];
        }
        return out;
    }

    @Test
    void parseMpeg2AudioFramesYieldsOneFrame() throws CodecParseException {
        List<Mpeg2AudioFrame> frames = Codec.parseMpeg2AudioFrames(MP2_FRAME);
        assertEquals(1, frames.size());
        Mpeg2AudioFrame f = frames.get(0);
        assertSame(Layer.II, f.layer());
        assertSame(Version.MPEG1, f.version());
        assertEquals(256L, f.bitrateKbps());
        assertEquals(44100L, f.sampleRateHz());
        assertSame(ChannelMode.STEREO, f.channelMode());
        assertEquals(2, f.channels());
        assertEquals(835L, f.frameLengthBytes());
        assertEquals(1152, f.samplesPerFrame());
        assertFalse(f.hasCrc());

        // rawHeader = fixed 4 bytes; payload = full frame (header + body).
        assertEquals(4, f.rawHeader().remaining());
        assertEquals(835, f.payload().remaining());
    }

    @Test
    void parseMpeg2AudioFramesBadSyncThrows() {
        // First bytes not a valid 11-bit sync → BAD_SYNC_WORD.
        byte[] garbage = unsigned(0x00, 0x00, 0x00, 0x00);
        CodecParseException ex = assertThrows(
                CodecParseException.class, () -> Codec.parseMpeg2AudioFrames(garbage));
        assertSame(CodecParseException.Kind.BAD_SYNC_WORD, ex.kind());
        assertEquals("mpeg2audio", ex.codec());
    }

    @Test
    void parseMpeg2AudioFramesWithResyncSkipsErrors() {
        // Resync is best-effort: it never throws and yields only frames that
        // parse completely. Pure garbage therefore yields an empty list.
        byte[] garbage = unsigned(0x00, 0x11, 0x22, 0x33);
        List<Mpeg2AudioFrame> frames = Codec.parseMpeg2AudioFramesWithResync(garbage);
        assertTrue(frames.isEmpty());

        // A valid frame is still recovered through the resync path.
        List<Mpeg2AudioFrame> good = Codec.parseMpeg2AudioFramesWithResync(MP2_FRAME);
        assertEquals(1, good.size());
        assertSame(Layer.II, good.get(0).layer());
        assertSame(Version.MPEG1, good.get(0).version());
    }
}
