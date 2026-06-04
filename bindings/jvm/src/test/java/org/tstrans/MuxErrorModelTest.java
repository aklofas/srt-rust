package org.tstrans;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class MuxErrorModelTest {
    @Test
    void muxExceptionCarriesKindAndMessage() {
        MuxException e = new MuxException(MuxException.Kind.CONFIG_INVALID, "bad config");
        assertTrue(e instanceof BindingException);
        assertEquals(MuxException.Kind.CONFIG_INVALID, e.kind());
        assertEquals("bad config", e.getMessage());
    }
    @Test
    void kindMatchesTstPyBuckets() {
        for (String n : new String[]{
                "INPUT_MALFORMED","CONFIG_INVALID","INVALID_USAGE","BACKPRESSURE","INTERNAL"}) {
            MuxException.Kind.valueOf(n);
        }
        assertEquals(5, MuxException.Kind.values().length);
    }
}
