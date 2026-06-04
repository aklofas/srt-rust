package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class MuxerConfigTest {
    @Test
    void builderAccumulatesStreamsAndScalars() {
        MuxerConfig cfg = MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000).pcrPid(0x1011)
            .addVideo(0x1011, VideoCodec.H264)
            .addKlv(0x1031, KlvStreamType.PRIVATE_DATA, /*carriesPts=*/ true)
            .pcrIntervalMs(40).psiIntervalMs(100).bufferPackets(10_000)
            .av1Carriage(Av1CarriageMode.MPEG2_TS_BINDING)
            .build();
        assertEquals(1, cfg.programNumber());
        assertEquals(2, cfg.streamCount());
    }

    @Test
    void emptyConfigRejected() {
        assertThrows(IllegalArgumentException.class, () -> MuxerConfig.builder().build());
    }

    @Test
    void dvbSubtitleDeferred() {
        assertThrows(IllegalArgumentException.class,
            () -> MuxerConfig.builder().addSubtitle(0x1041, SubtitleCodec.DVB_SUBTITLING));
    }

    @Test
    void outOfRangePidRejected() {
        assertThrows(IllegalArgumentException.class,
            () -> MuxerConfig.builder().addVideo(0x0000, VideoCodec.H264));
    }
}
