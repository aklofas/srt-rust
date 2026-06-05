package org.tstrans;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import org.junit.jupiter.api.Test;

class ErrorModelTest {
    @Test
    void demuxExceptionCarriesKindAndMessage() {
        DemuxException e = new DemuxException(DemuxException.Kind.BAD_PMT, "bad pmt");
        assertTrue(e instanceof BindingException, "DemuxException must extend BindingException");
        assertEquals(DemuxException.Kind.BAD_PMT, e.kind());
        assertEquals("bad pmt", e.getMessage());
    }

    @Test
    void kindHasAllRustVariants() {
        // 5 constants map raw tst-core::DemuxError producer variants; UNEXPECTED_EOF
        // is a 6th, parity-only constant: tst-py's DemuxErrorKind carries the same
        // dead entry (no producer in tst_core::DemuxError — the file path treats
        // truncation as clean EOF and surfaces read failures as native IOException).
        DemuxException.Kind[] ks = DemuxException.Kind.values();
        assertEquals(6, ks.length);
        // names asserted so a Rust-side rename is caught here.
        for (String n : new String[] {
                "SYNC_LOSS", "BAD_PMT", "BAD_PES", "UNEXPECTED_EOF",
                "STRICT_REJECTION", "INTERNAL"}) {
            DemuxException.Kind.valueOf(n); // throws if missing
        }
    }
}
