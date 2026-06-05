package org.tstrans.codec;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertSame;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.Set;
import org.junit.jupiter.api.Test;

class SharedTypesTest {
    @Test
    void nalUnitH264FactoryRoundTrips() {
        byte[] bytes = {1, 2, 3, 4};
        NalUnit n = NalUnit.h264(5, 3, bytes);
        assertEquals("H264", n.kind());
        assertEquals(5, n.nalType());
        assertEquals(Integer.valueOf(3), n.refIdc());
        assertNull(n.layerId());
        assertNull(n.temporalIdPlus1());
        assertEquals(4, n.payload().remaining());
        assertTrue(n instanceof VideoUnit, "NalUnit must be a VideoUnit");
    }

    @Test
    void obuIsAVideoUnit() {
        Obu o = new Obu(6, new ObuExtension(2, 1), java.nio.ByteBuffer.wrap(new byte[] {9}));
        assertEquals(6, o.obuType());
        assertEquals(2, o.extension().temporalId());
        assertTrue(o instanceof VideoUnit, "Obu must be a VideoUnit");
    }

    @Test
    void videoUnitIsSealedOverNalUnitAndObu() {
        // No switch-on-sealed on JDK 17 — prove sealing via reflection.
        assertTrue(VideoUnit.class.isSealed(), "VideoUnit must be sealed");
        Set<Class<?>> permitted = Set.of(VideoUnit.class.getPermittedSubclasses());
        assertEquals(Set.of(NalUnit.class, Obu.class), permitted);
    }

    @Test
    void rationalAsFloat() {
        Rational r = new Rational(30000, 1001);
        assertEquals(30000.0 / 1001.0, r.asFloat(), 1e-9);
    }

    @Test
    void colorInfoExposesSubset() {
        ColorInfo c = new ColorInfo(
                ColourPrimaries.BT709, TransferCharacteristics.BT709,
                MatrixCoefficients.BT709, false);
        assertSame(ColourPrimaries.BT709, c.primaries());
        assertEquals(false, c.fullRange());
    }
}
