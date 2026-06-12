package org.tstrans.srt;

import java.util.Optional;
import org.tstrans.MuxException;
import org.tstrans.NativeLoader;
import org.tstrans.SrtException;
import org.tstrans.mpegts.AudioStreamHandle;
import org.tstrans.mpegts.KlvStreamHandle;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.SubtitleStreamHandle;
import org.tstrans.mpegts.VideoStreamHandle;

/**
 * Single-call convenience wrapper that owns a {@code Muxer} + an
 * {@code SrtTransport}. Construct with a caller-mode SRT URL
 * ({@code srt://host:port?mode=caller&...}) and a built {@link MuxerConfig};
 * push elementary streams via the {@code push*} family and the wrapper assembles
 * MPEG-TS packets and sends them through the SRT socket in one step.
 *
 * <p>Mirrors {@code tstrans.srt.MuxSender}. Wraps
 * {@code tst_pipeline::MuxSender<SrtTransport>}.
 *
 * <p><b>Thread safety:</b> the underlying Rust shell serialises pushes through
 * an internal mutex, so concurrent pushes are safe, but for predictable PTS
 * ordering callers typically push from one thread.
 *
 * <p><b>Closing:</b> use try-with-resources or call {@link #close()} explicitly.
 * After close, further calls throw {@code IllegalStateException}.
 *
 * <p><b>Byte-copy posture (JDK 17):</b> every {@code push*} method copies the
 * supplied {@code byte[]} across the JNI boundary; a zero-copy path (FFM
 * {@code MemorySegment}) is JDK-22+ only and will be added in a future release.
 *
 * <pre>{@code
 * MuxerConfig program = MuxerConfig.builder()
 *     .addVideo(0x101, VideoCodec.H264)
 *     .build();
 * try (MuxSender s = MuxSender.fromUrl("srt://127.0.0.1:7000?mode=caller", program)) {
 *     s.pushVideo(annexBNal, 0L, true);
 * }
 * }</pre>
 */
