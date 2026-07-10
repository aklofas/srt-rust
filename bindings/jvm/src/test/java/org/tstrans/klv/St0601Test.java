package org.tstrans.klv;

import static org.junit.jupiter.api.Assertions.*;

import java.util.HexFormat;
import java.util.Optional;
import org.junit.jupiter.api.Test;
import org.tstrans.KlvDecodeException;
import org.tstrans.KlvEncodeException;

/**
 * Tests for {@link Klv#decodeUasDatalink} / {@link Klv#encodeUasDatalink} /
 * {@link Klv#encodeUasDatalinkStrictCompliance} and the composite accessor methods
 * on {@link UasDatalinkLs}.
 *
 * <p>Fixture bytes are the committed {@code crates/tst-core/tests/fixtures/st0601/}
 * synthetic KLV records. Expected field values are pinned from
 * {@code tst_core::klv::st0601::decode} of the same bytes.
 */
class St0601Test {

    /**
     * synthetic_full.klv — ~41 tags populated; used for composite accessor tests
     * and decode→encode→decode round-trip.
     *
     * <p>Expected values (pinned from {@code tst_core::klv::st0601::decode}):
     * <ul>
     *   <li>timestampUs = 1700123456789000</li>
     *   <li>sensorLatDeg ≈ 38.1234560199</li>
     *   <li>sensorLonDeg ≈ -121.6543210026</li>
     *   <li>sensorAltM ≈ 2500.0198</li>
     *   <li>frameCenterLatDeg ≈ 38.0000000158</li>
     *   <li>frameCenterLonDeg ≈ -121.5000000230</li>
     *   <li>frameCenterElevM ≈ 0.0320439</li>
     *   <li>platformHeadingDeg ≈ 123.4498</li>
     *   <li>sensorHfovDeg ≈ 45.0007</li>
     *   <li>sensorVfovDeg ≈ 30.0014</li>
     * </ul>
     */
    private static final byte[] FULL_FIXTURE = HexFormat.of().parseHex(
            "060e2b34020b01010e0103010100000081f4020800060a40d6b75a0803054d2d30303104064e31323334"
                    + "35050257c90602e0000702199908012d0901280a0744524f4e452d410b07454f2d4e4f53450c065747"
                    + "532d38340d04363853a50e04a97d81b90f022bbd1002400011022aab1204800000001304e000000014"
                    + "0400000000150400275254160203d71704360b60b61804a999999a19020b941a0211111b0211111c02"
                    + "11111d02eeef1e02eeef1f02eeef2002eeef210211112f0109300301020332024fff3b064543484f2d"
                    + "314101135204360bbdeb5304a999c8355404360bbdeb5504a9996b005604360b03815704a9996b0058"
                    + "04360b03815904a999c835010239c5");

    /**
     * synthetic_minimal.klv — Tag 02 (timestamp) + Tag 65 (version) + Tag 01
     * (checksum). Crafted to satisfy strict-compliance ordering.
     */
    private static final byte[] MINIMAL_FIXTURE = HexFormat.of().parseHex(
            "060e2b34020b01010e0103010100000011020800060a24181e40004101130102aa0a");

    // -----------------------------------------------------------------------
    // Basic decode tests
    // -----------------------------------------------------------------------

    @Test
    void decodeFullFixtureReturnsUasDatalinkLs() {
        UasDatalinkLs rec = assertDoesNotThrow(() -> Klv.decodeUasDatalink(FULL_FIXTURE));
        assertNotNull(rec);
    }

