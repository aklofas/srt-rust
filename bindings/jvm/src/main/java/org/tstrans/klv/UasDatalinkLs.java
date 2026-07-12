package org.tstrans.klv;

import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.List;
import java.util.Optional;

/**
 * MISB ST 0601 UAS Datalink Local Set typed view.
 *
 * <p>Mirror of the 80-field Rust {@code tst_core::klv::st0601::UasDatalinkLs}
 * flat struct. Composite views (sensor position, attitude, FOV, corners) are
 * accessor methods that return {@link Optional#empty()} when any of the
 * underlying primitive fields is absent.
 *
 * <p>{@link #securityLocalSet()} carries the Tag 48 ST 0102 LS body bytes
 * (no UL prefix); call {@link Klv#decodeSecurity(byte[])} for typed access.
 *
 * <p>{@link #vmti()} carries the Tag 74 ST 0903 LS body bytes (no UL prefix);
 * call {@link Klv#decodeVmti(byte[])} for typed access.
 *
 * <p>{@code unknown} preserves any tag not in the typed-modeled set per
 * ST 0107.5 §6 future-proof skip rule. {@code fieldErrors} collects per-field
 * decode failures from lenient mode; strict mode raises
 * {@link org.tstrans.KlvDecodeException} instead.
 *
 * <p>Decode via {@link Klv#decodeUasDatalink(byte[])}; encode via
 * {@link Klv#encodeUasDatalink(UasDatalinkLs)}.
 *
 * <h2>Encode-enforced ranges (MISB ST 0601.19)</h2>
 *
 * <p>Values outside the listed range cause {@code KlvEncodeException} on
 * {@link Klv#encodeUasDatalink}. Narrow/full twins: pitch 6&harr;90,
 * roll 7&harr;91, corner offsets 26&ndash;33&harr;82&ndash;89 (absolute).
 *
 * <pre>
 * Platform state
 *   platformHeadingDeg          Tag  5  [0, 360] deg
 *   platformPitchDeg            Tag  6  [-20, 20] deg      narrow; full twin: platformPitchFullDeg (Tag 90)
 *   platformRollDeg             Tag  7  [-50, 50] deg      narrow; full twin: platformRollFullDeg (Tag 91)
 *   platformTrueAirspeed        Tag  8  [0, 255] m/s
 *   platformIndicatedAirspeed   Tag  9  [0, 255] m/s
 *   platformPitchFullDeg        Tag 90  [-90, 90] deg      full twin of platformPitchDeg (Tag 6)
 *   platformRollFullDeg         Tag 91  [-90, 90] deg      full twin of platformRollDeg (Tag 7)
 *   platformAngleOfAttackDeg    Tag 50  [-20, 20] deg
 *
 * Sensor pose &amp; position
 *   sensorLatDeg                Tag 13  [-90, 90] deg
 *   sensorLonDeg                Tag 14  [-180, 180] deg
 *   sensorAltM                  Tag 15  [-900, 19000] m
 *   sensorEllipsoidHeightM      Tag 75  [-900, 19000] m
 *   sensorHfovDeg               Tag 16  [0, 180] deg
 *   sensorVfovDeg               Tag 17  [0, 180] deg
 *   sensorRelAzDeg              Tag 18  [0, 360] deg
 *   sensorRelElDeg              Tag 19  [-180, 180] deg
 *   sensorRelRollDeg            Tag 20  [0, 360] deg
 *
 * Ranging &amp; frame center
 *   slantRangeM                 Tag 21  [0, 5000000] m
 *   targetWidthM                Tag 22  [0, 10000] m
 *   frameCenterLatDeg           Tag 23  [-90, 90] deg
 *   frameCenterLonDeg           Tag 24  [-180, 180] deg
 *   frameCenterElevM            Tag 25  [-900, 19000] m
 *   frameCenterEllipsoidHeightM Tag 78  [-900, 19000] m
 *
 * Corner offsets (narrow +-0.075 deg)
 *   cornerLatOffsetP1Deg        Tag 26  [-0.075, 0.075] deg  full twin: cornerLatP1Deg (Tag 82)
 *   cornerLonOffsetP1Deg        Tag 27  [-0.075, 0.075] deg  full twin: cornerLonP1Deg (Tag 83)
 *   cornerLatOffsetP2Deg        Tag 28  [-0.075, 0.075] deg  full twin: cornerLatP2Deg (Tag 84)
 *   cornerLonOffsetP2Deg        Tag 29  [-0.075, 0.075] deg  full twin: cornerLonP2Deg (Tag 85)
 *   cornerLatOffsetP3Deg        Tag 30  [-0.075, 0.075] deg  full twin: cornerLatP3Deg (Tag 86)
 *   cornerLonOffsetP3Deg        Tag 31  [-0.075, 0.075] deg  full twin: cornerLonP3Deg (Tag 87)
 *   cornerLatOffsetP4Deg        Tag 32  [-0.075, 0.075] deg  full twin: cornerLatP4Deg (Tag 88)
 *   cornerLonOffsetP4Deg        Tag 33  [-0.075, 0.075] deg  full twin: cornerLonP4Deg (Tag 89)
 *
 * Corner full lat/lon (ST 0601.13+)
 *   cornerLatP1Deg              Tag 82  [-90, 90] deg      full twin of cornerLatOffsetP1Deg (Tag 26)
 *   cornerLonP1Deg              Tag 83  [-180, 180] deg    full twin of cornerLonOffsetP1Deg (Tag 27)
 *   cornerLatP2Deg              Tag 84  [-90, 90] deg      full twin of cornerLatOffsetP2Deg (Tag 28)
 *   cornerLonP2Deg              Tag 85  [-180, 180] deg    full twin of cornerLonOffsetP2Deg (Tag 29)
 *   cornerLatP3Deg              Tag 86  [-90, 90] deg      full twin of cornerLatOffsetP3Deg (Tag 30)
 *   cornerLonP3Deg              Tag 87  [-180, 180] deg    full twin of cornerLonOffsetP3Deg (Tag 31)
 *   cornerLatP4Deg              Tag 88  [-90, 90] deg      full twin of cornerLatOffsetP4Deg (Tag 32)
 *   cornerLonP4Deg              Tag 89  [-180, 180] deg    full twin of cornerLonOffsetP4Deg (Tag 33)
 * </pre>
 */
