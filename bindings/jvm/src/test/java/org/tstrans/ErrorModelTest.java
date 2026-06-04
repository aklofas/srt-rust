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
        // Mirrors the raw tst-core::DemuxError producer variants (5). tst-py's
        // DemuxErrorKind also lists UNEXPECTED_EOF, which has no raw-feed
        // producer; it is reserved here for the io/file-helper wave.
        DemuxException.Kind[] ks = DemuxException.Kind.values();
        assertEquals(5, ks.length);
        // names asserted so a Rust-side rename is caught here.
        for (String n : new String[] {
                "SYNC_LOSS", "BAD_PMT", "BAD_PES", "STRICT_REJECTION", "INTERNAL"}) {
            DemuxException.Kind.valueOf(n); // throws if missing
        }
    }
}
