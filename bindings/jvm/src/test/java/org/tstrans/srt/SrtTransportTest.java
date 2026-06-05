package org.tstrans.srt;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;
import org.tstrans.SrtException;

class SrtTransportTest {

    /**
     * A listener-mode URL supplied to {@code Sender.fromUrl} must throw
     * {@code CONFIG_INVALID} — the Sender checks that the parsed URL uses
     * {@code mode=caller} before attempting a connection.
     */
    @Test
    void senderRejectsListenerModeUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> Sender.fromUrl("srt://127.0.0.1:9000?mode=listener")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * A caller-mode URL supplied to {@code Receiver.fromUrl} must throw
     * {@code CONFIG_INVALID} — the Receiver checks that the parsed URL uses
     * {@code mode=listener} before attempting a bind+accept.
     */
    @Test
    void receiverRejectsCallerModeUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> Receiver.fromUrl("srt://127.0.0.1:9000?mode=caller")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }

    /**
     * A clearly-malformed URL (no scheme, no port) should fail
     * {@code SrtUrl::parse} and surface as {@code CONFIG_INVALID}.
     */
    @Test
    void senderRejectsMalformedUrl() {
        var e = assertThrows(
            SrtException.class,
            () -> Sender.fromUrl("not-a-url")
        );
        assertEquals(SrtException.Kind.CONFIG_INVALID, e.kind());
    }
}
