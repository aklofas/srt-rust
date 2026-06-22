package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.util.List;
import java.util.Optional;
import org.junit.jupiter.api.Test;
import org.tstrans.MuxException;

class MuxerHandlesTest {
    @Test
    void handleListsMatchConfiguredStreamCounts() throws MuxException {
        MuxerConfig cfg = MuxerConfig.builder()
            .addVideo(0x100, VideoCodec.H264)
            .addVideo(0x101, VideoCodec.H264)
            .addAudio(0x110, AudioCodec.AAC)
            .addKlv(0x120, KlvStreamType.SYNCHRONOUS_METADATA, true)
            .build();
        try (Muxer m = new Muxer(cfg)) {
            List<VideoStreamHandle> vids = m.videoHandles();
            assertEquals(2, vids.size());
            assertEquals(1, m.audioHandles().size());
            assertEquals(1, m.klvHandles().size());
            assertEquals(0, m.subtitleHandles().size());

            Optional<VideoStreamHandle> second = m.videoStreamHandle(1);
            assertTrue(second.isPresent());
            assertEquals(vids.get(1), second.get());
            assertTrue(m.videoStreamHandle(5).isEmpty());
        }
    }
}
