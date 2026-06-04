package org.tstrans.klv;

import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0903.6 §10.2 Table 10 per-target pack.
 *
 * <p>Wire form is a leading BER-OID-encoded {@code targetId} (no Tag per §10.2.2.1)
 * followed by a Local Set body with BER-OID tag + BER short/long length + value tuples.
 *
 * <p>Seven nested LSes ({@code targetLocation}, {@code geospatialContourSeries},
 * {@code vmask}, {@code vtracker}, {@code vchip}, {@code vchipSeries},
 * {@code vobjectSeries}) remain as {@link ByteBuffer} pass-through bytes —
 * typed inner layers deferred (see {@code docs/project/deferred-features.md}).
 *
 * <p>{@code detectionStatus} is the raw §10.2.2.24 / §7.2 Table 5 codepoint:
 * 0=Inactive, 1=Active-Moving, 2=Dropped, 3=Active-Stopped, 4=Active-Coasting.
 * Typed enum deferred.
 *
 * <p>Instances are immutable; use {@link Builder} to construct. Does NOT implement
 * {@link KlvSet} — VTargetPack is carried inside {@link VmtiLs#targets()}, not
 * dispatched via {@link Klv#parseUniversal}.
 */
public record VTargetPack(
        // BER-OID targetId (mandatory, no Tag)
        long targetId,
        // Tag 1 — targetCentroid, pixel number V6
        Long centroidPixel,
        // Tag 2 — boundingBoxTopLeft, pixel number V6
        Long bboxTopLeftPixel,
        // Tag 3 — boundingBoxBottomRight, pixel number V6
        Long bboxBottomRightPixel,
        // Tag 4 — targetPriority, 1 byte 1..=255
        Integer priority,
        // Tag 5 — targetConfidenceLevel, 1 byte 0..=100
        Integer confidenceLevel,
        // Tag 6 — targetHistory, V2 0..=65535 frames
        Integer history,
        // Tag 7 — percentageOfTargetPixels, 1 byte 1..=100
        Integer percentageOfTargetPixels,
        // Tag 8 — targetColor, fixed 3 bytes [R, G, B]
        TargetColor targetColor,
        // Tag 9 — targetIntensity, V3 24-bit
        Long targetIntensity,
        // Tag 10 — targetLocationOffsetLat, IMAPB(-19.2, 19.2, 3)
        Double centroidLatOffset,
        // Tag 11 — targetLocationOffsetLon, IMAPB(-19.2, 19.2, 3)
        Double centroidLonOffset,
        // Tag 12 — targetHae, IMAPB(-900, 19000, 2)
        Double centroidHae,
        // Tag 13 — boundingBoxTopLeftLatOffset, IMAPB(-19.2, 19.2, 3)
        Double bboxTopLeftLatOffset,
        // Tag 14 — boundingBoxTopLeftLonOffset, IMAPB(-19.2, 19.2, 3)
        Double bboxTopLeftLonOffset,
        // Tag 15 — boundingBoxBottomRightLatOffset, IMAPB(-19.2, 19.2, 3)
        Double bboxBottomRightLatOffset,
        // Tag 16 — boundingBoxBottomRightLonOffset, IMAPB(-19.2, 19.2, 3)
        Double bboxBottomRightLonOffset,
        // Tag 17 — targetLocation, Defined Length Truncation Pack pass-through
        ByteBuffer targetLocation,
        // Tag 18 — geospatialContourSeries, BoundarySeries pass-through
        ByteBuffer geospatialContourSeries,
        // Tag 19 — centroidPixRow, V4
        Long centroidPixRow,
        // Tag 20 — centroidPixCol, V4
        Long centroidPixCol,
        // Tag 22 — algorithmId, V3
        Long algorithmId,
        // Tag 23 — detectionStatus, 1 byte codepoint
        Integer detectionStatus,
        // Tag 101 — vMask LS pass-through
        ByteBuffer vmask,
        // Tag 104 — vTracker LS pass-through
        ByteBuffer vtracker,
        // Tag 105 — vChip LS pass-through
        ByteBuffer vchip,
        // Tag 106 — vChipSeries pass-through
        ByteBuffer vchipSeries,
        // Tag 107 — vObjectSeries pass-through
        ByteBuffer vobjectSeries,
        // Forward-compat unknown TLVs
        List<KlvUnknownField> unknown,
        // Non-fatal per-field decode errors
        List<KlvFieldError> fieldErrors
) {
    /**
     * Immutable 3-channel RGB color (ST 0903.6 §10.2.2.9 Tag 8).
     * Each channel must be 0–255. Modeled as a nested record for value-equality
     * (avoids the array-identity problem with {@code int[]}).
     */
    public record TargetColor(int r, int g, int b) {
        /** Compact constructor validates each channel is in range 0..=255. */
        public TargetColor {
            if (r < 0 || r > 255 || g < 0 || g > 255 || b < 0 || b > 255) {
                throw new IllegalArgumentException(
                        "TargetColor channels must be 0..=255; got r=" + r + " g=" + g + " b=" + b);
            }
        }
    }

    /** Compact constructor: make list fields truly immutable. */
    public VTargetPack {
        unknown = unknown != null ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Fluent mutable builder for {@link VTargetPack}.
     *
     * <p>Only {@code targetId} is mandatory; all other fields are optional.
     * Lists default to empty immutable lists.
     */
    public static final class Builder {
        private final long targetId;
        private Long centroidPixel;
        private Long bboxTopLeftPixel;
        private Long bboxBottomRightPixel;
        private Integer priority;
        private Integer confidenceLevel;
        private Integer history;
        private Integer percentageOfTargetPixels;
        private TargetColor targetColor;
        private Long targetIntensity;
        private Double centroidLatOffset;
        private Double centroidLonOffset;
        private Double centroidHae;
        private Double bboxTopLeftLatOffset;
        private Double bboxTopLeftLonOffset;
        private Double bboxBottomRightLatOffset;
        private Double bboxBottomRightLonOffset;
        private ByteBuffer targetLocation;
        private ByteBuffer geospatialContourSeries;
        private Long centroidPixRow;
        private Long centroidPixCol;
        private Long algorithmId;
        private Integer detectionStatus;
        private ByteBuffer vmask;
        private ByteBuffer vtracker;
        private ByteBuffer vchip;
        private ByteBuffer vchipSeries;
        private ByteBuffer vobjectSeries;
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        /** Create a Builder with the mandatory {@code targetId}. */
        public Builder(long targetId) {
            this.targetId = targetId;
        }

        public Builder centroidPixel(long v) { this.centroidPixel = v; return this; }
        public Builder bboxTopLeftPixel(long v) { this.bboxTopLeftPixel = v; return this; }
        public Builder bboxBottomRightPixel(long v) { this.bboxBottomRightPixel = v; return this; }
        public Builder priority(int v) { this.priority = v; return this; }
        public Builder confidenceLevel(int v) { this.confidenceLevel = v; return this; }
        public Builder history(int v) { this.history = v; return this; }
        public Builder percentageOfTargetPixels(int v) { this.percentageOfTargetPixels = v; return this; }
        public Builder targetColor(TargetColor v) { this.targetColor = v; return this; }
        public Builder targetIntensity(long v) { this.targetIntensity = v; return this; }
        public Builder centroidLatOffset(double v) { this.centroidLatOffset = v; return this; }
        public Builder centroidLonOffset(double v) { this.centroidLonOffset = v; return this; }
        public Builder centroidHae(double v) { this.centroidHae = v; return this; }
        public Builder bboxTopLeftLatOffset(double v) { this.bboxTopLeftLatOffset = v; return this; }
        public Builder bboxTopLeftLonOffset(double v) { this.bboxTopLeftLonOffset = v; return this; }
        public Builder bboxBottomRightLatOffset(double v) { this.bboxBottomRightLatOffset = v; return this; }
        public Builder bboxBottomRightLonOffset(double v) { this.bboxBottomRightLonOffset = v; return this; }
        public Builder targetLocation(ByteBuffer v) { this.targetLocation = v; return this; }
        public Builder geospatialContourSeries(ByteBuffer v) { this.geospatialContourSeries = v; return this; }
        public Builder centroidPixRow(long v) { this.centroidPixRow = v; return this; }
        public Builder centroidPixCol(long v) { this.centroidPixCol = v; return this; }
        public Builder algorithmId(long v) { this.algorithmId = v; return this; }
        public Builder detectionStatus(int v) { this.detectionStatus = v; return this; }
        public Builder vmask(ByteBuffer v) { this.vmask = v; return this; }
        public Builder vtracker(ByteBuffer v) { this.vtracker = v; return this; }
        public Builder vchip(ByteBuffer v) { this.vchip = v; return this; }
        public Builder vchipSeries(ByteBuffer v) { this.vchipSeries = v; return this; }
        public Builder vobjectSeries(ByteBuffer v) { this.vobjectSeries = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }

        /** Build an immutable {@link VTargetPack}. */
        public VTargetPack build() {
            return new VTargetPack(
                    targetId, centroidPixel, bboxTopLeftPixel, bboxBottomRightPixel,
                    priority, confidenceLevel, history, percentageOfTargetPixels,
                    targetColor, targetIntensity,
                    centroidLatOffset, centroidLonOffset, centroidHae,
                    bboxTopLeftLatOffset, bboxTopLeftLonOffset,
                    bboxBottomRightLatOffset, bboxBottomRightLonOffset,
                    targetLocation, geospatialContourSeries,
                    centroidPixRow, centroidPixCol, algorithmId, detectionStatus,
                    vmask, vtracker, vchip, vchipSeries, vobjectSeries,
                    unknown, fieldErrors
            );
        }
    }
}
