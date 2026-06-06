package org.tstrans.rtp;

import java.util.Optional;
import org.tstrans.MuxException;
import org.tstrans.NativeLoader;
import org.tstrans.RtpException;
import org.tstrans.mpegts.AudioStreamHandle;
import org.tstrans.mpegts.KlvStreamHandle;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.SubtitleStreamHandle;
import org.tstrans.mpegts.VideoStreamHandle;

/**
 * Single-call convenience wrapper that owns a {@code Muxer} + an RTP
 * {@code RtpTransport}. Construct with an {@code rtp://host:port} URL and a built
 * {@link MuxerConfig}; push elementary streams via the {@code push*} family and
 * the wrapper assembles MPEG-TS packets and sends them over RTP/UDP in one step.
 *
 * <p>Mirrors {@code tstrans.rtp.MuxSender}. Wraps
 * {@code tst_pipeline::MuxSender<tst_rtp::RtpTransport>}.
 *
 * <p><b>Thread safety:</b> the underlying Rust shell serialises pushes through an
 * internal mutex, so concurrent pushes are safe, but for predictable PTS ordering
 * callers typically push from one thread.
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
 *     .programNumber(1).pmtPid(0x1000)
 *     .addVideo(0x1011, VideoCodec.H264)
 *     .build();
 * try (MuxSender s = MuxSender.fromUrl("rtp://127.0.0.1:5004", program)) {
 *     s.pushVideo(annexBNal, 0L, true);
 * }
 * }</pre>
 */
public final class MuxSender implements AutoCloseable {
    static { NativeLoader.load(); }

    /** Default UDP datagram payload size (7 × 188 TS packets). Matches tst-py. */
    public static final int DEFAULT_PKT_SIZE = 1316;

    private long handle; // Box<tst_pipeline::MuxSender<RtpTransport>>; 0 = closed

    MuxSender(long handle) { this.handle = handle; }

    /**
     * Build a {@code MuxSender} targeting {@code url} with the default packet size
     * ({@value #DEFAULT_PKT_SIZE}).
     *
     * @param url           {@code rtp://host:port}
     * @param programConfig the muxer program configuration
     * @return an open {@code MuxSender}
     * @throws RtpException {@code TRANSPORT} on URL-parse / socket-bind failure
     * @throws MuxException {@code CONFIG_INVALID} if the muxer rejects the config
     */
    public static MuxSender fromUrl(String url, MuxerConfig programConfig)
            throws RtpException, MuxException {
        return fromUrl(url, programConfig, DEFAULT_PKT_SIZE);
    }