public final class MuxSender implements AutoCloseable {
    static { NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    /** Package-private constructor from a native handle. */
    MuxSender(long h) { this.handle.set(h); }

    /**
     * Build a {@code MuxSender} targeting {@code url} for the single-program
     * configuration {@code programConfig}. The URL must use {@code mode=caller}
     * (the default when omitted). Opens a libsrt socket, applies query-string
     * options, and blocks on the SRT handshake.
     *
     * @param url           {@code srt://host:port[?key=value&...]} with {@code mode=caller}
     * @param programConfig the muxer program configuration
     * @return a connected {@code MuxSender}
     * @throws SrtException {@code CONFIG_INVALID} if the URL is malformed or uses
     *     a non-caller mode; {@code TIMEOUT} on handshake timeout;
     *     {@code CONNECT_FAILED} on refused/rejected/bad-encryption connections.
     *     A muxer-config rejection surfaces as {@link MuxException}
     *     ({@code CONFIG_INVALID}), thrown from the same native call.
     */
    public static MuxSender fromUrl(String url, MuxerConfig programConfig)
            throws SrtException, MuxException {
        long h = nFromUrl(
            url,
            programConfig.programNumber(), programConfig.pmtPid(), programConfig.pcrPid(),
            programConfig.pcrIntervalMs(), programConfig.psiIntervalMs(),
            programConfig.bufferPackets(), programConfig.av1Carriage().ordinal(),
            programConfig.streamPids(), programConfig.streamKinds(),
            programConfig.streamCodecs(), programConfig.streamTypeCodes(),
            programConfig.streamCarriesPts(),
            programConfig.dataDescBytes(), programConfig.dataDescLens());
        if (h == 0) {
            // nFromUrl throws a pending SrtException/MuxException; JNI re-raises.
            // Unreachable in practice, but satisfies the compiler.
            throw new SrtException(SrtException.Kind.IO, "nFromUrl returned 0 without throwing");
        }
        return new MuxSender(h);
    }

    // ── Push family — single-stream variants ──────────────────────────────

    /**
     * Push one video access unit onto the lone configured video stream.
     * Annex-B framing for H.264/H.265/H.266; raw OBU stream for AV1.
     *
     * @param nal      the access unit bytes
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this access unit is a key frame
     * @throws IllegalStateException if the sender is closed
     * @throws MuxException on muxer/framing failure
     * @throws SrtException on transport failure
     */
    public void pushVideo(byte[] nal, long pts, boolean keyFrame)
            throws MuxException, SrtException {
        ensureOpen();
        nPushVideo(handle.get(), nal, pts, keyFrame);
    }

    /**
     * Push one KLV blob onto the lone configured KLV stream. Pass raw KLV LS
     * bytes — for {@code SYNCHRONOUS_METADATA} streams the muxer auto-wraps the
     * AU-cell header; do not pre-wrap.
     *
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId AU-cell metadata service id (0..=255; default 0)
     * @throws IllegalStateException if the sender is closed
     * @throws IllegalArgumentException if {@code metadataServiceId} is out of 0..=255
     * @throws MuxException on muxer failure
     * @throws SrtException on transport failure
     */
    public void pushKlv(byte[] klv, long pts, int metadataServiceId)
            throws MuxException, SrtException {
        ensureOpen();
        nPushKlv(handle.get(), klv, pts, metadataServiceId);
    }

    /**
     * Push one encoded audio frame onto the lone configured audio stream.
     * {@code frames} is one or more pre-framed audio frames concatenated by the
     * caller (ADTS for AAC, MPEG-2 audio frames for MP2).
     *
     * @param frames the encoded audio bytes
     * @param pts    90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws MuxException on muxer failure
     * @throws SrtException on transport failure
     */
    public void pushAudio(byte[] frames, long pts) throws MuxException, SrtException {
        ensureOpen();
        nPushAudio(handle.get(), frames, pts);
    }

    /**
     * Push one subtitle payload onto the lone configured subtitle stream.
     *
     * @param payload the subtitle access-unit bytes
     * @param pts     90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws MuxException on muxer failure
     * @throws SrtException on transport failure
     */
    public void pushSubtitle(byte[] payload, long pts) throws MuxException, SrtException {
        ensureOpen();
        // Native arg order is (handle, pts, payload); reorder here.
        nPushSubtitle(handle.get(), pts, payload);
    }

    // ── Push family — handle-targeted variants ────────────────────────────

    /**
     * Push one video access unit to a specific configured video stream.
     *
     * @param h        the target stream handle (from {@link #videoHandle()})
     * @param nal      the access unit bytes
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this access unit is a key frame
     * @throws IllegalStateException if the sender is closed
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid for
     *     this muxer
     * @throws MuxException on muxer/framing failure
     */
    public void pushVideoTo(VideoStreamHandle h, byte[] nal, long pts, boolean keyFrame)
            throws MuxException, SrtException {
        ensureOpen();
        nPushVideoTo(handle.get(), h.raw(), nal, pts, keyFrame);
    }

    /**
     * Push one KLV blob to a specific configured KLV stream. Pass raw KLV LS
     * bytes — the muxer auto-wraps the AU-cell header for synchronous-metadata
     * streams; do not pre-wrap.
     *
     * @param h                 the target stream handle (from {@link #klvHandle()})
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId AU-cell metadata service id (0..=255; default 0)
     * @throws IllegalStateException if the sender is closed
     * @throws IllegalArgumentException if {@code metadataServiceId} is out of 0..=255
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid
     * @throws MuxException on muxer failure
     */
    public void pushKlvTo(KlvStreamHandle h, byte[] klv, long pts, int metadataServiceId)
            throws MuxException, SrtException {
        ensureOpen();
        nPushKlvTo(handle.get(), h.raw(), klv, pts, metadataServiceId);
    }

    /**
     * Push one encoded audio frame to a specific configured audio stream.
     *
     * @param h      the target stream handle (from {@link #audioHandle()})
     * @param frames the encoded audio bytes
     * @param pts    90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid
     * @throws MuxException on muxer failure
     */
    public void pushAudioTo(AudioStreamHandle h, byte[] frames, long pts)
            throws MuxException, SrtException {
        ensureOpen();
        nPushAudioTo(handle.get(), h.raw(), frames, pts);
    }

    /**
     * Push one subtitle payload to a specific configured subtitle stream.
     *
     * @param h       the target stream handle (from {@link #subtitleHandle()})
     * @param payload the subtitle access-unit bytes
     * @param pts     90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid
     * @throws MuxException on muxer failure
     */
    public void pushSubtitleTo(SubtitleStreamHandle h, byte[] payload, long pts)
            throws MuxException, SrtException {
        ensureOpen();
        nPushSubtitleTo(handle.get(), h.raw(), pts, payload);
    }

    // ── Handle getters ────────────────────────────────────────────────────

    /**
     * First configured video stream handle, or {@link Optional#empty()} if no
     * video stream is configured.
     *
     * @return the first video handle, if any
     */
    public Optional<VideoStreamHandle> videoHandle() {
        ensureOpen();
        long raw = nVideoHandle(handle.get());
        return raw < 0 ? Optional.empty() : Optional.of(VideoStreamHandle.fromRaw(raw));
    }

    /**
     * First configured KLV stream handle, or {@link Optional#empty()}.
     *
     * @return the first KLV handle, if any
     */
    public Optional<KlvStreamHandle> klvHandle() {
        ensureOpen();
        long raw = nKlvHandle(handle.get());
        return raw < 0 ? Optional.empty() : Optional.of(KlvStreamHandle.fromRaw(raw));
    }

    /**
     * First configured audio stream handle, or {@link Optional#empty()}.
     *
     * @return the first audio handle, if any
     */
    public Optional<AudioStreamHandle> audioHandle() {
        ensureOpen();
        long raw = nAudioHandle(handle.get());
        return raw < 0 ? Optional.empty() : Optional.of(AudioStreamHandle.fromRaw(raw));
    }

    /**
     * First configured subtitle stream handle, or {@link Optional#empty()}.
     *
     * @return the first subtitle handle, if any
     */
    public Optional<SubtitleStreamHandle> subtitleHandle() {
        ensureOpen();
        long raw = nSubtitleHandle(handle.get());
        return raw < 0 ? Optional.empty() : Optional.of(SubtitleStreamHandle.fromRaw(raw));
    }

    // ── Stats + lifecycle ─────────────────────────────────────────────────

    /**
     * Combined {@code (SocketStats, MuxerStats)} snapshot. The socket stats
     * reflect the SRT transport's wire-level counters; the muxer stats reflect
     * the inner muxer's program / packets-emitted totals.
     *
     * @return the combined stats snapshot
     * @throws IllegalStateException if the sender is closed
     */
    public TransportStats stats() {
        ensureOpen();
        return nStats(handle.get());
    }

    /**
     * Close the sender. Best-effort drains any pending bytes, then drops the
     * underlying SRT transport. Idempotent — subsequent calls are no-ops.
     */
    @Override
    public void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    /**
     * Return {@code true} while the sender owns a live transport.
     *
     * @return liveness state of the underlying SRT socket
     */
    public boolean isAlive() {
        if (handle.get() == 0) return false;
        return nIsAlive(handle.get());
    }

    private void ensureOpen() {
        if (handle.get() == 0) throw new IllegalStateException("MuxSender is closed");
    }

    // --- Natives ---

    private static native long nFromUrl(String url, int programNumber, int pmtPid, int pcrPid,
        int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
        int[] streamPids, int[] streamKinds, int[] streamCodecs, int[] streamTypeCodes,
        boolean[] streamCarriesPts,
        byte[] dataDescBytes, int[] dataDescLens) throws SrtException, MuxException;

    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
        throws MuxException, SrtException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
        throws MuxException, SrtException;
    private static native void nPushAudio(long handle, byte[] frames, long pts)
        throws MuxException, SrtException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
        throws MuxException, SrtException;

    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal,
        long pts, boolean keyFrame) throws MuxException, SrtException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv,
        long pts, int metadataServiceId) throws MuxException, SrtException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
        long pts) throws MuxException, SrtException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw, long pts,
        byte[] payload) throws MuxException, SrtException;

    private static native long nVideoHandle(long handle);
    private static native long nKlvHandle(long handle);
    private static native long nAudioHandle(long handle);
    private static native long nSubtitleHandle(long handle);

    private static native TransportStats nStats(long handle);
    private static native void nClose(long handle);
    private static native boolean nIsAlive(long handle);
}
