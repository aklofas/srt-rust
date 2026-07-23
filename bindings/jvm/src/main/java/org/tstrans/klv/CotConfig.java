package org.tstrans.klv;

/**
 * Configuration for MISB ST 0805.1 KLV -&gt; Cursor-on-Target (CoT) conversion
 * ({@link Klv#platformPositionXml} / {@link Klv#sensorPointOfInterestXml}).
 * Construct via {@link #builder()} or use {@link #defaults()}; defaults match
 * {@code tst_core}'s {@code CotConfig::default()}.
 *
 * @param platformType CoT {@code type} for the Platform Position event. ST
 *     0805.1 §5 gives {@code a-f-A-M-F} as the fixed-wing example and
 *     requires it be overridable per platform (rotary-wing, manned pods,
 *     ...). The Sensor Point of Interest event's {@code type} is fixed at
 *     {@code b-m-p-s-p-i} by the spec and is not configurable.
 * @param updateIntervalUs {@code stale = time + updateIntervalUs}. ST 0805.1
 *     defines {@code stale} as "time of next message" but gives no concrete
 *     interval — this default is an implementation choice, not a spec value.
 * @param producer XML attribute <em>name</em> stamped verbatim into
 *     {@code <detail><_flow-tags_ .../>}. It is a Name production (an
 *     attribute name, not a value): neither validated nor escaped, so an
 *     invalid value produces malformed XML.
 * @param geoidUndulationM geoid undulation (HAE − MSL) applied when only an
 *     MSL-referenced altitude tag is available. {@code null} emits the MSL
 *     value as-is.
 * @param how CoT {@code how} attribute. ST 0805.1 §5 fixes this at
 *     {@code m-p} (machine-passed) for both event types.
 */
public record CotConfig(
        String platformType,
        long updateIntervalUs,
        String producer,
        Double geoidUndulationM,
        String how
) {
    /** Defaults matching {@code tst_core::klv::st0805::CotConfig::default()}. */
    public static CotConfig defaults() {
        return new CotConfig("a-f-A-M-F", 5_000_000L, "ST0601CoT", null, "m-p");
    }

    public static Builder builder() {
        return new Builder();
    }

    /** Fluent builder; mirrors {@link org.tstrans.io.ExtractKlvOptions.Builder}'s style. */
    public static final class Builder {
        private String platformType = "a-f-A-M-F";
        private long updateIntervalUs = 5_000_000L;
        private String producer = "ST0601CoT";
        private Double geoidUndulationM = null;
        private String how = "m-p";

        public Builder platformType(String v) { this.platformType = v; return this; }
        public Builder updateIntervalUs(long v) { this.updateIntervalUs = v; return this; }

        /**
         * XML attribute <em>name</em> stamped verbatim into
         * {@code <detail><_flow-tags_ .../>} — a Name production (an
         * attribute name, not a value). Neither validated nor escaped; an
         * invalid value produces malformed XML.
         */
        public Builder producer(String v) { this.producer = v; return this; }
        public Builder geoidUndulationM(Double v) { this.geoidUndulationM = v; return this; }
        public Builder how(String v) { this.how = v; return this; }

        public CotConfig build() {
            return new CotConfig(platformType, updateIntervalUs, producer, geoidUndulationM, how);
        }
    }
}
