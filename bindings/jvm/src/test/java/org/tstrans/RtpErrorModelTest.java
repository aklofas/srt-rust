package org.tstrans;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class RtpErrorModelTest {
    @Test
    void rtpExceptionCarriesKindAndMessage() {
        RtpException e = new RtpException(RtpException.Kind.TRANSPORT, "wire broke");
        assertTrue(e instanceof BindingException);
        assertEquals(RtpException.Kind.TRANSPORT, e.kind());
        assertEquals("wire broke", e.getMessage());
    }
    @Test
    void kindMatchesTstPyBuckets() {
        for (String n : new String[]{
                "TRANSPORT","MALFORMED_PACKET","CANCELLED","TIMEOUT"}) {
            RtpException.Kind.valueOf(n);
        }
        assertEquals(4, RtpException.Kind.values().length);
    }
}
