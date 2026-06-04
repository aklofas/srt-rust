package org.tstrans.mpegts;

/**
 * Configuration for {@link Demuxer}. Mirrors {@code tstrans.mpegts.DemuxerConfig} (7 knobs).
 *
 * <p>An immutable value object built via {@link #builder()}. Defaults match
 * {@code tst_core::mpegts::demux::DemuxerConfig::default()}.
 *
 * <p>The cap knobs ({@code pesCapPerPid}, {@code pesCapTotal}, {@code auCellCapPerPid})
 * use {@code 0} as the sentinel meaning "use the Rust default" — the JNI bridge maps
 * {@code 0} to Rust {@code None} (4 MiB / 64 MiB / 1 MiB respectively).
 *
 * <p>{@code klv_link_overrides}/{@code stream_kind_overrides} are Rust-only and
 * deferred — not exposed here.
 */
public final class DemuxerConfig {
    private final StrictMode strictMode;
    private final long pesCapPerPid;       // 0 = use Rust default (4 MiB)
    private final long pesCapTotal;        // 0 = use Rust default (64 MiB)
    private final boolean cfiTolerance;    // default true
    private final Av1CarriageMode av1Carriage;
    private final long auCellCapPerPid;    // 0 = use Rust default (1 MiB)
    private final boolean lenientPsiReassembly;

    private DemuxerConfig(Builder b) {
        this.strictMode = b.strictMode;
        this.pesCapPerPid = b.pesCapPerPid;
        this.pesCapTotal = b.pesCapTotal;
        this.cfiTolerance = b.cfiTolerance;
        this.av1Carriage = b.av1Carriage;
        this.auCellCapPerPid = b.auCellCapPerPid;
        this.lenientPsiReassembly = b.lenientPsiReassembly;
    }

    public static Builder builder() { return new Builder(); }

    // Accessors the Demuxer ctor reads to marshal primitives across the JNI boundary.
    StrictMode strictMode() { return strictMode; }
    long pesCapPerPid() { return pesCapPerPid; }
    long pesCapTotal() { return pesCapTotal; }
    boolean cfiTolerance() { return cfiTolerance; }
    Av1CarriageMode av1Carriage() { return av1Carriage; }
    long auCellCapPerPid() { return auCellCapPerPid; }
    boolean lenientPsiReassembly() { return lenientPsiReassembly; }

    /** Fluent builder for {@link DemuxerConfig}. Defaults match {@code tst_core}'s. */
    public static final class Builder {
        private StrictMode strictMode = StrictMode.OFF;
        private long pesCapPerPid = 0;
        private long pesCapTotal = 0;
        private boolean cfiTolerance = true;
        private Av1CarriageMode av1Carriage = Av1CarriageMode.MPEG2_TS_BINDING;
        private long auCellCapPerPid = 0;
        private boolean lenientPsiReassembly = false;

        public Builder strictMode(StrictMode v) { this.strictMode = v; return this; }
        public Builder pesCapPerPid(long v) { this.pesCapPerPid = v; return this; }
        public Builder pesCapTotal(long v) { this.pesCapTotal = v; return this; }
        public Builder cfiTolerance(boolean v) { this.cfiTolerance = v; return this; }
        public Builder av1Carriage(Av1CarriageMode v) { this.av1Carriage = v; return this; }
        public Builder auCellCapPerPid(long v) { this.auCellCapPerPid = v; return this; }
        public Builder lenientPsiReassembly(boolean v) { this.lenientPsiReassembly = v; return this; }

        public DemuxerConfig build() { return new DemuxerConfig(this); }
    }
}
