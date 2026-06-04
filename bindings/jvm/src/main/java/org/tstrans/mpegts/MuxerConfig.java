package org.tstrans.mpegts;

import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

/**
 * Single-program muxer configuration. Mirrors the user-facing shape of
 * {@code tstrans.mpegts.MuxerConfigBuilder} + {@code MuxerProgramConfigBuilder},
 * collapsed to one program. Built via {@link #builder()}; the resulting
 * immutable value exposes package-private parallel-array accessors that the
 * {@link Muxer} constructor marshals across the JNI boundary in a single
 * {@code nOpen} call.
 *
 * <p>The builder performs only cheap Java-side sanity checks (≥1 stream, PID
 * range). Deep validation (PID collisions, PMT-size budget, PCR-on-KLV, sync-KLV
 * needs carriesPts, etc.) runs Rust-side in {@code Muxer::new} and surfaces as
 * {@link org.tstrans.MuxException} ({@code CONFIG_INVALID}).
 *
 * <p><b>Deferred (documented):</b> multi-program configs; per-stream/program
 * descriptors; the {@code *_to(handle, ...)} multi-stream variants; stats /
 * file-sink. DVB subtitle codecs (which need language + page-ID config) are also
 * deferred — {@link Builder#addSubtitle} accepts only the no-config codecs
 * (CEA-708 / WebVTT).
 */
public final class MuxerConfig {
    // Internal stream-kind codes — the parallel-array contract the Muxer JNI
    // nOpen relies on (same 0..3 mapping is hardcoded Rust-side).
    static final int KIND_VIDEO = 0;
    static final int KIND_KLV = 1;
    static final int KIND_AUDIO = 2;
    static final int KIND_SUBTITLE = 3;

    private final int programNumber;
    private final int pmtPid;
    private final int pcrPid;            // -1 = auto-resolve (Rust None)
    private final int pcrIntervalMs;
    private final int psiIntervalMs;
    private final int bufferPackets;
    private final Av1CarriageMode av1Carriage;
    private final int[] streamPids;
    private final int[] streamKinds;     // KIND_* per stream
    private final int[] streamCodecs;    // codec ordinal per kind; -1 for KLV
    private final int[] klvStreamTypes;  // KlvStreamType ordinal for KLV; -1 otherwise
    private final boolean[] klvCarriesPts;

    private MuxerConfig(Builder b) {
        this.programNumber = b.programNumber;
        this.pmtPid = b.pmtPid;
        this.pcrPid = b.pcrPid;
        this.pcrIntervalMs = b.pcrIntervalMs;
        this.psiIntervalMs = b.psiIntervalMs;
        this.bufferPackets = b.bufferPackets;
        this.av1Carriage = b.av1Carriage;
        int n = b.pids.size();
        this.streamPids = new int[n];
        this.streamKinds = new int[n];
        this.streamCodecs = new int[n];
        this.klvStreamTypes = new int[n];
        this.klvCarriesPts = new boolean[n];
        for (int i = 0; i < n; i++) {
            this.streamPids[i] = b.pids.get(i);
            this.streamKinds[i] = b.kinds.get(i);
            this.streamCodecs[i] = b.codecs.get(i);
            this.klvStreamTypes[i] = b.klvTypes.get(i);
            this.klvCarriesPts[i] = b.klvPts.get(i);
        }
    }

    /** Start a new builder. Single program for now (multi-program deferred). */
    public static Builder builder() {
        return new Builder();
    }

    public int programNumber() {
        return programNumber;
    }

    public int streamCount() {
        return streamPids.length;
    }

    // Package-private accessors the Muxer ctor reads to marshal nOpen.
    int pmtPid() { return pmtPid; }
    int pcrPid() { return pcrPid; }
    int pcrIntervalMs() { return pcrIntervalMs; }
    int psiIntervalMs() { return psiIntervalMs; }
    int bufferPackets() { return bufferPackets; }
    Av1CarriageMode av1Carriage() { return av1Carriage; }
    int[] streamPids() { return streamPids; }
    int[] streamKinds() { return streamKinds; }
    int[] streamCodecs() { return streamCodecs; }
    int[] klvStreamTypes() { return klvStreamTypes; }
    boolean[] klvCarriesPts() { return klvCarriesPts; }