    @Test
    void decodeFullFixturePopulatesTimestamp() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        assertNotNull(rec.timestampUs());
        assertEquals(1700123456789000L, rec.timestampUs());
    }

    @Test
    void decodeFullFixturePopulatesIdentityFields() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        assertEquals("M-001", rec.missionId());
        assertEquals("EO-NOSE", rec.imageSourceSensor());
        assertEquals("N12345", rec.platformTailNumber());
        assertEquals("DRONE-A", rec.platformDesignation());
        assertEquals("WGS-84", rec.imageCoordinateSystem());
        assertEquals(19, rec.uasLsVersion());
    }

    @Test
    void decodeFullFixtureNoFieldErrors() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        assertTrue(rec.fieldErrors().isEmpty(), "expected no field errors on well-formed input");
    }

    @Test
    void decodeRejectsTruncated() {
        // 4-byte buffer — too short to contain a 16-byte UL
        byte[] truncated = HexFormat.of().parseHex("060e2b34");
        KlvDecodeException ex = assertThrows(KlvDecodeException.class,
                () -> Klv.decodeUasDatalink(truncated));
        assertEquals(KlvDecodeException.Kind.TRUNCATED_SET, ex.kind());
    }

    @Test
    void decodeMinimalFixtureSucceeds() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(MINIMAL_FIXTURE);
        assertNotNull(rec.timestampUs());
        assertNotNull(rec.uasLsVersion());
    }

    // -----------------------------------------------------------------------
    // Composite accessor tests
    // -----------------------------------------------------------------------

    @Test
    void sensorPositionPopulated() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<GeoPoint> pos = rec.sensorPosition();
        assertTrue(pos.isPresent(), "expected sensorPosition to be present");
        GeoPoint gp = pos.get();
        assertEquals(38.1234560199, gp.latDeg(), 1e-6);
        assertEquals(-121.6543210026, gp.lonDeg(), 1e-6);
        assertEquals(2500.019, gp.altM(), 0.01);
    }

    @Test
    void frameCenterPopulated() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<GeoPoint> fc = rec.frameCenter();
        assertTrue(fc.isPresent(), "expected frameCenter to be present");
        GeoPoint gp = fc.get();
        assertEquals(38.000000015, gp.latDeg(), 1e-6);
        assertEquals(-121.500000023, gp.lonDeg(), 1e-6);
        assertEquals(0.032, gp.altM(), 0.01);
    }

    @Test
    void sensorAttitudePopulated() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<Attitude> att = rec.sensorAttitude();
        assertTrue(att.isPresent(), "expected sensorAttitude to be present");
        Attitude a = att.get();
        assertEquals(180.0, a.headingDeg(), 0.01);
        assertEquals(-45.0, a.pitchDeg(), 0.01);
        assertEquals(0.0, a.rollDeg(), 0.01);
    }

    @Test
    void platformAttitudePopulated() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<Attitude> pa = rec.platformAttitude();
        assertTrue(pa.isPresent(), "expected platformAttitude to be present");
        Attitude a = pa.get();
        assertEquals(123.45, a.headingDeg(), 0.01);
        assertEquals(-5.0, a.pitchDeg(), 0.01);
        assertEquals(10.0, a.rollDeg(), 0.01);
    }

    @Test
    void sensorFovPopulated() throws KlvDecodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<FieldOfView> fov = rec.sensorFov();
        assertTrue(fov.isPresent(), "expected sensorFov to be present");
        assertEquals(45.0, fov.get().horizontalDeg(), 0.01);
        assertEquals(30.0, fov.get().verticalDeg(), 0.01);
    }

    @Test
    void cornersAbsolutePreferred() throws KlvDecodeException {
        // The full fixture has absolute corner tags (82–89) populated.
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE);
        Optional<Corners> c = rec.corners();
        assertTrue(c.isPresent(), "expected corners to be present");
        Corners corners = c.get();
        // p1 should be the absolute cornerLatP1Deg / cornerLonP1Deg value from the fixture
        assertEquals(38.001, corners.p1().latDeg(), 0.001);
        assertEquals(-121.499, corners.p1().lonDeg(), 0.001);
    }

    @Test
    void cornersEmptyWhenFieldsAbsent() {
        // Build a minimal record with only frame center — no corners
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .frameCenterLatDeg(38.0)
                .frameCenterLonDeg(-121.5)
                .frameCenterElevM(100.0)
                .build();
        assertTrue(rec.corners().isEmpty(), "expected empty corners when only frame center set");
    }

    @Test
    void cornersOffsetFallback() {
        // Record has frame center + offset fields but NOT absolute corner fields
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .frameCenterLatDeg(38.0)
                .frameCenterLonDeg(-121.5)
                .frameCenterElevM(100.0)
                .cornerLatOffsetP1Deg(0.001)
                .cornerLonOffsetP1Deg(-0.001)
                .cornerLatOffsetP2Deg(0.001)
                .cornerLonOffsetP2Deg(0.001)
                .cornerLatOffsetP3Deg(-0.001)
                .cornerLonOffsetP3Deg(0.001)
                .cornerLatOffsetP4Deg(-0.001)
                .cornerLonOffsetP4Deg(-0.001)
                .build();
        Optional<Corners> c = rec.corners();
        assertTrue(c.isPresent(), "expected corners via offset fallback");
        Corners corners = c.get();
        assertEquals(38.001, corners.p1().latDeg(), 1e-9);
        assertEquals(-121.501, corners.p1().lonDeg(), 1e-9);
    }

    @Test
    void sensorPositionEmptyWhenFieldsAbsent() {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .sensorLatDeg(38.0)
                // no lon or alt → should be empty
                .build();
        assertTrue(rec.sensorPosition().isEmpty());
    }

    // -----------------------------------------------------------------------
    // Decode strict / compliance paths
    // -----------------------------------------------------------------------

    @Test
    void complianceModeAcceptsMinimalFixture() throws KlvDecodeException {
        // Minimal fixture was crafted to satisfy compliance (Tag 2 first / Tag 65 / Tag 1 last)
        UasDatalinkLs rec = Klv.decodeUasDatalink(MINIMAL_FIXTURE, false, true);
        assertNotNull(rec.timestampUs());
        assertNotNull(rec.uasLsVersion());
    }

    @Test
    void strictModeAcceptsFullFixture() throws KlvDecodeException {
        // Full fixture has a valid ST 0601 family UL
        UasDatalinkLs rec = Klv.decodeUasDatalink(FULL_FIXTURE, true, false);
        assertNotNull(rec.timestampUs());
    }

    @Test
    void strictModeRejectsWrongUl() {
        // Take the valid full fixture and overwrite the leading 16-byte UL with
        // the VMTI LS UL (not an ST 0601 family UL). Strict mode requires the
        // ST 0601 family UL → decode_strict returns UnexpectedUniversalLabel →
        // mapped to BAD_UNIVERSAL_LABEL.
        byte[] buf = FULL_FIXTURE.clone();
        byte[] vmtiUl = HexFormat.of().parseHex("060e2b34020b01010e01030306000000");
        System.arraycopy(vmtiUl, 0, buf, 0, 16);
        KlvDecodeException ex = assertThrows(KlvDecodeException.class,
                () -> Klv.decodeUasDatalink(buf, true, false));
        assertEquals(KlvDecodeException.Kind.BAD_UNIVERSAL_LABEL, ex.kind());
    }

    // -----------------------------------------------------------------------
    // Encode tests
    // -----------------------------------------------------------------------

    @Test
    void encodeRoundTripManyFields() throws KlvDecodeException, KlvEncodeException {
        // decode → encode → decode, assert many fields survive
        UasDatalinkLs original = Klv.decodeUasDatalink(FULL_FIXTURE);
        byte[] wire = Klv.encodeUasDatalink(original);
        UasDatalinkLs decoded = Klv.decodeUasDatalink(wire);

        // Timestamp
        assertEquals(original.timestampUs(), decoded.timestampUs());
        // Identity strings
        assertEquals(original.missionId(), decoded.missionId());
        assertEquals(original.imageSourceSensor(), decoded.imageSourceSensor());
        assertEquals(original.platformTailNumber(), decoded.platformTailNumber());
        assertEquals(original.platformDesignation(), decoded.platformDesignation());
        assertEquals(original.imageCoordinateSystem(), decoded.imageCoordinateSystem());
        // Platform state
        assertEquals(original.platformHeadingDeg(), decoded.platformHeadingDeg(), 1e-9);
        assertEquals(original.platformPitchDeg(), decoded.platformPitchDeg(), 1e-9);
        assertEquals(original.platformRollDeg(), decoded.platformRollDeg(), 1e-9);
        // Sensor position
        assertEquals(original.sensorLatDeg(), decoded.sensorLatDeg(), 1e-9);
        assertEquals(original.sensorLonDeg(), decoded.sensorLonDeg(), 1e-9);
        assertEquals(original.sensorAltM(), decoded.sensorAltM(), 1e-9);
        // Sensor FOV
        assertEquals(original.sensorHfovDeg(), decoded.sensorHfovDeg(), 1e-9);
        assertEquals(original.sensorVfovDeg(), decoded.sensorVfovDeg(), 1e-9);
        // Frame center
        assertEquals(original.frameCenterLatDeg(), decoded.frameCenterLatDeg(), 1e-9);
        assertEquals(original.frameCenterLonDeg(), decoded.frameCenterLonDeg(), 1e-9);
        assertEquals(original.frameCenterElevM(), decoded.frameCenterElevM(), 1e-9);
        // Slant range
        assertEquals(original.slantRangeM(), decoded.slantRangeM(), 1e-9);
        // Absolute corners
        assertEquals(original.cornerLatP1Deg(), decoded.cornerLatP1Deg(), 1e-9);
        assertEquals(original.cornerLonP1Deg(), decoded.cornerLonP1Deg(), 1e-9);
        // UAS LS version
        assertEquals(original.uasLsVersion(), decoded.uasLsVersion());
        // Composite views survive
        assertEquals(original.sensorPosition(), decoded.sensorPosition());
        assertEquals(original.frameCenter(), decoded.frameCenter());
        assertEquals(original.sensorFov(), decoded.sensorFov());
    }

    @Test
    void encodeStrictComplianceMissingMandatoryTagThrows() {
        // Default record has no timestamp (Tag 2) — strict compliance requires it.
        // The Rust KlvEncodeError::MissingMandatoryItem { tag: 2, .. } is thrown with tag=2.
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder().universalLabel(ul).build();
        KlvEncodeException ex = assertThrows(KlvEncodeException.class,
                () -> Klv.encodeUasDatalinkStrictCompliance(rec));
        assertEquals(KlvEncodeException.Kind.MISSING_MANDATORY_ITEM, ex.kind());
        // tag-bearing: MISSING_MANDATORY_ITEM carries the offending tag (Tag 2 = timestamp)
        assertTrue(ex.tag().isPresent(), "MISSING_MANDATORY_ITEM must carry a tag");
        assertEquals(2L, ex.tag().get().longValue());
    }

    @Test
    void encodeDeclaredVersionOutOfRangeThrowsIllegalArgument() {
        // declaredVersion is stored as Java int, so values > 255 fit in the field
        // but must be rejected on encode (Rust field is u8).
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .declaredVersion(300) // out of u8 range
                .build();
        assertThrows(IllegalArgumentException.class, () -> Klv.encodeUasDatalink(rec));
    }

    @Test
    void encodeSlicedSecurityLocalSetRoundTrips() throws KlvDecodeException, KlvEncodeException {
        // Fix 3: encode a record with a sliced (position/limit-constrained) ByteBuffer
        // as securityLocalSet; the encode path must honour the slice, not read the full
        // backing array.
        //
        // Use a minimal ST 0102 body: Tag 1 = UNCLASSIFIED (0x01, 0x01, 0x01).
        byte[] padding = {(byte) 0xDE, (byte) 0xAD};
        byte[] klvBody = {0x01, 0x01, 0x01};
        byte[] backing = new byte[padding.length + klvBody.length + padding.length];
        System.arraycopy(padding, 0, backing, 0, padding.length);
        System.arraycopy(klvBody, 0, backing, padding.length, klvBody.length);
        System.arraycopy(padding, 0, backing, padding.length + klvBody.length, padding.length);

        // Slice to expose only the 3 klvBody bytes (skip leading + trailing padding).
        java.nio.ByteBuffer sliced = java.nio.ByteBuffer.wrap(
                backing, padding.length, klvBody.length).slice();
        assertEquals(3, sliced.remaining(), "slice should have exactly 3 remaining bytes");

        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .securityLocalSet(sliced)
                .build();

        byte[] wire = Klv.encodeUasDatalink(rec);
        UasDatalinkLs decoded = Klv.decodeUasDatalink(wire);

        assertNotNull(decoded.securityLocalSet(), "securityLocalSet should survive encode/decode");
        // The decoded securityLocalSet must equal exactly the 3-byte klvBody window.
        byte[] secBytes = new byte[decoded.securityLocalSet().remaining()];
        decoded.securityLocalSet().duplicate().get(secBytes);
        assertArrayEquals(klvBody, secBytes,
                "encoded securityLocalSet must match the sliced window, not the full backing array");
    }

    @Test
    void universalLabelWrongSizeThrowsAtConstruction() {
        assertThrows(IllegalArgumentException.class, () -> {
            java.nio.ByteBuffer ul3 = java.nio.ByteBuffer.wrap(new byte[]{0x06, 0x0e, 0x2b});
            new UasDatalinkLs.Builder().universalLabel(ul3).build();
        });
    }

    @Test
    void encodeMinimalFixtureRoundTrip() throws KlvDecodeException, KlvEncodeException {
        UasDatalinkLs rec = Klv.decodeUasDatalink(MINIMAL_FIXTURE);
        byte[] wire = Klv.encodeUasDatalink(rec);
        UasDatalinkLs decoded = Klv.decodeUasDatalink(wire);
        assertEquals(rec.timestampUs(), decoded.timestampUs());
        assertEquals(rec.uasLsVersion(), decoded.uasLsVersion());
    }

    // -----------------------------------------------------------------------
    // DA-KLV-4: sentinel round-trip tests
    // -----------------------------------------------------------------------

    /**
     * Tag 6 (Platform Pitch) INT_MIN must decode with a null {@code platformPitchDeg},
     * tag 6 in {@code sentinelTags}, and no field errors.
     *
     * <p>Encodes a record with {@code sentinelTags=(6)} and no typed pitch field, which
     * causes the encoder to emit {@code 0x8000} (i16::MIN) per ST 0601.19 §8.6.
     * Decoding the result must record the sentinel without reporting an error.
     */
    @Test
    void sentinelDecodePopulatesSentinelTagsNotFieldError() throws KlvDecodeException, KlvEncodeException {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .sentinelTags(java.util.List.of(6L))
                .build();
        byte[] wire = Klv.encodeUasDatalink(rec);
        UasDatalinkLs rec2 = Klv.decodeUasDatalink(wire);
        assertNull(rec2.platformPitchDeg(), "INT_MIN sentinel must leave typed field null");
        assertTrue(rec2.sentinelTags().contains(6L), "tag 6 must appear in sentinelTags");
        assertTrue(rec2.fieldErrors().isEmpty(), "sentinel must not produce a field error");
    }

    /**
     * A sentinel record survives two encode/decode cycles with the sentinel preserved.
     */
    @Test
    void sentinelRoundTripsThroughEncode() throws KlvDecodeException, KlvEncodeException {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .sentinelTags(java.util.List.of(6L))
                .build();
        byte[] wire1 = Klv.encodeUasDatalink(rec);
        UasDatalinkLs rec2 = Klv.decodeUasDatalink(wire1);
        byte[] wire2 = Klv.encodeUasDatalink(rec2);
        UasDatalinkLs rec3 = Klv.decodeUasDatalink(wire2);
        assertNull(rec3.platformPitchDeg(), "sentinel field must remain null after re-encode");
        assertTrue(rec3.sentinelTags().contains(6L), "tag 6 must still be a sentinel after re-encode");
        assertTrue(rec3.fieldErrors().isEmpty(), "no field errors after re-encode");
    }

    /**
     * Value-wins: if a typed field is set AND its tag also appears in
     * {@code sentinelTags}, the value must be encoded — not INT_MIN.
     * Re-decoded record must carry the real roll value, not a sentinel.
     */
    @Test
    void sentinelValueWinsOverSentinelTagsEntry() throws KlvDecodeException, KlvEncodeException {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .platformRollDeg(25.0)
                .sentinelTags(java.util.List.of(7L))
                .build();
        byte[] encoded = Klv.encodeUasDatalink(rec);
        UasDatalinkLs rec2 = Klv.decodeUasDatalink(encoded);
        assertNotNull(rec2.platformRollDeg(), "value must survive (not replaced by sentinel)");
        assertEquals(25.0, rec2.platformRollDeg(), 0.5, "value must be close to 25.0");
        assertFalse(rec2.sentinelTags().contains(7L),
                "tag 7 must NOT be a sentinel after value-wins encoding");
    }

    /**
     * A {@code sentinelTags} entry above the u32 range must throw
     * {@link IllegalArgumentException} instead of being silently dropped
     * (regression test for the silent-drop path in the JNI translator).
     */
    @Test
    void sentinelTagsOutOfU32RangeThrows() {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .sentinelTags(java.util.List.of(0x1_0000_0000L))
                .build();
        assertThrows(IllegalArgumentException.class, () -> Klv.encodeUasDatalink(rec));
    }

    // -----------------------------------------------------------------------
    // OutOfRangePolicy overload tests
    // -----------------------------------------------------------------------

    /**
     * Tag 6 (Platform Pitch, ±20°) with value 25.0 is out of range.
     *
     * <p>The default 1-arg {@link Klv#encodeUasDatalink(UasDatalinkLs)} still
     * throws {@link KlvEncodeException} (policy = ERROR). The 2-arg overload
     * with {@link OutOfRangePolicy#INDICATOR} succeeds and the decoded record
     * carries a null {@code platformPitchDeg} with tag 6 in {@code sentinelTags}.
     */
    @Test
    void encodeOutOfRangeIndicatorPolicy() throws KlvDecodeException, KlvEncodeException {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .platformPitchDeg(25.0)
                .build();

        // Default (ERROR) policy still throws for an out-of-range value.
        assertThrows(KlvEncodeException.class, () -> Klv.encodeUasDatalink(rec));

        // INDICATOR policy emits the Out-of-Range special instead of erroring.
        byte[] raw = Klv.encodeUasDatalink(rec, OutOfRangePolicy.INDICATOR);
        UasDatalinkLs back = Klv.decodeUasDatalink(raw);
        assertNull(back.platformPitchDeg(),
                "Out-of-Range indicator must leave platformPitchDeg null");
        assertTrue(back.sentinelTags().contains(6L),
                "tag 6 must appear in sentinelTags after INDICATOR encode");
        assertTrue(back.fieldErrors().isEmpty(),
                "sentinel must not produce a field error on decode");
    }

    /**
     * The 2-arg overload with {@link OutOfRangePolicy#ERROR} behaves identically
     * to the 1-arg overload: it throws for an out-of-range value.
     */
    @Test
    void encodeErrorPolicyThrowsLikeDefault() {
        java.nio.ByteBuffer ul = java.nio.ByteBuffer.wrap(
                HexFormat.of().parseHex("060e2b34020b01010e01030101000000"));
        UasDatalinkLs rec = new UasDatalinkLs.Builder()
                .universalLabel(ul)
                .platformPitchDeg(25.0)
                .build();

        assertThrows(KlvEncodeException.class,
                () -> Klv.encodeUasDatalink(rec, OutOfRangePolicy.ERROR));
    }
}
