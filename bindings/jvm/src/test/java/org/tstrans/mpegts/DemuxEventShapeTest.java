package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.util.Set;
import org.junit.jupiter.api.Test;

class DemuxEventShapeTest {
    @Test
    void variantsAreSealedRecords() {
        StreamId sid = new StreamId(0x100, new StreamKind.Unknown(0), 1);
        DemuxEvent e = new DemuxEvent.Discontinuity(sid, DiscontinuityKind.CONTINUITY_JUMP);
        assertTrue(e instanceof DemuxEvent.Discontinuity d
            && d.stream().pid() == 0x100 && d.kind() == DiscontinuityKind.CONTINUITY_JUMP);

        // The interface is sealed and permits exactly the full event set.
        assertTrue(DemuxEvent.class.isSealed(), "DemuxEvent must be sealed");
        assertEquals(
            Set.of(DemuxEvent.ProgramMap.class, DemuxEvent.Video.class, DemuxEvent.Audio.class,
                   DemuxEvent.Subtitle.class, DemuxEvent.UnknownSample.class, DemuxEvent.Metadata.class,
                   DemuxEvent.NonConformant.class, DemuxEvent.Discontinuity.class,
                   DemuxEvent.ReconnectDiscontinuity.class),
            Set.of(DemuxEvent.class.getPermittedSubclasses()),
            "permits set must match the full event set");

        // The variants are records.
        assertTrue(DemuxEvent.Video.class.isRecord());
        assertTrue(DemuxEvent.Audio.class.isRecord());
        assertTrue(DemuxEvent.Subtitle.class.isRecord());
        assertTrue(DemuxEvent.UnknownSample.class.isRecord());
        assertTrue(DemuxEvent.Metadata.class.isRecord());
        assertTrue(DemuxEvent.ProgramMap.class.isRecord());
        assertTrue(DemuxEvent.Discontinuity.class.isRecord());
        assertTrue(DemuxEvent.ReconnectDiscontinuity.class.isRecord());
    }
}
