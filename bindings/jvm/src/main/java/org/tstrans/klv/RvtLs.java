package org.tstrans.klv;

import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0806.4 RVT (Remote Video Terminal) Local Set typed view.
 *
 * <p>Standalone-capable: carries its own 16-byte Universal Label and, per
 * ST 0806.4-02/-04, a timestamp-first Tag 2 plus a CRC-last Tag 1 when
 * transmitted independently — see {@link Klv#decodeRvtStandalone(byte[])} /
 * {@link Klv#encodeRvtStandalone(RvtLs)}. Also embeddable: ST 0601 Tag 73
 * carries the RVT LS <em>body</em> (no UL, no timestamp/CRC-position
 * requirement) — see {@link Klv#decodeRvt(byte[])} / {@link Klv#encodeRvt(RvtLs)}
 * and {@link UasDatalinkLs}.
 *
 * <p>The checksum ({@link #crc32()}) is CRC-32/MPEG-2 (ISO/IEC 13818-1) — a
 * real divergence from the ST 0601 16-bit running-sum checksum. It is
 * captured on decode but only verified by {@link Klv#decodeRvtStandalone(byte[])}
 * (an embedded RVT LS is not required to carry one).
 *
 * <p>Integer widths: u8/u16 fields are {@code Integer}; u32/u64 fields are
 * {@code Long} (matches {@code tst_core::klv::st0806::RvtLs}).
 *
 * <p>Instances are immutable; use {@link Builder} to construct.
 */
public record RvtLs(
        // Tag 1 — crc32 (u32; CRC-32/MPEG-2, observability for standalone RVT; recomputed by encode)
        Long crc32,
        // Tag 2 — timestampUs (u64, microseconds)
        Long timestampUs,
        // Tag 3 — platformTrueAirspeed (u16, m/s)
        Integer platformTrueAirspeed,
        // Tag 4 — platformIndicatedAirspeed (u16, m/s)
        Integer platformIndicatedAirspeed,
        // Tag 5 — telemetryAccuracyIndicator (u8; spec-reserved)
        Integer telemetryAccuracyIndicator,
        // Tag 6 — fragCircleRadiusM (u16)
        Integer fragCircleRadiusM,
        // Tag 7 — frameCode (u32, 60 Hz counter)
        Long frameCode,
        // Tag 8 — rvtLsVersion (u8; wire schema version, NOT necessarily 4)
        Integer rvtLsVersion,
        // Tag 9 — videoDataRate (u32, bps/Hz)
        Long videoDataRate,
        // Tag 10 — digitalVideoFileFormat (ISO 7 string, max 127 bytes)
        String digitalVideoFileFormat,
        // Tag 11 — userDefined (repeatable)
        List<RvtUserData> userDefined,
        // Tag 12 — pointsOfInterest (repeatable)
        List<RvtPoi> pointsOfInterest,
        // Tag 13 — areasOfInterest (repeatable)
        List<RvtAoi> areasOfInterest,
        // Tag 14 — aircraftMgrsZone (u8, UTM zone 1-60)
        Integer aircraftMgrsZone,
        // Tag 15 — aircraftMgrsBandGrid (3-char ISO 7 string)
        String aircraftMgrsBandGrid,
        // Tag 16 — aircraftMgrsEastingM (u24, 0-99999)
        Long aircraftMgrsEastingM,
        // Tag 17 — aircraftMgrsNorthingM (u24, 0-99999)
        Long aircraftMgrsNorthingM,
        // Tag 18 — frameCenterMgrsZone (u8, UTM zone 1-60)
        Integer frameCenterMgrsZone,
        // Tag 19 — frameCenterMgrsBandGrid (3-char ISO 7 string)
        String frameCenterMgrsBandGrid,
        // Tag 20 — frameCenterMgrsEastingM (u24, 0-99999)
        Long frameCenterMgrsEastingM,
        // Tag 21 — frameCenterMgrsNorthingM (u24, 0-99999)
        Long frameCenterMgrsNorthingM,
        // Forward-compat unknown TLVs
        List<KlvUnknownField> unknown,
        // Non-fatal per-field decode errors
        List<KlvFieldError> fieldErrors
) {

    /** Compact constructor: make list fields truly immutable. */
    public RvtLs {
        userDefined = userDefined != null
                ? Collections.unmodifiableList(userDefined) : Collections.emptyList();
        pointsOfInterest = pointsOfInterest != null
                ? Collections.unmodifiableList(pointsOfInterest) : Collections.emptyList();
        areasOfInterest = areasOfInterest != null
                ? Collections.unmodifiableList(areasOfInterest) : Collections.emptyList();
        unknown = unknown != null
                ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null
                ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    /**
     * Reconstruct the 15-char aircraft MGRS string (Tags 14-17): zone
     * zero-padded to 2, band+grid 3 chars, easting/northing zero-padded to
     * 5. Pure-Java port of the Rust {@code RvtLs::aircraft_mgrs} helper.
     *
     * @return the composite MGRS string, or {@code null} if any of the four
     *         components is missing
     */
    public String aircraftMgrs() {
        return mgrsString(aircraftMgrsZone, aircraftMgrsBandGrid, aircraftMgrsEastingM, aircraftMgrsNorthingM);
    }

    /**
     * Frame-center MGRS string (Tags 18-21), same layout as {@link #aircraftMgrs()}.
     *
     * @return the composite MGRS string, or {@code null} if any of the four
     *         components is missing
     */
    public String frameCenterMgrs() {
        return mgrsString(frameCenterMgrsZone, frameCenterMgrsBandGrid, frameCenterMgrsEastingM, frameCenterMgrsNorthingM);
    }

    private static String mgrsString(Integer zone, String bandGrid, Long easting, Long northing) {
        if (zone == null || bandGrid == null || easting == null || northing == null) {
            return null;
        }
        return String.format("%02d%s%05d%05d", zone, bandGrid, easting, northing);
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Fluent mutable builder for {@link RvtLs}. All fields are optional.
     * List fields default to empty immutable lists.
     */
    public static final class Builder {
        private Long crc32;
        private Long timestampUs;
        private Integer platformTrueAirspeed;
        private Integer platformIndicatedAirspeed;
        private Integer telemetryAccuracyIndicator;
        private Integer fragCircleRadiusM;
        private Long frameCode;
        private Integer rvtLsVersion;
        private Long videoDataRate;
        private String digitalVideoFileFormat;
        private List<RvtUserData> userDefined = Collections.emptyList();
        private List<RvtPoi> pointsOfInterest = Collections.emptyList();
        private List<RvtAoi> areasOfInterest = Collections.emptyList();
        private Integer aircraftMgrsZone;
        private String aircraftMgrsBandGrid;
        private Long aircraftMgrsEastingM;
        private Long aircraftMgrsNorthingM;
        private Integer frameCenterMgrsZone;
        private String frameCenterMgrsBandGrid;
        private Long frameCenterMgrsEastingM;
        private Long frameCenterMgrsNorthingM;
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        /** Create an empty Builder. */
        public Builder() {}

        public Builder crc32(long v) { this.crc32 = v; return this; }
        public Builder timestampUs(long v) { this.timestampUs = v; return this; }
        public Builder platformTrueAirspeed(int v) { this.platformTrueAirspeed = v; return this; }
        public Builder platformIndicatedAirspeed(int v) { this.platformIndicatedAirspeed = v; return this; }
        public Builder telemetryAccuracyIndicator(int v) { this.telemetryAccuracyIndicator = v; return this; }
        public Builder fragCircleRadiusM(int v) { this.fragCircleRadiusM = v; return this; }
        public Builder frameCode(long v) { this.frameCode = v; return this; }
        public Builder rvtLsVersion(int v) { this.rvtLsVersion = v; return this; }
        public Builder videoDataRate(long v) { this.videoDataRate = v; return this; }
        public Builder digitalVideoFileFormat(String v) { this.digitalVideoFileFormat = v; return this; }
        public Builder userDefined(List<RvtUserData> v) { this.userDefined = v; return this; }
        public Builder pointsOfInterest(List<RvtPoi> v) { this.pointsOfInterest = v; return this; }
        public Builder areasOfInterest(List<RvtAoi> v) { this.areasOfInterest = v; return this; }
        public Builder aircraftMgrsZone(int v) { this.aircraftMgrsZone = v; return this; }
        public Builder aircraftMgrsBandGrid(String v) { this.aircraftMgrsBandGrid = v; return this; }
        public Builder aircraftMgrsEastingM(long v) { this.aircraftMgrsEastingM = v; return this; }
        public Builder aircraftMgrsNorthingM(long v) { this.aircraftMgrsNorthingM = v; return this; }
        public Builder frameCenterMgrsZone(int v) { this.frameCenterMgrsZone = v; return this; }
        public Builder frameCenterMgrsBandGrid(String v) { this.frameCenterMgrsBandGrid = v; return this; }
        public Builder frameCenterMgrsEastingM(long v) { this.frameCenterMgrsEastingM = v; return this; }
        public Builder frameCenterMgrsNorthingM(long v) { this.frameCenterMgrsNorthingM = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }

        /** Build an immutable {@link RvtLs}. */
        public RvtLs build() {
            return new RvtLs(
                    crc32, timestampUs,
                    platformTrueAirspeed, platformIndicatedAirspeed,
                    telemetryAccuracyIndicator, fragCircleRadiusM,
                    frameCode, rvtLsVersion, videoDataRate, digitalVideoFileFormat,
                    userDefined, pointsOfInterest, areasOfInterest,
                    aircraftMgrsZone, aircraftMgrsBandGrid, aircraftMgrsEastingM, aircraftMgrsNorthingM,
                    frameCenterMgrsZone, frameCenterMgrsBandGrid, frameCenterMgrsEastingM, frameCenterMgrsNorthingM,
                    unknown, fieldErrors);
        }
    }
}
