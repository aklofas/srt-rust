package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.util.Set;
import org.junit.jupiter.api.Test;

class StreamShapeTest {
    @Test
    void streamKindIsSealedWithCodecVariants() {
        StreamId id = new StreamId(0x100, new StreamKind.Video(VideoCodec.H264), 1);
        assertEquals(0x100, id.pid());
        assertEquals(1, id.programNumber());
        assertTrue(id.kind() instanceof StreamKind.Video v && v.codec() == VideoCodec.H264);
        assertTrue(StreamKind.class.isSealed());
        assertEquals(
            Set.of(StreamKind.Video.class, StreamKind.Audio.class, StreamKind.Subtitle.class,
                   StreamKind.KlvSync.class, StreamKind.KlvAsync.class, StreamKind.Unknown.class),
            Set.of(StreamKind.class.getPermittedSubclasses()));
    }
}
