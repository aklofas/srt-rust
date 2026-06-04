package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.util.Set;
import org.junit.jupiter.api.Test;

class DemuxEventShapeTest {
    @Test
    void variantsAreSealedRecords() {
        DemuxEvent e = new DemuxEvent.Discontinuity(0x100);
        assertTrue(e instanceof DemuxEvent.Discontinuity);
        assertEquals(256, ((DemuxEvent.Discontinuity) e).pid());

        // The interface is sealed and permits exactly the keystone variants.
        assertTrue(DemuxEvent.class.isSealed(), "DemuxEvent must be sealed");
        assertEquals(
            Set.of(DemuxEvent.ProgramMap.class, DemuxEvent.Video.class, DemuxEvent.Audio.class,
                   DemuxEvent.Subtitle.class, DemuxEvent.UnknownSample.class, DemuxEvent.Discontinuity.class),
            Set.of(DemuxEvent.class.getPermittedSubclasses()),
            "permits set must match the keystone variants");

        // The variants are records.
        assertTrue(DemuxEvent.Video.class.isRecord());
        assertTrue(DemuxEvent.Audio.class.isRecord());
        assertTrue(DemuxEvent.Subtitle.class.isRecord());
        assertTrue(DemuxEvent.UnknownSample.class.isRecord());
        assertTrue(DemuxEvent.ProgramMap.class.isRecord());
        assertTrue(DemuxEvent.Discontinuity.class.isRecord());
    }
}
