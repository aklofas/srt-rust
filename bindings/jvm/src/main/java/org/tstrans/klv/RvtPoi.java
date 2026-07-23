package org.tstrans.klv;

import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0806.4 Table 8-2 Point of Interest Local Set, carried on RVT Tag
 * 12 (repeatable — ST 0806.4-25). Number/Latitude/Longitude are mandatory
 * on encode (ST 0806.4-08..-10); a {@link #sentinelTags()} entry for tag 2
 * or 3 counts as present even when the paired {@code Double} field is
 * {@code null} (the wire carried the spec's {@code 0x80000000} "error"
 * indicator instead of a value — mirrors {@code UasDatalinkLs.sentinelTags}
 * at the ST 0601 layer).
 *
 * <p>{@code poiTypeCode} is the raw ST 0806.4 Table 8-2 Tag 5 wire codepoint;
 * {@link #poiType()} is the typed accessor, {@code null} for an absent or
 * wire-unknown codepoint (mirrors {@link IcingDetected}).
 *
 * <p>Instances are immutable; use {@link Builder} to construct. The
 * canonical constructor has five consecutive {@code String} parameters
 * ({@code text}/{@code sourceIcon}/{@code sourceId}/{@code label}/
 * {@code operationId}) — easy to silently transpose in a bare positional
 * call. The Builder's named setters remove that risk; prefer it over the
 * canonical constructor.
 *
 * @param number       POI Number (Tag 1), mandatory on encode
 * @param latDeg       POI Latitude in degrees (Tag 2), or {@code null} if absent/sentinel
 * @param lonDeg       POI Longitude in degrees (Tag 3), or {@code null} if absent/sentinel
 * @param altM         POI Altitude in metres MSL (Tag 4), or {@code null} if absent
 * @param poiTypeCode  raw ST 0806.4 Table 8-2 Tag 5 wire codepoint, or {@code null} if absent
 * @param text         POI Text Description (Tag 6), or {@code null} if absent
 * @param sourceIcon   POI Source Icon, MIL-STD-2525B (Tag 7), or {@code null} if absent
 * @param sourceId     POI Source ID (Tag 8), or {@code null} if absent
 * @param label        POI Label (Tag 9), or {@code null} if absent
 * @param operationId  POI Operation ID (Tag 10), or {@code null} if absent
 * @param sentinelTags tags whose lat/lon carried the wire "error" indicator on decode
 * @param unknown      tags not modeled above, passed through byte-for-byte
 * @param fieldErrors  per-field decode errors collected instead of aborting the whole set
 */
public record RvtPoi(
        Integer number,
        Double latDeg,
        Double lonDeg,
        Double altM,
        Integer poiTypeCode,
        String text,
        String sourceIcon,
        String sourceId,
        String label,
        String operationId,
        List<Long> sentinelTags,
        List<KlvUnknownField> unknown,
        List<KlvFieldError> fieldErrors
) {

    /** Compact constructor: make list fields truly immutable + non-null. */
    public RvtPoi {
        sentinelTags = sentinelTags != null
                ? Collections.unmodifiableList(sentinelTags) : Collections.emptyList();
        unknown = unknown != null
                ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null
                ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    /** @return the typed {@link RvtPoiType}, or {@code null} for an absent/wire-unknown codepoint. */
    public RvtPoiType poiType() {
        return poiTypeCode == null ? null : RvtPoiType.fromCode(poiTypeCode);
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Fluent mutable builder for {@link RvtPoi}. No field is mandatory at
     * construction time (encode-side mandatory-item enforcement — Number/
     * Latitude/Longitude — happens in the Rust encoder); list fields default
     * to empty immutable lists.
     */
    public static final class Builder {
        private Integer number;
        private Double latDeg;
        private Double lonDeg;
        private Double altM;
        private Integer poiTypeCode;
        private String text;
        private String sourceIcon;
        private String sourceId;
        private String label;
        private String operationId;
        private List<Long> sentinelTags = Collections.emptyList();
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        /** Create an empty Builder. */
        public Builder() {}

        public Builder number(int v) { this.number = v; return this; }
        public Builder latDeg(double v) { this.latDeg = v; return this; }
        public Builder lonDeg(double v) { this.lonDeg = v; return this; }
        public Builder altM(double v) { this.altM = v; return this; }
        public Builder poiTypeCode(int v) { this.poiTypeCode = v; return this; }
        public Builder text(String v) { this.text = v; return this; }
        public Builder sourceIcon(String v) { this.sourceIcon = v; return this; }
        public Builder sourceId(String v) { this.sourceId = v; return this; }
        public Builder label(String v) { this.label = v; return this; }
        public Builder operationId(String v) { this.operationId = v; return this; }
        public Builder sentinelTags(List<Long> v) { this.sentinelTags = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }

        /** Build an immutable {@link RvtPoi}. */
        public RvtPoi build() {
            return new RvtPoi(
                    number, latDeg, lonDeg, altM, poiTypeCode,
                    text, sourceIcon, sourceId, label, operationId,
                    sentinelTags, unknown, fieldErrors);
        }
    }
}
