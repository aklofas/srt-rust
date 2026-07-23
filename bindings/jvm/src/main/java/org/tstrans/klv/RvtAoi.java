package org.tstrans.klv;

import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0806.4 Table 8-3 Area of Interest Local Set, carried on RVT Tag
 * 13 (repeatable — ST 0806.4-25). Corner 1 = NW (upper-left), Corner 3 = SE
 * (lower-right). Number/both corner pairs/Type are mandatory on encode
 * (ST 0806.4-13..-18); same sentinel-counts-as-present rule as
 * {@link RvtPoi} applies to the corner lat/lon pairs.
 *
 * <p>{@code aoiTypeCode} is the raw ST 0806.4 Table 8-3 Tag 6 wire codepoint;
 * {@link #aoiType()} is the typed accessor, {@code null} for an absent or
 * wire-unknown codepoint (mirrors {@link IcingDetected}).
 *
 * <p>Instances are immutable; use {@link Builder} to construct. The
 * canonical constructor has four consecutive {@code Double} corner
 * parameters and four consecutive {@code String} parameters
 * ({@code text}/{@code sourceId}/{@code label}/{@code operationId}) — easy
 * to silently transpose (e.g. swapping NW/SE corners) in a bare positional
 * call. The Builder's named setters remove that risk; prefer it over the
 * canonical constructor.
 *
 * @param number            AOI Number (Tag 1), mandatory on encode
 * @param cornerLatP1Deg    Point 1 (NW) Latitude in degrees (Tag 2), or {@code null} if absent/sentinel
 * @param cornerLonP1Deg    Point 1 (NW) Longitude in degrees (Tag 3), or {@code null} if absent/sentinel
 * @param cornerLatP3Deg    Point 3 (SE) Latitude in degrees (Tag 4), or {@code null} if absent/sentinel
 * @param cornerLonP3Deg    Point 3 (SE) Longitude in degrees (Tag 5), or {@code null} if absent/sentinel
 * @param aoiTypeCode       raw ST 0806.4 Table 8-3 Tag 6 wire codepoint, or {@code null} if absent
 * @param text              AOI Text Description (Tag 7), or {@code null} if absent
 * @param sourceId          AOI Source ID (Tag 8), or {@code null} if absent
 * @param label             AOI Label (Tag 9), or {@code null} if absent
 * @param operationId       AOI Operation ID (Tag 10), or {@code null} if absent
 * @param sentinelTags      tags whose lat/lon carried the wire "error" indicator on decode
 * @param unknown           tags not modeled above, passed through byte-for-byte
 * @param fieldErrors       per-field decode errors collected instead of aborting the whole set
 */
public record RvtAoi(
        Integer number,
        Double cornerLatP1Deg,
        Double cornerLonP1Deg,
        Double cornerLatP3Deg,
        Double cornerLonP3Deg,
        Integer aoiTypeCode,
        String text,
        String sourceId,
        String label,
        String operationId,
        List<Long> sentinelTags,
        List<KlvUnknownField> unknown,
        List<KlvFieldError> fieldErrors
) {

    /** Compact constructor: make list fields truly immutable + non-null. */
    public RvtAoi {
        sentinelTags = sentinelTags != null
                ? Collections.unmodifiableList(sentinelTags) : Collections.emptyList();
        unknown = unknown != null
                ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null
                ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    /** @return the typed {@link RvtAoiType}, or {@code null} for an absent/wire-unknown codepoint. */
    public RvtAoiType aoiType() {
        return aoiTypeCode == null ? null : RvtAoiType.fromCode(aoiTypeCode);
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Fluent mutable builder for {@link RvtAoi}. No field is mandatory at
     * construction time (encode-side mandatory-item enforcement — Number/
     * both corner pairs/Type — happens in the Rust encoder); list fields
     * default to empty immutable lists.
     */
    public static final class Builder {
        private Integer number;
        private Double cornerLatP1Deg;
        private Double cornerLonP1Deg;
        private Double cornerLatP3Deg;
        private Double cornerLonP3Deg;
        private Integer aoiTypeCode;
        private String text;
        private String sourceId;
        private String label;
        private String operationId;
        private List<Long> sentinelTags = Collections.emptyList();
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        /** Create an empty Builder. */
        public Builder() {}

        public Builder number(int v) { this.number = v; return this; }
        public Builder cornerLatP1Deg(double v) { this.cornerLatP1Deg = v; return this; }
        public Builder cornerLonP1Deg(double v) { this.cornerLonP1Deg = v; return this; }
        public Builder cornerLatP3Deg(double v) { this.cornerLatP3Deg = v; return this; }
        public Builder cornerLonP3Deg(double v) { this.cornerLonP3Deg = v; return this; }
        public Builder aoiTypeCode(int v) { this.aoiTypeCode = v; return this; }
        public Builder text(String v) { this.text = v; return this; }
        public Builder sourceId(String v) { this.sourceId = v; return this; }
        public Builder label(String v) { this.label = v; return this; }
        public Builder operationId(String v) { this.operationId = v; return this; }
        public Builder sentinelTags(List<Long> v) { this.sentinelTags = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }

        /** Build an immutable {@link RvtAoi}. */
        public RvtAoi build() {
            return new RvtAoi(
                    number, cornerLatP1Deg, cornerLonP1Deg, cornerLatP3Deg, cornerLonP3Deg,
                    aoiTypeCode, text, sourceId, label, operationId,
                    sentinelTags, unknown, fieldErrors);
        }
    }
}
