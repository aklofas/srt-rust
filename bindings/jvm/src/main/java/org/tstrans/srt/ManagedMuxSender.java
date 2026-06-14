package org.tstrans.srt;

import java.util.Optional;
import org.tstrans.MuxException;
import org.tstrans.NativeLoader;
import org.tstrans.SrtException;
import org.tstrans.mpegts.AudioStreamHandle;
import org.tstrans.mpegts.DataStreamHandle;
import org.tstrans.mpegts.KlvStreamHandle;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.SubtitleStreamHandle;
import org.tstrans.mpegts.VideoStreamHandle;

/**
 * Single-call convenience wrapper that owns a {@code Muxer} plus a managed
 * (auto-reconnect) SRT transport. Construct with a caller-mode SRT URL
 * ({@code srt://host:port?mode=caller&...}) and a built {@link MuxerConfig};
 * push elementary streams via the {@code push*} family and the wrapper assembles
 * MPEG-TS packets and sends them through the SRT socket in one step. On any
 * Broken/Closed event the captured (URL, socket config) is replayed through the
 * reconnect factory under the configured {@link ReconnectPolicy}.
 *
 * <p>Mirrors {@code tstrans.srt.ManagedMuxSender}. Wraps
 * {@code tst_pipeline::MuxSender<ManagedTransport<SrtTransport>>}.
 *
 * <p><b>Thread safety:</b> the underlying Rust shell serialises pushes through
 * an internal mutex, so concurrent pushes are safe, but for predictable PTS
 * ordering callers typically push from one thread.
 *
 * <p><b>Closing:</b> use try-with-resources or call {@link #close()} explicitly.
 * After close, further calls throw {@code IllegalStateException}.
 *
 * <p><b>Stats:</b> {@link #stats()} returns the combined
 * {@code (SocketStats, MuxerStats)} {@link TransportStats}; there is NO
 * {@code srtStats()} on this wrapper. {@link #reconnectAttempts()} counts every
 * reconnect-factory invocation since construction (an ATTEMPT counter).
 *
 * <p><b>Byte-copy posture (JDK 17):</b> every {@code push*} method copies the
 * supplied {@code byte[]} across the JNI boundary; a zero-copy path (FFM
 * {@code MemorySegment}) is JDK-22+ only and will be added in a future release.
 *
 * <pre>{@code
 * MuxerConfig program = MuxerConfig.builder()
 *     .addVideo(0x101, VideoCodec.H264)
 *     .build();
 * try (ManagedMuxSender s = ManagedMuxSender.fromUrl(
 *         "srt://127.0.0.1:7000?mode=caller", program)) {
 *     s.pushVideo(annexBNal, 0L, true);
 * }
 * }</pre>
 */
public final class ManagedMuxSender implements AutoCloseable {
    static { NativeLoader.load(); }

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    /** Package-private constructor from a native handle. */
    ManagedMuxSender(long h) { this.handle.set(h); }

    /**
     * Build a {@code ManagedMuxSender} targeting {@code url} for the single-program
     * configuration {@code programConfig} with the default {@link ReconnectPolicy}.
     *
     * @param url           {@code srt://host:port[?key=value&...]} with {@code mode=caller}
     * @param programConfig the muxer program configuration
     * @return a connected {@code ManagedMuxSender}
     * @throws SrtException {@code CONFIG_INVALID} if the URL is malformed or uses
     *     a non-caller mode; {@code CONNECT_FAILED} on initial-connect failure
     * @throws MuxException {@code CONFIG_INVALID} if the muxer config is rejected
     */
    public static ManagedMuxSender fromUrl(String url, MuxerConfig programConfig)
            throws SrtException, MuxException {
        return fromUrl(url, programConfig, null);
    }