public record UasDatalinkLs(
        // Non-optional identity fields (always present on the wire after decode)
        ByteBuffer universalLabel,
        int declaredVersion,

        // Identity
        String missionId,
        String platformTailNumber,
        String platformDesignation,
        String imageSourceSensor,
        String imageCoordinateSystem,
        String platformCallSign,
        Integer uasLsVersion,

        // Time
        Long timestampUs,

        // Platform state
        Double platformHeadingDeg,
        Double platformPitchDeg,
        Double platformRollDeg,
        Double platformTrueAirspeed,
        Double platformIndicatedAirspeed,
        Double platformPitchFullDeg,
        Double platformRollFullDeg,
        Double platformAngleOfAttackDeg,

        // Sensor pose & position
        Double sensorLatDeg,
        Double sensorLonDeg,
        Double sensorAltM,
        Double sensorEllipsoidHeightM,
        Double sensorHfovDeg,
        Double sensorVfovDeg,
        Double sensorRelAzDeg,
        Double sensorRelElDeg,
        Double sensorRelRollDeg,

        // Ranging & frame center
        Double slantRangeM,
        Double targetWidthM,
        Double frameCenterLatDeg,
        Double frameCenterLonDeg,
        Double frameCenterElevM,
        Double frameCenterEllipsoidHeightM,

        // Image corner offsets from frame center (tags 26–33)
        Double cornerLatOffsetP1Deg,
        Double cornerLonOffsetP1Deg,
        Double cornerLatOffsetP2Deg,
        Double cornerLonOffsetP2Deg,
        Double cornerLatOffsetP3Deg,
        Double cornerLonOffsetP3Deg,
        Double cornerLatOffsetP4Deg,
        Double cornerLonOffsetP4Deg,

        // Image corners — full lat/lon (tags 82–89, ST 0601.13+)
        Double cornerLatP1Deg,
        Double cornerLonP1Deg,
        Double cornerLatP2Deg,
        Double cornerLonP2Deg,
        Double cornerLatP3Deg,
        Double cornerLonP3Deg,
        Double cornerLatP4Deg,
        Double cornerLonP4Deg,

        // Misc
        Integer genericFlagData,
        ByteBuffer securityLocalSet,
        ByteBuffer vmti,

        /**
         * Tag 94 — MIIS Core Identifier raw bytes (ST 0601.19 §8.94 / ST 1204.3).
         * {@code null} when absent. Call {@link Klv#decodeCoreId(byte[])} to parse
         * these bytes into a typed {@link CoreId}.
         */
        byte[] miisCoreId,

        // Pass-through
        List<KlvUnknownField> unknown,
        List<KlvFieldError> fieldErrors,

        /**
         * Tags whose wire value was the INT_MIN sentinel for their signed
         * linear mapping. INT_MIN is a spec-defined signal, not an error;
         * the corresponding typed field is {@code null} and the tag number
         * is recorded here. Consult the special-value assignments table in
         * ST 0601.19 to look up the spec-defined meaning for each tag number.
         */
        List<Long> sentinelTags
) implements KlvSet {

    /**
     * Compact constructor — validates {@code universalLabel} is exactly 16 bytes
     * and that the lists are non-null.
     */
    public UasDatalinkLs {
        if (universalLabel == null || universalLabel.capacity() != 16) {
            int cap = universalLabel == null ? 0 : universalLabel.capacity();
            throw new IllegalArgumentException(
                    "UasDatalinkLs.universalLabel must be exactly 16 bytes; got " + cap);
        }
        unknown = unknown == null ? Collections.emptyList() : Collections.unmodifiableList(unknown);
        fieldErrors = fieldErrors == null ? Collections.emptyList() : Collections.unmodifiableList(fieldErrors);
        sentinelTags = sentinelTags == null ? Collections.emptyList() : Collections.unmodifiableList(sentinelTags);
    }

    // -----------------------------------------------------------------------
    // Composite accessor methods
    // -----------------------------------------------------------------------

    /**
     * Return the sensor position when all three underlying fields are present.
     * Mirrors tst-py's {@code sensor_position()}.
     *
     * @return {@link GeoPoint} with lat/lon/alt, or {@link Optional#empty()}
     */
    public Optional<GeoPoint> sensorPosition() {
        if (sensorLatDeg != null && sensorLonDeg != null && sensorAltM != null) {
            return Optional.of(new GeoPoint(sensorLatDeg, sensorLonDeg, sensorAltM));
        }
        return Optional.empty();
    }

    /**
     * Return the sensor relative attitude when all three underlying fields are present.
     * Uses sensor relative azimuth/elevation/roll (tags 18/19/20). Mirrors tst-py's
     * {@code sensor_attitude()}.
     *
     * @return {@link Attitude} with heading/pitch/roll in degrees, or empty
     */
    public Optional<Attitude> sensorAttitude() {
        if (sensorRelAzDeg != null && sensorRelElDeg != null && sensorRelRollDeg != null) {
            return Optional.of(new Attitude(sensorRelAzDeg, sensorRelElDeg, sensorRelRollDeg));
        }
        return Optional.empty();
    }

    /**
     * Return the sensor field-of-view when both underlying fields are present.
     * Mirrors tst-py's {@code sensor_fov()}.
     *
     * @return {@link FieldOfView} with horizontal/vertical degrees, or empty
     */
    public Optional<FieldOfView> sensorFov() {
        if (sensorHfovDeg != null && sensorVfovDeg != null) {
            return Optional.of(new FieldOfView(sensorHfovDeg, sensorVfovDeg));
        }
        return Optional.empty();
    }

    /**
     * Return the platform attitude when all three underlying fields are present.
     * Uses platform heading/pitch/roll (tags 5/6/7). Mirrors tst-py's
     * {@code platform_attitude()}.
     *
     * @return {@link Attitude} with heading/pitch/roll in degrees, or empty
     */
    public Optional<Attitude> platformAttitude() {
        if (platformHeadingDeg != null && platformPitchDeg != null && platformRollDeg != null) {
            return Optional.of(new Attitude(platformHeadingDeg, platformPitchDeg, platformRollDeg));
        }
        return Optional.empty();
    }

    /**
     * Return the frame center when all three underlying fields are present.
     * Mirrors tst-py's {@code frame_center()}.
     *
     * @return {@link GeoPoint} with lat/lon/elev, or empty
     */
    public Optional<GeoPoint> frameCenter() {
        if (frameCenterLatDeg != null && frameCenterLonDeg != null && frameCenterElevM != null) {
            return Optional.of(new GeoPoint(frameCenterLatDeg, frameCenterLonDeg, frameCenterElevM));
        }
        return Optional.empty();
    }

    /**
     * Return the sensor footprint corners. Prefers the absolute corner tags
     * (82–89) when all eight are present; falls back to frame-center + offset
     * tags (26–33) when all eight offsets and both frame-center lat/lon are
     * present; otherwise returns empty. Mirrors tst-py's {@code corners()}.
     *
     * @return {@link Corners} with four {@link LatLon} points, or empty
     */
    public Optional<Corners> corners() {
        // Prefer absolute (tags 82-89) when fully populated.
        if (cornerLatP1Deg != null && cornerLonP1Deg != null
                && cornerLatP2Deg != null && cornerLonP2Deg != null
                && cornerLatP3Deg != null && cornerLonP3Deg != null
                && cornerLatP4Deg != null && cornerLonP4Deg != null) {
            return Optional.of(new Corners(
                    new LatLon(cornerLatP1Deg, cornerLonP1Deg),
                    new LatLon(cornerLatP2Deg, cornerLonP2Deg),
                    new LatLon(cornerLatP3Deg, cornerLonP3Deg),
                    new LatLon(cornerLatP4Deg, cornerLonP4Deg)
            ));
        }
        // Fall back to offsets + frame center.
        if (frameCenterLatDeg == null || frameCenterLonDeg == null) {
            return Optional.empty();
        }
        if (cornerLatOffsetP1Deg == null || cornerLonOffsetP1Deg == null
                || cornerLatOffsetP2Deg == null || cornerLonOffsetP2Deg == null
                || cornerLatOffsetP3Deg == null || cornerLonOffsetP3Deg == null
                || cornerLatOffsetP4Deg == null || cornerLonOffsetP4Deg == null) {
            return Optional.empty();
        }
        double lat0 = frameCenterLatDeg;
        double lon0 = frameCenterLonDeg;
        return Optional.of(new Corners(
                new LatLon(lat0 + cornerLatOffsetP1Deg, lon0 + cornerLonOffsetP1Deg),
                new LatLon(lat0 + cornerLatOffsetP2Deg, lon0 + cornerLonOffsetP2Deg),
                new LatLon(lat0 + cornerLatOffsetP3Deg, lon0 + cornerLonOffsetP3Deg),
                new LatLon(lat0 + cornerLatOffsetP4Deg, lon0 + cornerLonOffsetP4Deg)
        ));
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /** Public mutable builder for {@link UasDatalinkLs}. */
    public static final class Builder {
        private ByteBuffer universalLabel;
        private int declaredVersion;
        private String missionId;
        private String platformTailNumber;
        private String platformDesignation;
        private String imageSourceSensor;
        private String imageCoordinateSystem;
        private String platformCallSign;
        private Integer uasLsVersion;
        private Long timestampUs;
        private Double platformHeadingDeg;
        private Double platformPitchDeg;
        private Double platformRollDeg;
        private Double platformTrueAirspeed;
        private Double platformIndicatedAirspeed;
        private Double platformPitchFullDeg;
        private Double platformRollFullDeg;
        private Double platformAngleOfAttackDeg;
        private Double sensorLatDeg;
        private Double sensorLonDeg;
        private Double sensorAltM;
        private Double sensorEllipsoidHeightM;
        private Double sensorHfovDeg;
        private Double sensorVfovDeg;
        private Double sensorRelAzDeg;
        private Double sensorRelElDeg;
        private Double sensorRelRollDeg;
        private Double slantRangeM;
        private Double targetWidthM;
        private Double frameCenterLatDeg;
        private Double frameCenterLonDeg;
        private Double frameCenterElevM;
        private Double frameCenterEllipsoidHeightM;
        private Double cornerLatOffsetP1Deg;
        private Double cornerLonOffsetP1Deg;
        private Double cornerLatOffsetP2Deg;
        private Double cornerLonOffsetP2Deg;
        private Double cornerLatOffsetP3Deg;
        private Double cornerLonOffsetP3Deg;
        private Double cornerLatOffsetP4Deg;
        private Double cornerLonOffsetP4Deg;
        private Double cornerLatP1Deg;
        private Double cornerLonP1Deg;
        private Double cornerLatP2Deg;
        private Double cornerLonP2Deg;
        private Double cornerLatP3Deg;
        private Double cornerLonP3Deg;
        private Double cornerLatP4Deg;
        private Double cornerLonP4Deg;
        private Integer genericFlagData;
        private ByteBuffer securityLocalSet;
        private ByteBuffer vmti;
        private byte[] miisCoreId;
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();
        private List<Long> sentinelTags = Collections.emptyList();

        public Builder() {}

        public Builder universalLabel(ByteBuffer v) { this.universalLabel = v; return this; }
        public Builder declaredVersion(int v) { this.declaredVersion = v; return this; }
        public Builder missionId(String v) { this.missionId = v; return this; }
        public Builder platformTailNumber(String v) { this.platformTailNumber = v; return this; }
        public Builder platformDesignation(String v) { this.platformDesignation = v; return this; }
        public Builder imageSourceSensor(String v) { this.imageSourceSensor = v; return this; }
        public Builder imageCoordinateSystem(String v) { this.imageCoordinateSystem = v; return this; }
        public Builder platformCallSign(String v) { this.platformCallSign = v; return this; }
        public Builder uasLsVersion(int v) { this.uasLsVersion = v; return this; }
        public Builder timestampUs(long v) { this.timestampUs = v; return this; }
        public Builder platformHeadingDeg(double v) { this.platformHeadingDeg = v; return this; }
        public Builder platformPitchDeg(double v) { this.platformPitchDeg = v; return this; }
        public Builder platformRollDeg(double v) { this.platformRollDeg = v; return this; }
        public Builder platformTrueAirspeed(double v) { this.platformTrueAirspeed = v; return this; }
        public Builder platformIndicatedAirspeed(double v) { this.platformIndicatedAirspeed = v; return this; }
        public Builder platformPitchFullDeg(double v) { this.platformPitchFullDeg = v; return this; }
        public Builder platformRollFullDeg(double v) { this.platformRollFullDeg = v; return this; }
        public Builder platformAngleOfAttackDeg(double v) { this.platformAngleOfAttackDeg = v; return this; }
        public Builder sensorLatDeg(double v) { this.sensorLatDeg = v; return this; }
        public Builder sensorLonDeg(double v) { this.sensorLonDeg = v; return this; }
        public Builder sensorAltM(double v) { this.sensorAltM = v; return this; }
        public Builder sensorEllipsoidHeightM(double v) { this.sensorEllipsoidHeightM = v; return this; }
        public Builder sensorHfovDeg(double v) { this.sensorHfovDeg = v; return this; }
        public Builder sensorVfovDeg(double v) { this.sensorVfovDeg = v; return this; }
        public Builder sensorRelAzDeg(double v) { this.sensorRelAzDeg = v; return this; }
        public Builder sensorRelElDeg(double v) { this.sensorRelElDeg = v; return this; }
        public Builder sensorRelRollDeg(double v) { this.sensorRelRollDeg = v; return this; }
        public Builder slantRangeM(double v) { this.slantRangeM = v; return this; }
        public Builder targetWidthM(double v) { this.targetWidthM = v; return this; }
        public Builder frameCenterLatDeg(double v) { this.frameCenterLatDeg = v; return this; }
        public Builder frameCenterLonDeg(double v) { this.frameCenterLonDeg = v; return this; }
        public Builder frameCenterElevM(double v) { this.frameCenterElevM = v; return this; }
        public Builder frameCenterEllipsoidHeightM(double v) { this.frameCenterEllipsoidHeightM = v; return this; }
        public Builder cornerLatOffsetP1Deg(double v) { this.cornerLatOffsetP1Deg = v; return this; }
        public Builder cornerLonOffsetP1Deg(double v) { this.cornerLonOffsetP1Deg = v; return this; }
        public Builder cornerLatOffsetP2Deg(double v) { this.cornerLatOffsetP2Deg = v; return this; }
        public Builder cornerLonOffsetP2Deg(double v) { this.cornerLonOffsetP2Deg = v; return this; }
        public Builder cornerLatOffsetP3Deg(double v) { this.cornerLatOffsetP3Deg = v; return this; }
        public Builder cornerLonOffsetP3Deg(double v) { this.cornerLonOffsetP3Deg = v; return this; }
        public Builder cornerLatOffsetP4Deg(double v) { this.cornerLatOffsetP4Deg = v; return this; }
        public Builder cornerLonOffsetP4Deg(double v) { this.cornerLonOffsetP4Deg = v; return this; }
        public Builder cornerLatP1Deg(double v) { this.cornerLatP1Deg = v; return this; }
        public Builder cornerLonP1Deg(double v) { this.cornerLonP1Deg = v; return this; }
        public Builder cornerLatP2Deg(double v) { this.cornerLatP2Deg = v; return this; }
        public Builder cornerLonP2Deg(double v) { this.cornerLonP2Deg = v; return this; }
        public Builder cornerLatP3Deg(double v) { this.cornerLatP3Deg = v; return this; }
        public Builder cornerLonP3Deg(double v) { this.cornerLonP3Deg = v; return this; }
        public Builder cornerLatP4Deg(double v) { this.cornerLatP4Deg = v; return this; }
        public Builder cornerLonP4Deg(double v) { this.cornerLonP4Deg = v; return this; }
        public Builder genericFlagData(int v) { this.genericFlagData = v; return this; }
        public Builder securityLocalSet(ByteBuffer v) { this.securityLocalSet = v; return this; }
        public Builder vmti(ByteBuffer v) { this.vmti = v; return this; }
        public Builder miisCoreId(byte[] v) { this.miisCoreId = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }
        public Builder sentinelTags(List<Long> v) { this.sentinelTags = v; return this; }

        /** Build an immutable {@link UasDatalinkLs}. */
        public UasDatalinkLs build() {
            return new UasDatalinkLs(
                    universalLabel, declaredVersion,
                    missionId, platformTailNumber, platformDesignation,
                    imageSourceSensor, imageCoordinateSystem, platformCallSign,
                    uasLsVersion,
                    timestampUs,
                    platformHeadingDeg, platformPitchDeg, platformRollDeg,
                    platformTrueAirspeed, platformIndicatedAirspeed,
                    platformPitchFullDeg, platformRollFullDeg, platformAngleOfAttackDeg,
                    sensorLatDeg, sensorLonDeg, sensorAltM, sensorEllipsoidHeightM,
                    sensorHfovDeg, sensorVfovDeg,
                    sensorRelAzDeg, sensorRelElDeg, sensorRelRollDeg,
                    slantRangeM, targetWidthM,
                    frameCenterLatDeg, frameCenterLonDeg, frameCenterElevM,
                    frameCenterEllipsoidHeightM,
                    cornerLatOffsetP1Deg, cornerLonOffsetP1Deg,
                    cornerLatOffsetP2Deg, cornerLonOffsetP2Deg,
                    cornerLatOffsetP3Deg, cornerLonOffsetP3Deg,
                    cornerLatOffsetP4Deg, cornerLonOffsetP4Deg,
                    cornerLatP1Deg, cornerLonP1Deg,
                    cornerLatP2Deg, cornerLonP2Deg,
                    cornerLatP3Deg, cornerLonP3Deg,
                    cornerLatP4Deg, cornerLonP4Deg,
                    genericFlagData,
                    securityLocalSet, vmti,
                    miisCoreId,
                    unknown, fieldErrors,
                    sentinelTags
            );
        }
    }
}
