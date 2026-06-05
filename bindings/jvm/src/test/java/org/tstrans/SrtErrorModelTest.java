package org.tstrans;
import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class SrtErrorModelTest {
    @Test void kindHasEightConstants() {
        assertEquals(8, SrtException.Kind.values().length);
        assertNotNull(SrtException.Kind.valueOf("CONFIG_INVALID"));
        assertNotNull(SrtException.Kind.valueOf("BROKEN"));
        assertNotNull(SrtException.Kind.valueOf("WOULD_BLOCK"));
    }
    @Test void carriesKindAndMessage() {
        var e = new SrtException(SrtException.Kind.TIMEOUT, "boom");
        assertEquals(SrtException.Kind.TIMEOUT, e.kind());
        assertEquals("boom", e.getMessage());
    }
}
