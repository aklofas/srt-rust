package org.tstrans.pipeline;

import java.util.List;
import org.tstrans.DemuxException;
import org.tstrans.mpegts.DemuxerConfig;
import org.tstrans.mpegts.DemuxerStats;

/**
 * Byte-feeding KLV↔video pairer. Feed TS bytes, collect {@link PairerOutput}s.
 * Mirrors {@code tstrans.pipeline.Pairer}; wraps the core
 * {@code tst_pipeline::ext::pairing::PairingDemuxer}. Single-threaded — the
 * consumer owns concurrency (the {@code org.tstrans.mpegts.Demuxer} contract).
 *
 * <pre>{@code
 * try (Pairer p = new Pairer(0x101, 0x102)) {
 *     for (PairerOutput o : p.feed(tsBytes)) { ... }
 *     for (PairerOutput o : p.flush()) { ... }
 * }
 * }</pre>
 */
public final class Pairer implements AutoCloseable {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<PairingDemuxer> pointer; 0 = closed

    /** Construct for the given video + KLV PIDs with default configs. */
    public Pairer(int videoPid, int klvPid) {
        this.handle = nOpen(videoPid, klvPid);
    }

    /**
     * Construct with explicit configs.
     *
     * <p>Ordinal contract: the {@code strict}/{@code av1} ints passed to
     * {@code nOpenWithConfig} are the Java enum ORDINALS — the Rust side maps by
     * ordinal in the SAME declaration order as {@link org.tstrans.mpegts.StrictMode}
     * / {@link org.tstrans.mpegts.Av1CarriageMode}.
     */
    public Pairer(int videoPid, int klvPid, PairingDemuxerConfig config) {
        if (config == null) throw new IllegalArgumentException("config must be non-null");
        PairerConfig pc = config.pairer();
        boolean buffered = pc.mode() instanceof PairerMode.Buffered;
        long maxLagNanos = buffered ? ((PairerMode.Buffered) pc.mode()).maxLag().toNanos() : 0L;
        DemuxerConfig dx = config.demuxer();
        this.handle = nOpenWithConfig(videoPid, klvPid,
            buffered, maxLagNanos, pc.tolerance().toNanos(),
            pc.maxBufferedKlv(), pc.maxBufferedVideo(), pc.linkKlvToVideo(),
            dx != null,
            dx != null ? dx.strictMode().ordinal() : 0,
            dx != null ? dx.pesCapPerPid() : 0L,
            dx != null ? dx.pesCapTotal() : 0L,
            dx != null && dx.cfiTolerance(),
            dx != null ? dx.av1Carriage().ordinal() : 0,
            dx != null ? dx.auCellCapPerPid() : 0L,
            dx != null && dx.lenientPsiReassembly());
    }

    /** Feed TS bytes; returns the pairing outputs produced. @throws DemuxException on non-conformant input. */
    @SuppressWarnings("unchecked")
    public List<PairerOutput> feed(byte[] bytes) throws DemuxException {
        ensureOpen();
        return (List<PairerOutput>) nFeed(handle, bytes);
    }

    /** Drain end-of-stream state (load-bearing in Buffered mode; no-op in Realtime). */
    @SuppressWarnings("unchecked")
    public List<PairerOutput> flush() {
        ensureOpen();
        return (List<PairerOutput>) nFlush(handle);
    }

    /** Pairing counters. */
    public PairerStats stats() {
        ensureOpen();
        return nStats(handle);
    }

    /** Underlying demuxer counters. */
    public DemuxerStats demuxerStats() {
        ensureOpen();
        return nDemuxerStats(handle);
    }

    /** Reset the pairing counters (not demuxer stats). */
    public void resetStats() {
        ensureOpen();
        nResetStats(handle);
    }

    @Override
    public void close() {
        if (handle != 0) { nClose(handle); handle = 0; }
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("Pairer is closed");
    }

    private static native long nOpen(int videoPid, int klvPid);
    private static native long nOpenWithConfig(int videoPid, int klvPid,
        boolean buffered, long maxLagNanos, long toleranceNanos,
        long maxBufferedKlv, long maxBufferedVideo, boolean linkKlvToVideo,
        boolean hasDemuxerConfig, int strict, long pesCapPerPid, long pesCapTotal,
        boolean cfi, int av1, long auCellCap, boolean lenientPsi);
    private static native Object nFeed(long handle, byte[] bytes) throws DemuxException;
    private static native Object nFlush(long handle);
    private static native PairerStats nStats(long handle);
    private static native DemuxerStats nDemuxerStats(long handle);
    private static native void nResetStats(long handle);
    private static native void nClose(long handle);
}
