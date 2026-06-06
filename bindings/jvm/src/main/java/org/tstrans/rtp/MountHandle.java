package org.tstrans.rtp;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import org.tstrans.NativeLoader;
import org.tstrans.RtspException;
import org.tstrans.mpegts.AudioStreamHandle;
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
 * concurrently from multiple threads. (Do not, however, race {@link #close()}
 * against a concurrent push — see below.)
 *
 * <p><b>Closing:</b> {@code MountHandle} is {@link AutoCloseable} as a JVM-only
 * lifecycle convenience (tst-py relies on Python GC; the JVM has no refcount-driven
 * native free). {@link #close()} frees ONLY this handle wrapper — the mount itself
 * persists in the server (and keeps fanning out) until {@link RtspServer#stop()} /
 * {@link RtspServer#close()}. Closing while another thread is mid-push is a
 * use-after-free (the standard single-owner-at-close contract); coordinate closes.
 *
 * <p><b>Push errors</b> surface as {@link RtspException} of kind {@code MOUNT}
 * (the failure originates in the mount push path) — this differs from
 * {@link MuxSender}, whose muxer errors are {@code MuxException}.
 */
public final class MountHandle implements AutoCloseable {
    static { NativeLoader.load(); }

    private long handle; // Box<tst_rtp::...::MountHandle>; 0 = closed

    MountHandle(long handle) { this.handle = handle; }

    // ── Identity / introspection ──────────────────────────────────────────
    public String mountPath() { ensureOpen(); return nMountPath(handle); }
    public long peerCount() { ensureOpen(); return nPeerCount(handle); }
    /** {@code "unicast"} / {@code "multicast"} / {@code "unknown"}. */
    public String mountKind() { ensureOpen(); return nMountKind(handle); }
    public MountStats stats() { ensureOpen(); return nStats(handle); }

    // ── Push family — single stream ───────────────────────────────────────
    public void pushVideo(byte[] nal, long pts, boolean keyFrame) throws RtspException {
        ensureOpen(); nPushVideo(handle, nal, pts, keyFrame);
    }
    public void pushKlv(byte[] klv, long pts, int metadataServiceId) throws RtspException {
        ensureOpen(); nPushKlv(handle, klv, pts, metadataServiceId);
    }
    public void pushAudio(byte[] frames, long pts) throws RtspException {
        ensureOpen(); nPushAudio(handle, frames, pts);
    }
    public void pushSubtitle(byte[] payload, long pts) throws RtspException {
        ensureOpen(); nPushSubtitle(handle, pts, payload);
    }

    // ── Push family — handle-targeted ─────────────────────────────────────
    public void pushVideoTo(VideoStreamHandle h, byte[] nal, long pts, boolean keyFrame)
            throws RtspException {
        ensureOpen(); nPushVideoTo(handle, h.raw(), nal, pts, keyFrame);
    }
    public void pushKlvTo(KlvStreamHandle h, byte[] klv, long pts, int metadataServiceId)
            throws RtspException {
        ensureOpen(); nPushKlvTo(handle, h.raw(), klv, pts, metadataServiceId);
    }
    public void pushAudioTo(AudioStreamHandle h, byte[] frames, long pts) throws RtspException {
        ensureOpen(); nPushAudioTo(handle, h.raw(), frames, pts);
    }
    public void pushSubtitleTo(SubtitleStreamHandle h, byte[] payload, long pts)
            throws RtspException {
        ensureOpen(); nPushSubtitleTo(handle, h.raw(), pts, payload);
    }

    // ── Stream-handle accessors (first-of-kind + all-of-kind) ─────────────
    public Optional<VideoStreamHandle> videoHandle() {
        ensureOpen(); long r = nVideoHandle(handle);
        return r < 0 ? Optional.empty() : Optional.of(VideoStreamHandle.fromRaw(r));
    }
    public Optional<KlvStreamHandle> klvHandle() {
        ensureOpen(); long r = nKlvHandle(handle);
        return r < 0 ? Optional.empty() : Optional.of(KlvStreamHandle.fromRaw(r));
    }
    public Optional<AudioStreamHandle> audioHandle() {
        ensureOpen(); long r = nAudioHandle(handle);
        return r < 0 ? Optional.empty() : Optional.of(AudioStreamHandle.fromRaw(r));
    }
    public Optional<SubtitleStreamHandle> subtitleHandle() {
        ensureOpen(); long r = nSubtitleHandle(handle);
        return r < 0 ? Optional.empty() : Optional.of(SubtitleStreamHandle.fromRaw(r));
    }
    public List<VideoStreamHandle> videoHandles() {
        ensureOpen();
        List<VideoStreamHandle> out = new ArrayList<>();
        for (long r : nVideoHandles(handle)) out.add(VideoStreamHandle.fromRaw(r));
        return out;
    }
    public List<KlvStreamHandle> klvHandles() {
        ensureOpen();
        List<KlvStreamHandle> out = new ArrayList<>();
        for (long r : nKlvHandles(handle)) out.add(KlvStreamHandle.fromRaw(r));
        return out;
    }
    public List<AudioStreamHandle> audioHandles() {
        ensureOpen();
        List<AudioStreamHandle> out = new ArrayList<>();
        for (long r : nAudioHandles(handle)) out.add(AudioStreamHandle.fromRaw(r));
        return out;
    }
    public List<SubtitleStreamHandle> subtitleHandles() {
        ensureOpen();
        List<SubtitleStreamHandle> out = new ArrayList<>();
        for (long r : nSubtitleHandles(handle)) out.add(SubtitleStreamHandle.fromRaw(r));
        return out;
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────
    /** Drain buffered TS and broadcast to subscribers. Always safe. */
    public void flush() { ensureOpen(); nFlush(handle); }
    /** Reset all flow counters to zero. */
    public void resetStats() { ensureOpen(); nResetStats(handle); }

    /** Free this handle wrapper (the mount itself persists in the server). Idempotent. */
    @Override
    public void close() {
        if (handle != 0) { nClose(handle); handle = 0; }
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("MountHandle is closed");
    }

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
    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal, long pts,
        boolean keyFrame) throws RtspException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv, long pts,
        int metadataServiceId) throws RtspException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
        long pts) throws RtspException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw, long pts,
        byte[] payload) throws RtspException;
    private static native long nVideoHandle(long handle);
    private static native long nKlvHandle(long handle);
    private static native long nAudioHandle(long handle);
    private static native long nSubtitleHandle(long handle);
    private static native long[] nVideoHandles(long handle);
    private static native long[] nKlvHandles(long handle);
    private static native long[] nAudioHandles(long handle);
    private static native long[] nSubtitleHandles(long handle);
    private static native void nFlush(long handle);
    private static native void nResetStats(long handle);
    private static native void nClose(long handle);
}
