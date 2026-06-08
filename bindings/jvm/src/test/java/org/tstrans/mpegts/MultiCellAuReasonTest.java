package org.tstrans.mpegts;

import static org.junit.jupiter.api.Assertions.*;
import java.util.Set;
import java.util.stream.Collectors;
import java.util.stream.Stream;
import org.junit.jupiter.api.Test;

class MultiCellAuReasonTest {
    @Test
    void enumMirrorsRustVariants() {
        // Full constant set mirrors tst_core::mpegts::demux::event::MultiCellAuReason.
        // The last two (OVERFLOW_TOTAL, TOO_MANY_PIDS) are the aggregate-byte-cap and
        // too-many-in-flight-PID rejections; before they were mirrored, the native
        // bridge mapped them to the misleading ORPHAN default.
        Set<String> names = Stream.of(MultiCellAuReason.values())
            .map(Enum::name)
            .collect(Collectors.toSet());
        assertEquals(
            Set.of("ORPHAN", "SEQUENCE_GAP", "CONCURRENT_FIRST", "OVERFLOW",
                   "OVERFLOW_TOTAL", "TOO_MANY_PIDS"),
            names);

        // valueOf round-trips the two new constants.
        assertEquals(MultiCellAuReason.OVERFLOW_TOTAL, MultiCellAuReason.valueOf("OVERFLOW_TOTAL"));
        assertEquals(MultiCellAuReason.TOO_MANY_PIDS, MultiCellAuReason.valueOf("TOO_MANY_PIDS"));
    }
}
