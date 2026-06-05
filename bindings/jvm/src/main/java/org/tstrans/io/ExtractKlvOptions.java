package org.tstrans.io;

import org.tstrans.mpegts.DemuxerConfig;

/**
 * Options for {@link Io#extractKlv}. Mirrors the keyword arguments of tst-py's
 * {@code tstrans.io.extract_klv}. Construct via {@link #builder()}; defaults match
 * tst-py ({@code withPts=false, parsed=false, skipUnknown=true, skipMalformed=false}).
 * The {@code config} field is the {@link DemuxerConfig} to use, or {@code null} for
 * the default.
 */
public record ExtractKlvOptions(
    boolean withPts,
    boolean parsed,
    boolean skipUnknown,
    boolean skipMalformed,
    DemuxerConfig config
) {
    /** Defaults matching tst-py. */
    public static ExtractKlvOptions defaults() {
        return new ExtractKlvOptions(false, false, true, false, null);
    }

    public static Builder builder() {
        return new Builder();
    }

    /** Fluent builder; mirrors {@link DemuxerConfig.Builder}'s style. */
    public static final class Builder {
        private boolean withPts = false;
        private boolean parsed = false;
        private boolean skipUnknown = true;
        private boolean skipMalformed = false;
        private DemuxerConfig config = null;

        public Builder withPts(boolean v) { this.withPts = v; return this; }
        public Builder parsed(boolean v) { this.parsed = v; return this; }
        public Builder skipUnknown(boolean v) { this.skipUnknown = v; return this; }
        public Builder skipMalformed(boolean v) { this.skipMalformed = v; return this; }
        public Builder config(DemuxerConfig v) { this.config = v; return this; }

        public ExtractKlvOptions build() {
            return new ExtractKlvOptions(withPts, parsed, skipUnknown, skipMalformed, config);
        }
    }
}
