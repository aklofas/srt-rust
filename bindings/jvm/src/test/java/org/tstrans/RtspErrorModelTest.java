package org.tstrans;
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class RtspErrorModelTest {
    @Test void kindHasTenConstants() {
        assertEquals(10, RtspException.Kind.values().length);
        assertNotNull(RtspException.Kind.valueOf("PROTOCOL"));
        assertNotNull(RtspException.Kind.valueOf("AUTH_REQUIRED"));
        assertNotNull(RtspException.Kind.valueOf("UNSUPPORTED_TRANSPORT"));
        assertNotNull(RtspException.Kind.valueOf("MOUNT"));
    }
    @Test void carriesKindAndMessage() {
        var e = new RtspException(RtspException.Kind.TLS, "boom");
        assertEquals(RtspException.Kind.TLS, e.kind());
        assertEquals("boom", e.getMessage());
    }
}