    /**
     * Fluent builder for {@link MuxerConfig}. Defaults mirror
     * {@code tst_core::mpegts::mux::MuxerConfig::default()} scalars
     * (pcrIntervalMs=40, psiIntervalMs=100, bufferPackets=10000,
     * av1Carriage=MPEG2_TS_BINDING). {@code programNumber}/{@code pmtPid} are
     * single-valued (one program).
     */
    public static final class Builder {
        private int programNumber = 1;
        private int pmtPid = 0x1000;
        private int pcrPid = -1; // auto-resolve
        private int pcrIntervalMs = 40;
        private int psiIntervalMs = 100;
        private int bufferPackets = 10_000;
        private Av1CarriageMode av1Carriage = Av1CarriageMode.MPEG2_TS_BINDING;
        private final List<Integer> pids = new ArrayList<>();
        private final List<Integer> kinds = new ArrayList<>();
        private final List<Integer> codecs = new ArrayList<>();
        private final List<Integer> klvTypes = new ArrayList<>();
        private final List<Boolean> klvPts = new ArrayList<>();

        public Builder programNumber(int v) { this.programNumber = v; return this; }
        public Builder pmtPid(int v) { this.pmtPid = v; return this; }
        /** Pin the PCR PID to a configured stream's PID. Default: auto-resolve (first video, else KLV/audio). */
        public Builder pcrPid(int v) { this.pcrPid = v; return this; }
        public Builder pcrIntervalMs(int v) { this.pcrIntervalMs = v; return this; }
        public Builder psiIntervalMs(int v) { this.psiIntervalMs = v; return this; }
        public Builder bufferPackets(int v) { this.bufferPackets = v; return this; }
        public Builder av1Carriage(Av1CarriageMode v) {
            this.av1Carriage = Objects.requireNonNull(v, "av1Carriage");
            return this;
        }

        /** Add an H.264/H.265/H.266/AV1 video elementary stream. */
        public Builder addVideo(int pid, VideoCodec codec) {
            Objects.requireNonNull(codec, "codec");
            addStream(pid, KIND_VIDEO, codec.ordinal(), -1, false);
            return this;
        }

        /**
         * Add a KLV metadata stream. Pass raw KLV LS bytes at push time — for
         * {@code SYNCHRONOUS_METADATA} the muxer auto-wraps the AU cell header.
         */
        public Builder addKlv(int pid, KlvStreamType type, boolean carriesPts) {
            Objects.requireNonNull(type, "type");
            addStream(pid, KIND_KLV, -1, type.ordinal(), carriesPts);
            return this;
        }

        /** Add an audio elementary stream. */
        public Builder addAudio(int pid, AudioCodec codec) {
            Objects.requireNonNull(codec, "codec");
            addStream(pid, KIND_AUDIO, codec.ordinal(), -1, false);
            return this;
        }

        /**
         * Add a subtitle elementary stream. Only the no-config codecs
         * {@link SubtitleCodec#CEA708_STANDALONE} and
         * {@link SubtitleCodec#WEBVTT_IN_TS} are supported; the DVB codecs need
         * language + page-ID configuration not yet exposed by the JVM binding
         * (deferred), so passing one throws {@link IllegalArgumentException}.
         */
        public Builder addSubtitle(int pid, SubtitleCodec codec) {
            Objects.requireNonNull(codec, "codec");
            if (codec == SubtitleCodec.DVB_SUBTITLING || codec == SubtitleCodec.DVB_TELETEXT) {
                throw new IllegalArgumentException(
                    "DVB subtitle codecs need language/page-ID config not yet exposed in the "
                        + "JVM binding (deferred); use CEA708_STANDALONE or WEBVTT_IN_TS");
            }
            addStream(pid, KIND_SUBTITLE, codec.ordinal(), -1, false);
            return this;
        }

        private void addStream(int pid, int kind, int codec, int klvType, boolean klvPtsFlag) {
            if (pid < 0x0010 || pid > 0x1FFE) {
                throw new IllegalArgumentException(
                    "pid must be in 0x0010..=0x1FFE, got 0x" + Integer.toHexString(pid));
            }
            pids.add(pid);
            kinds.add(kind);
            codecs.add(codec);
            klvTypes.add(klvType);
            klvPts.add(klvPtsFlag);
        }

        /** Finalize. Requires ≥1 stream. Deep validation happens in {@code Muxer::new}. */
        public MuxerConfig build() {
            if (pids.isEmpty()) {
                throw new IllegalArgumentException("at least one stream is required");
            }
            return new MuxerConfig(this);
        }
    }
}
