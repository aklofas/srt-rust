package org.tstrans.mpegts;

import org.tstrans.MuxException;

/**
 * Stateful MPEG-TS multiplexer (sender side). Mirrors {@code tstrans.mpegts.Muxer}.
 * Configure with a {@link MuxerConfig}, push encoded access units via the
 * {@code push*} family, then drain assembled TS packets with {@link #pull}.
 * The muxer is deterministic — output is a function of inputs only.
 *
 * <p>One muxer is single-threaded; the consumer owns concurrency (spec §5.5).
 * Although the underlying Rust {@code Muxer} owns no OS handles, the native
 * allocation is reclaimed by {@link #close()} — use try-with-resources.
 *
 * <pre>{@code
 * MuxerConfig cfg = MuxerConfig.builder()
 *     .addVideo(0x1011, VideoCodec.H264)
 *     .build();
 * byte[] out = new byte[8192];
 * try (Muxer m = new Muxer(cfg)) {
 *     m.pushVideo(annexBNal, 0L, true);
 *     int n;
 *     while ((n = m.pull(out)) > 0) { sink.write(out, 0, n); }
 * }
 * }</pre>
 *
 * <p><b>PTS units:</b> every {@code pts} argument is a 90&nbsp;kHz tick count
 * (ISO/IEC 13818-1 presentation timestamp). <b>Draining:</b> call {@link #pull}
 * in a loop until it returns 0 — a single call writes at most {@code out.length /
 * 188 * 188} bytes.
 */
public final class Muxer implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<tst_core::mpegts::mux::Muxer> pointer; 0 = closed

    /**
     * Build a muxer from {@code cfg}. The whole single-program config is
     * marshalled across one {@code nOpen} call.
     *
     * @throws MuxException ({@code CONFIG_INVALID}) if {@code Muxer::new} rejects
     *     the config (PID collisions, PMT over budget, sync-KLV without PTS, …).
     */
    public Muxer(MuxerConfig cfg) throws MuxException {
        this.handle = nOpen(
            cfg.programNumber(), cfg.pmtPid(), cfg.pcrPid(),
            cfg.pcrIntervalMs(), cfg.psiIntervalMs(), cfg.bufferPackets(),
            cfg.av1Carriage().ordinal(),
            cfg.streamPids(), cfg.streamKinds(), cfg.streamCodecs(),
            cfg.klvStreamTypes(), cfg.klvCarriesPts());
    }

    /**
     * Push one H.264/H.265/H.266 access unit (Annex-B framing) or AV1 OBU
     * bitstream onto the lone configured video stream.
     *
     * @param nal      the access unit bytes (Annex-B start-code prefixed for H.26x)
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this AU is a random-access point (sets the
     *                 adaptation-field {@code random_access_indicator}; forces a PCR
     *                 when coincident with the PCR PID)
     * @throws MuxException {@code INPUT_MALFORMED} (not Annex-B), {@code INVALID_USAGE}
     *     (zero or &gt;1 video stream — configure exactly one), or {@code BACKPRESSURE}
     *     (queue full — drain via {@link #pull}).
     */
    public void pushVideo(byte[] nal, long pts, boolean keyFrame) throws MuxException {
        ensureOpen();
        nPushVideo(handle, nal, pts, keyFrame);
    }

    /**
     * Push one KLV local-set onto the lone configured KLV stream. Pass raw KLV
     * LS bytes — for {@code SYNCHRONOUS_METADATA} streams the muxer auto-prepends
     * the 5-byte {@code Metadata_AU_cell} header (do NOT pre-wrap).
     *
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId metadata service selector (0 for the common single-service case)
     * @throws MuxException {@code INPUT_MALFORMED} (too large for one PES),
     *     {@code INVALID_USAGE} (zero or &gt;1 KLV stream), or {@code BACKPRESSURE}.
     */
    public void pushKlv(byte[] klv, long pts, int metadataServiceId) throws MuxException {
        ensureOpen();
        nPushKlv(handle, klv, pts, metadataServiceId);
    }

    /**
     * Push one encoded audio frame (codec-native framing) onto the lone
     * configured audio stream.
     *
     * @param frames codec-native audio frame bytes (ADTS for AAC, raw for MP2/AC-3/LATM)
     * @param pts    90&nbsp;kHz presentation timestamp
     * @throws MuxException {@code INPUT_MALFORMED}, {@code INVALID_USAGE}
     *     (zero or &gt;1 audio stream), or {@code BACKPRESSURE}.
     */
    public void pushAudio(byte[] frames, long pts) throws MuxException {
        ensureOpen();
        nPushAudio(handle, frames, pts);
    }

    /**
     * Push one subtitle PES payload onto the lone configured subtitle stream.
     * Note the {@code (pts, payload)} argument order (matches {@code tst_core}).
     *
     * @param pts     90&nbsp;kHz presentation timestamp
     * @param payload subtitle PES payload bytes
     * @throws MuxException {@code INPUT_MALFORMED}, {@code INVALID_USAGE}
     *     (zero or &gt;1 subtitle stream), or {@code BACKPRESSURE}.
     */
    public void pushSubtitle(long pts, byte[] payload) throws MuxException {
        ensureOpen();
        nPushSubtitle(handle, pts, payload);
    }

    /**
     * Drain ready TS packets into {@code out}. Returns the number of bytes
     * written — always a multiple of 188 — or 0 when the queue is empty or
     * {@code out.length < 188}. Call in a loop until it returns 0.
     */
    public int pull(byte[] out) {
        ensureOpen();
        return nPull(handle, out);
    }

    /** Number of 188-byte TS packets currently queued awaiting {@link #pull}. */
    public long pendingPackets() {
        ensureOpen();
        return nPending(handle);
    }

    /** Configured queue capacity in 188-byte TS packets (snapshot of {@code bufferPackets}). */
    public long capacityPackets() {
        ensureOpen();
        return nCapacity(handle);
    }

    @Override
    public void close() {
        if (handle != 0) { nClose(handle); handle = 0; }
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("Muxer is closed");
    }

    private static native long nOpen(int programNumber, int pmtPid, int pcrPid,
            int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
            int[] streamPids, int[] streamKinds, int[] streamCodecs,
            int[] klvStreamTypes, boolean[] klvCarriesPts) throws MuxException;
    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
            throws MuxException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
            throws MuxException;
    private static native void nPushAudio(long handle, byte[] frames, long pts) throws MuxException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
            throws MuxException;
    private static native int nPull(long handle, byte[] out);
    private static native long nPending(long handle);
    private static native long nCapacity(long handle);
    private static native void nClose(long handle);
}
