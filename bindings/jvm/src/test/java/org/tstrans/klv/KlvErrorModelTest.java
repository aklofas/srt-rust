package org.tstrans.klv;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;
import org.tstrans.KlvDecodeException;
import org.tstrans.KlvEncodeException;

/** Verifies that every {@code KlvDecodeException.Kind} and {@code KlvEncodeException.Kind}
 *  constant round-trips through the JNI forced-throw path with the correct kind set. */
class KlvErrorModelTest {

    @Test
    void decodeKindsRoundTrip() {
        for (KlvDecodeException.Kind k : KlvDecodeException.Kind.values()) {
            KlvDecodeException ex = assertThrows(
                    KlvDecodeException.class,
                    () -> Klv.raiseDecodeForTest(k.name()));
            assertEquals(k, ex.kind(), "expected Kind." + k.name());
        }
    }

    @Test
    void encodeKindsRoundTrip() {
        for (KlvEncodeException.Kind k : KlvEncodeException.Kind.values()) {
            KlvEncodeException ex = assertThrows(
                    KlvEncodeException.class,
                    () -> Klv.raiseEncodeForTest(k.name()));
            assertEquals(k, ex.kind(), "expected Kind." + k.name());
        }
    }
}
