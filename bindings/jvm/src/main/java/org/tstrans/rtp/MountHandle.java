package org.tstrans.rtp;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import org.tstrans.NativeLoader;
import org.tstrans.RtspException;
import org.tstrans.mpegts.AudioStreamHandle;
import org.tstrans.mpegts.DataStreamHandle;
import org.tstrans.mpegts.KlvStreamHandle;
import org.tstrans.mpegts.SubtitleStreamHandle;
import org.tstrans.mpegts.VideoStreamHandle;

/**
 * Push surface for one RTSP-server mount, returned by
 * {@link RtspServer#addUnicastMount} / {@link RtspServer#addMulticastMount}.
 * Mirrors tst-py {@code tstrans.rtp.MountHandle}.
 *
 * <p><b>Thread safety:</b> the underlying Rust handle is {@code Arc}-shared and all
 * {@code push*} calls take {@code &self}, so a single {@code MountHandle} may be used
 * concurrently from multiple threads, including a concurrent {@link #close()}:
 * {@code close()} claims the registry id atomically and the leased
 * {@code HandleRegistry} guarantees no use-after-free/double-free — a push that
 * races a {@code close()} either completes or throws a clean
 * {@link IllegalStateException}.
 *
 * <p><b>Closing:</b> {@code MountHandle} is {@link AutoCloseable} as a JVM-only
 * lifecycle convenience (tst-py relies on Python GC; the JVM has no refcount-driven
 * native free). {@link #close()} frees ONLY this handle wrapper — the mount itself
 * persists in the server (and keeps fanning out) until {@link RtspServer#stop()} /
 * {@link RtspServer#close()}.
 *
 * <p><b>Push errors</b> surface as {@link RtspException} of kind {@code MOUNT}
 * (the failure originates in the mount push path) — this differs from
 * {@link MuxSender}, whose muxer errors are {@code MuxException}.
 */