    /**
     * Build a {@code ManagedMuxSender} targeting {@code url} for the single-program
     * configuration {@code programConfig}.
     *
     * <p>The URL must use {@code mode=caller} (the default when omitted). The
     * {@code policy} (or {@link ReconnectPolicy#defaults()} when {@code null})
     * governs the initial connect and every subsequent reconnect.
     *
     * @param url           {@code srt://host:port[?key=value&...]} with {@code mode=caller}
     * @param programConfig the muxer program configuration
     * @param policy        reconnect tuning; {@code null} applies the defaults
     * @return a connected {@code ManagedMuxSender}
     * @throws SrtException {@code CONFIG_INVALID} if the URL is malformed or uses
     *     a non-caller mode; {@code CONNECT_FAILED} on initial-connect failure
     * @throws MuxException {@code CONFIG_INVALID} if the muxer config is rejected
     */
    public static ManagedMuxSender fromUrl(String url, MuxerConfig programConfig, ReconnectPolicy policy)
            throws SrtException, MuxException {
        PolicyArgs p = PolicyArgs.from(policy);
        long h = nFromUrl(
            url,
            programConfig.programNumber(), programConfig.pmtPid(), programConfig.pcrPid(),
            programConfig.pcrIntervalMs(), programConfig.psiIntervalMs(),
            programConfig.bufferPackets(), programConfig.av1Carriage().ordinal(),
            programConfig.streamPids(), programConfig.streamKinds(),
            programConfig.streamCodecs(), programConfig.streamTypeCodes(),
            programConfig.streamCarriesPts(),
            programConfig.dataDescBytes(), programConfig.dataDescLens(),
            p.maxAttemptsPresent(), p.maxAttempts(),
            p.backoffKind(), p.backoffBaseMs(), p.backoffMaxMs(),
            p.gapBufferCapacity(), p.overflowPolicy());
        if (h == 0) {
            // nFromUrl throws a pending SrtException/MuxException; JNI re-raises.
            // Unreachable in practice, but satisfies the compiler.
            throw new SrtException(SrtException.Kind.IO, "nFromUrl returned 0 without throwing");
        }
        return new ManagedMuxSender(h);
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

    /**
     * Push one private-data payload onto the lone configured data stream.
     * Pass-through: the muxer applies no AU-cell wrap and no framing (UNLIKE
     * {@link #pushKlv}) — {@code data} lands verbatim as the PES payload, and
     * one push produces exactly one PES packet on stream_id {@code 0xBD}
     * ({@code private_stream_1}). {@code pts} is written into the PES header
     * only when the stream was configured with {@code carriesPts = true}, but
     * it ALWAYS drives PSI/PCR pacing.
     *
     * @param data raw payload bytes (caller's framing convention; at most
     *             65527 bytes with PTS, 65532 without)
     * @param pts  90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws MuxException {@code INPUT_MALFORMED} (payload over the PES
     *     ceiling), or {@code INVALID_USAGE} (zero data streams, or &gt;1 —
     *     ambiguous, use {@link #pushDataTo})
     * @throws SrtException on transport failure
     */
    public void pushData(byte[] data, long pts) throws MuxException, SrtException {
        ensureOpen();
        nPushData(handle.get(), data, pts);
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
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid
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

    /**
     * Push one private-data payload to a specific configured data stream. Same
     * pass-through and PTS semantics as {@link #pushData}.
     *
     * @param h    the target stream handle (from {@link #dataHandle()})
     * @param data raw payload bytes (caller's framing convention; at most
     *             65527 bytes with PTS, 65532 without)
     * @param pts  90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws SrtException {@code CONFIG_INVALID} if the handle is invalid
     * @throws MuxException on muxer failure
     */
    public void pushDataTo(DataStreamHandle h, byte[] data, long pts)
            throws MuxException, SrtException {
        ensureOpen();
        nPushDataTo(handle.get(), h.raw(), data, pts);
    }

    // ── Handle getters ────────────────────────────────────────────────────

    /**
     * First configured video stream handle, or {@link Optional#empty()}.
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

    /**
     * First configured data stream handle, or {@link Optional#empty()}.
     *
     * @return the first data handle, if any
     */
    public Optional<DataStreamHandle> dataHandle() {
        ensureOpen();
        long raw = nDataHandle(handle.get());
        return raw < 0 ? Optional.empty() : Optional.of(DataStreamHandle.fromRaw(raw));
    }

    // ── Stats + lifecycle ─────────────────────────────────────────────────

    /**
     * Combined {@code (SocketStats, MuxerStats)} snapshot. The socket stats
     * reflect the SRT transport's wire-level counters (zeros while mid-reconnect);
     * the muxer stats reflect the inner muxer's program / packets-emitted totals.
     *
     * @return the combined stats snapshot
     * @throws IllegalStateException if the sender is closed
     */
    public TransportStats stats() {
        ensureOpen();
        return nStats(handle.get());
    }

    /**
     * Total number of times the reconnect factory has been invoked since
     * construction. {@code 0} means the initial connect is still live; rising
     * values mean the inner SRT socket has been rebuilt (or a rebuild attempt
     * failed and was retried).
     *
     * @return the reconnect-attempt counter
     * @throws IllegalStateException if the sender is closed
     */
    public long reconnectAttempts() {
        ensureOpen();
        return nReconnectAttempts(handle.get());
    }

    /**
     * Close the sender. Best-effort drains any pending bytes, then drops the
     * underlying managed transport. Idempotent — subsequent calls are no-ops.
     */
    @Override
    public void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    /**
     * Return {@code true} while the sender owns a live transport.
     *
     * @return liveness state of the underlying managed transport
     */
    public boolean isAlive() {
        if (handle.get() == 0) return false;
        return nIsAlive(handle.get());
    }

    private void ensureOpen() {
        if (handle.get() == 0) throw new IllegalStateException("ManagedMuxSender is closed");
    }

    // --- Natives ---

    private static native long nFromUrl(String url, int programNumber, int pmtPid, int pcrPid,
        int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
        int[] streamPids, int[] streamKinds, int[] streamCodecs, int[] streamTypeCodes,
        boolean[] streamCarriesPts,
        byte[] dataDescBytes, int[] dataDescLens,
        boolean maxAttemptsPresent, int maxAttempts,
        int backoffKind, long backoffBaseMs, long backoffMaxMs,
        int gapBufferCapacity, int overflowPolicy) throws SrtException, MuxException;

    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
        throws MuxException, SrtException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
        throws MuxException, SrtException;
    private static native void nPushAudio(long handle, byte[] frames, long pts)
        throws MuxException, SrtException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
        throws MuxException, SrtException;
    private static native void nPushData(long handle, byte[] data, long pts)
        throws MuxException, SrtException;

    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal,
        long pts, boolean keyFrame) throws MuxException, SrtException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv,
        long pts, int metadataServiceId) throws MuxException, SrtException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
        long pts) throws MuxException, SrtException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw, long pts,
        byte[] payload) throws MuxException, SrtException;
    private static native void nPushDataTo(long handle, long streamHandleRaw, byte[] data,
        long pts) throws MuxException, SrtException;

    private static native long nVideoHandle(long handle);
    private static native long nKlvHandle(long handle);
    private static native long nAudioHandle(long handle);
    private static native long nSubtitleHandle(long handle);
    private static native long nDataHandle(long handle);

    private static native TransportStats nStats(long handle);
    private static native long nReconnectAttempts(long handle);
    private static native void nClose(long handle);
    private static native boolean nIsAlive(long handle);
}