    /**
     * Build a {@code MuxSender} targeting {@code url} with an explicit
     * {@code pktSize}.
     *
     * @param url           {@code rtp://host:port}
     * @param programConfig the muxer program configuration
     * @param pktSize       the UDP datagram payload size; must be &ge; 0
     * @return an open {@code MuxSender}
     * @throws RtpException {@code TRANSPORT} on URL-parse / socket-bind failure
     * @throws MuxException {@code CONFIG_INVALID} if the muxer rejects the config
     * @throws IllegalArgumentException if {@code pktSize} is negative
     */
    public static MuxSender fromUrl(String url, MuxerConfig programConfig, int pktSize)
            throws RtpException, MuxException {
        if (pktSize < 0) throw new IllegalArgumentException("pktSize must be >= 0: " + pktSize);
        long h = nFromUrl(
            url,
            programConfig.programNumber(), programConfig.pmtPid(), programConfig.pcrPid(),
            programConfig.pcrIntervalMs(), programConfig.psiIntervalMs(),
            programConfig.bufferPackets(), programConfig.av1Carriage().ordinal(),
            programConfig.streamPids(), programConfig.streamKinds(),
            programConfig.streamCodecs(), programConfig.klvStreamTypes(),
            programConfig.klvCarriesPts(),
            pktSize);
        if (h == 0) {
            // nFromUrl throws a pending RtpException/MuxException; JNI re-raises.
            // Unreachable in practice, but satisfies the compiler.
            throw new RtpException(RtpException.Kind.TRANSPORT,
                "nFromUrl returned 0 without throwing");
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
     * @throws RtpException on transport failure
     */
    public void pushVideo(byte[] nal, long pts, boolean keyFrame)
            throws MuxException, RtpException {
        ensureOpen();
        nPushVideo(handle, nal, pts, keyFrame);
    }

    /**
     * Push one KLV blob onto the lone configured KLV stream. Pass raw KLV LS
     * bytes — the muxer auto-wraps the AU-cell header for synchronous-metadata
     * streams; do not pre-wrap.
     *
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId AU-cell metadata service id (0..=255; default 0)
     * @throws IllegalStateException if the sender is closed
     * @throws IllegalArgumentException if {@code metadataServiceId} is out of 0..=255
     * @throws MuxException on muxer failure
     * @throws RtpException on transport failure
     */
    public void pushKlv(byte[] klv, long pts, int metadataServiceId)
            throws MuxException, RtpException {
        ensureOpen();
        nPushKlv(handle, klv, pts, metadataServiceId);
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
     * @throws RtpException on transport failure
     */
    public void pushAudio(byte[] frames, long pts) throws MuxException, RtpException {
        ensureOpen();
        nPushAudio(handle, frames, pts);
    }

    /**
     * Push one subtitle payload onto the lone configured subtitle stream.
     *
     * @param payload the subtitle access-unit bytes
     * @param pts     90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws MuxException on muxer failure
     * @throws RtpException on transport failure
     */
    public void pushSubtitle(byte[] payload, long pts) throws MuxException, RtpException {
        ensureOpen();
        nPushSubtitle(handle, pts, payload);
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
     * @throws RtpException on transport failure
     * @throws MuxException on muxer/framing failure (incl. an invalid handle)
     */
    public void pushVideoTo(VideoStreamHandle h, byte[] nal, long pts, boolean keyFrame)
            throws MuxException, RtpException {
        ensureOpen();
        nPushVideoTo(handle, h.raw(), nal, pts, keyFrame);
    }

    /**
     * Push one KLV blob to a specific configured KLV stream. Pass raw KLV LS
     * bytes — the muxer auto-wraps the AU-cell header; do not pre-wrap.
     *
     * @param h                 the target stream handle (from {@link #klvHandle()})
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId AU-cell metadata service id (0..=255; default 0)
     * @throws IllegalStateException if the sender is closed
     * @throws IllegalArgumentException if {@code metadataServiceId} is out of 0..=255
     * @throws RtpException on transport failure
     * @throws MuxException on muxer failure (incl. an invalid handle)
     */
    public void pushKlvTo(KlvStreamHandle h, byte[] klv, long pts, int metadataServiceId)
            throws MuxException, RtpException {
        ensureOpen();
        nPushKlvTo(handle, h.raw(), klv, pts, metadataServiceId);
    }

    /**
     * Push one encoded audio frame to a specific configured audio stream.
     *
     * @param h      the target stream handle (from {@link #audioHandle()})
     * @param frames the encoded audio bytes
     * @param pts    90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws RtpException on transport failure
     * @throws MuxException on muxer failure (incl. an invalid handle)
     */
    public void pushAudioTo(AudioStreamHandle h, byte[] frames, long pts)
            throws MuxException, RtpException {
        ensureOpen();
        nPushAudioTo(handle, h.raw(), frames, pts);
    }

    /**
     * Push one subtitle payload to a specific configured subtitle stream.
     *
     * @param h       the target stream handle (from {@link #subtitleHandle()})
     * @param payload the subtitle access-unit bytes
     * @param pts     90&nbsp;kHz presentation timestamp
     * @throws IllegalStateException if the sender is closed
     * @throws RtpException on transport failure
     * @throws MuxException on muxer failure (incl. an invalid handle)
     */
    public void pushSubtitleTo(SubtitleStreamHandle h, byte[] payload, long pts)
            throws MuxException, RtpException {
        ensureOpen();
        nPushSubtitleTo(handle, h.raw(), pts, payload);
    }

    // ── Handle getters ────────────────────────────────────────────────────

    /** First configured video stream handle, or {@link Optional#empty()}. */
    public Optional<VideoStreamHandle> videoHandle() {
        ensureOpen();
        long raw = nVideoHandle(handle);
        return raw < 0 ? Optional.empty() : Optional.of(VideoStreamHandle.fromRaw(raw));
    }

    /** First configured KLV stream handle, or {@link Optional#empty()}. */
    public Optional<KlvStreamHandle> klvHandle() {
        ensureOpen();
        long raw = nKlvHandle(handle);
        return raw < 0 ? Optional.empty() : Optional.of(KlvStreamHandle.fromRaw(raw));
    }

    /** First configured audio stream handle, or {@link Optional#empty()}. */
    public Optional<AudioStreamHandle> audioHandle() {
        ensureOpen();
        long raw = nAudioHandle(handle);
        return raw < 0 ? Optional.empty() : Optional.of(AudioStreamHandle.fromRaw(raw));
    }

    /** First configured subtitle stream handle, or {@link Optional#empty()}. */
    public Optional<SubtitleStreamHandle> subtitleHandle() {
        ensureOpen();
        long raw = nSubtitleHandle(handle);
        return raw < 0 ? Optional.empty() : Optional.of(SubtitleStreamHandle.fromRaw(raw));
    }

    // ── Stats + lifecycle ─────────────────────────────────────────────────

    /**
     * Combined {@code (SocketStats, MuxerStats)} snapshot. The socket stats reflect
     * the RTP transport's wire-level counters; the muxer stats reflect the inner
     * muxer's program / packets-emitted totals.
     *
     * @return the combined stats snapshot
     * @throws IllegalStateException if the sender is closed
     */
    public TransportStats stats() {
        ensureOpen();
        return nStats(handle);
    }

    /** Close the sender. Best-effort drains pending bytes, then drops the RTP
     * transport. Idempotent. */
    @Override
    public void close() {
        if (handle != 0) {
            nClose(handle);
            handle = 0;
        }
    }

    /** Whether the sender owns a live transport. */
    public boolean isAlive() {
        if (handle == 0) return false;
        return nIsAlive(handle);
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("MuxSender is closed");
    }

    // --- Natives ---

    private static native long nFromUrl(String url, int programNumber, int pmtPid, int pcrPid,
        int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
        int[] streamPids, int[] streamKinds, int[] streamCodecs, int[] klvStreamTypes,
        boolean[] klvCarriesPts, int pktSize) throws RtpException, MuxException;

    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
        throws MuxException, RtpException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
        throws MuxException, RtpException;
    private static native void nPushAudio(long handle, byte[] frames, long pts)
        throws MuxException, RtpException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
        throws MuxException, RtpException;

    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal,
        long pts, boolean keyFrame) throws MuxException, RtpException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv,
        long pts, int metadataServiceId) throws MuxException, RtpException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
        long pts) throws MuxException, RtpException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw, long pts,
        byte[] payload) throws MuxException, RtpException;

    private static native long nVideoHandle(long handle);
    private static native long nKlvHandle(long handle);
    private static native long nAudioHandle(long handle);
    private static native long nSubtitleHandle(long handle);

    private static native TransportStats nStats(long handle);
    private static native void nClose(long handle);
    private static native boolean nIsAlive(long handle);
}