public final class MountHandle implements AutoCloseable {
    static { NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    MountHandle(long h) { this.handle.set(h); }

    // ── Identity / introspection ──────────────────────────────────────────
    public String mountPath() { ensureOpen(); return nMountPath(handle.get()); }
    public long peerCount() { ensureOpen(); return nPeerCount(handle.get()); }
    /** {@code "unicast"} / {@code "multicast"} / {@code "unknown"}. */
    public String mountKind() { ensureOpen(); return nMountKind(handle.get()); }
    public MountStats stats() { ensureOpen(); return nStats(handle.get()); }

    // ── Push family — single stream ───────────────────────────────────────
    public void pushVideo(byte[] nal, long pts, boolean keyFrame) throws RtspException {
        ensureOpen(); nPushVideo(handle.get(), nal, pts, keyFrame);
    }
    public void pushKlv(byte[] klv, long pts, int metadataServiceId) throws RtspException {
        ensureOpen(); nPushKlv(handle.get(), klv, pts, metadataServiceId);
    }
    public void pushAudio(byte[] frames, long pts) throws RtspException {
        ensureOpen(); nPushAudio(handle.get(), frames, pts);
    }
    public void pushSubtitle(byte[] payload, long pts) throws RtspException {
        ensureOpen(); nPushSubtitle(handle.get(), pts, payload);
    }
    /**
     * Push one private-data payload onto the lone configured data stream
     * (pass-through: no AU-cell wrap, no framing — {@code data} lands verbatim as
     * the PES payload). {@code pts} drives PSI/PCR pacing and is written into the
     * PES header only when the stream was configured with {@code carriesPts = true}.
     */
    public void pushData(byte[] data, long pts) throws RtspException {
        ensureOpen(); nPushData(handle.get(), data, pts);
    }

    // ── Push family — handle-targeted ─────────────────────────────────────
    public void pushVideoTo(VideoStreamHandle h, byte[] nal, long pts, boolean keyFrame)
            throws RtspException {
        ensureOpen(); nPushVideoTo(handle.get(), h.raw(), nal, pts, keyFrame);
    }
    public void pushKlvTo(KlvStreamHandle h, byte[] klv, long pts, int metadataServiceId)
            throws RtspException {
        ensureOpen(); nPushKlvTo(handle.get(), h.raw(), klv, pts, metadataServiceId);
    }
    public void pushAudioTo(AudioStreamHandle h, byte[] frames, long pts) throws RtspException {
        ensureOpen(); nPushAudioTo(handle.get(), h.raw(), frames, pts);
    }
    public void pushSubtitleTo(SubtitleStreamHandle h, byte[] payload, long pts)
            throws RtspException {
        ensureOpen(); nPushSubtitleTo(handle.get(), h.raw(), pts, payload);
    }
    /**
     * Push one private-data payload to a specific configured data stream. Same
     * pass-through and PTS semantics as {@link #pushData}. An invalid handle
     * raises {@link RtspException} of kind {@code MOUNT}.
     */
    public void pushDataTo(DataStreamHandle h, byte[] data, long pts) throws RtspException {
        ensureOpen(); nPushDataTo(handle.get(), h.raw(), data, pts);
    }

    // ── Stream-handle accessors (first-of-kind + all-of-kind) ─────────────
    public Optional<VideoStreamHandle> videoHandle() {
        ensureOpen(); long r = nVideoHandle(handle.get());
        return r < 0 ? Optional.empty() : Optional.of(VideoStreamHandle.fromRaw(r));
    }
    public Optional<KlvStreamHandle> klvHandle() {
        ensureOpen(); long r = nKlvHandle(handle.get());
        return r < 0 ? Optional.empty() : Optional.of(KlvStreamHandle.fromRaw(r));
    }
    public Optional<AudioStreamHandle> audioHandle() {
        ensureOpen(); long r = nAudioHandle(handle.get());
        return r < 0 ? Optional.empty() : Optional.of(AudioStreamHandle.fromRaw(r));
    }
    public Optional<SubtitleStreamHandle> subtitleHandle() {
        ensureOpen(); long r = nSubtitleHandle(handle.get());
        return r < 0 ? Optional.empty() : Optional.of(SubtitleStreamHandle.fromRaw(r));
    }
    public Optional<DataStreamHandle> dataHandle() {
        ensureOpen(); long r = nDataHandle(handle.get());
        return r < 0 ? Optional.empty() : Optional.of(DataStreamHandle.fromRaw(r));
    }
    public List<VideoStreamHandle> videoHandles() {
        ensureOpen();
        List<VideoStreamHandle> out = new ArrayList<>();
        for (long r : nVideoHandles(handle.get())) out.add(VideoStreamHandle.fromRaw(r));
        return out;
    }
    public List<KlvStreamHandle> klvHandles() {
        ensureOpen();
        List<KlvStreamHandle> out = new ArrayList<>();
        for (long r : nKlvHandles(handle.get())) out.add(KlvStreamHandle.fromRaw(r));
        return out;
    }
    public List<AudioStreamHandle> audioHandles() {
        ensureOpen();
        List<AudioStreamHandle> out = new ArrayList<>();
        for (long r : nAudioHandles(handle.get())) out.add(AudioStreamHandle.fromRaw(r));
        return out;
    }
    public List<SubtitleStreamHandle> subtitleHandles() {
        ensureOpen();
        List<SubtitleStreamHandle> out = new ArrayList<>();
        for (long r : nSubtitleHandles(handle.get())) out.add(SubtitleStreamHandle.fromRaw(r));
        return out;
    }
    public List<DataStreamHandle> dataHandles() {
        ensureOpen();
        List<DataStreamHandle> out = new ArrayList<>();
        for (long r : nDataHandles(handle.get())) out.add(DataStreamHandle.fromRaw(r));
        return out;
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────
    /** Drain buffered TS and broadcast to subscribers. Always safe. */
    public void flush() { ensureOpen(); nFlush(handle.get()); }
    /** Reset all flow counters to zero. */
    public void resetStats() { ensureOpen(); nResetStats(handle.get()); }

    /** Free this handle wrapper (the mount itself persists in the server). Idempotent. */
    @Override
    public void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    private void ensureOpen() {
        if (handle.get() == 0) throw new IllegalStateException("MountHandle is closed");
    }

    /**
     * Test-only: the raw native handle, for routing a panic through the real
     * {@code REGISTRY_MOUNT} in {@code RtspPanicPoisoningTest} (proves the mount
     * mutators are wired to {@code with_mount_poisoning}). Package-private,
     * mirrors the {@code *ForTest} convention in {@code Klv}.
     */
    long nativeHandleForTest() { return handle.get(); }

    // --- Natives ---
    private static native String nMountPath(long handle);
    private static native long nPeerCount(long handle);
    private static native String nMountKind(long handle);
    private static native MountStats nStats(long handle);
    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
        throws RtspException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
        throws RtspException;
    private static native void nPushAudio(long handle, byte[] frames, long pts) throws RtspException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
        throws RtspException;
    private static native void nPushData(long handle, byte[] data, long pts) throws RtspException;
    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal, long pts,
        boolean keyFrame) throws RtspException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv, long pts,
        int metadataServiceId) throws RtspException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
        long pts) throws RtspException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw, long pts,
        byte[] payload) throws RtspException;
    private static native void nPushDataTo(long handle, long streamHandleRaw, byte[] data,
        long pts) throws RtspException;
    private static native long nVideoHandle(long handle);
    private static native long nKlvHandle(long handle);
    private static native long nAudioHandle(long handle);
    private static native long nSubtitleHandle(long handle);
    private static native long nDataHandle(long handle);
    private static native long[] nVideoHandles(long handle);
    private static native long[] nKlvHandles(long handle);
    private static native long[] nAudioHandles(long handle);
    private static native long[] nSubtitleHandles(long handle);
    private static native long[] nDataHandles(long handle);
    private static native void nFlush(long handle);
    private static native void nResetStats(long handle);
    private static native void nClose(long handle);
}
