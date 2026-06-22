package org.tstrans.mpegts;

import java.io.IOException;
import java.nio.file.Path;
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

    private final java.util.concurrent.atomic.AtomicLong handle =
        new java.util.concurrent.atomic.AtomicLong(); // registry key; 0 = closed

    /**
     * Build a muxer from {@code cfg}. The whole single-program config is
     * marshalled across one {@code nOpen} call.
     *
     * @throws MuxException ({@code CONFIG_INVALID}) if {@code Muxer::new} rejects
     *     the config (PID collisions, PMT over budget, sync-KLV without PTS, …).
     */
    public Muxer(MuxerConfig cfg) throws MuxException {
        this.handle.set(nOpen(
            cfg.programNumber(), cfg.pmtPid(), cfg.pcrPid(),
            cfg.pcrIntervalMs(), cfg.psiIntervalMs(), cfg.bufferPackets(),
            cfg.av1Carriage().ordinal(),
            cfg.streamPids(), cfg.streamKinds(), cfg.streamCodecs(),
            cfg.streamTypeCodes(), cfg.streamCarriesPts(),
            cfg.dataDescBytes(), cfg.dataDescLens()));
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
        nPushVideo(handle.get(), nal, pts, keyFrame);
    }

    /**
     * Push one already-carried on-wire video access unit onto the lone configured
     * video stream. Emits {@code wire} verbatim — no Annex-B start-code check, no
     * AV1 OBU re-wrapping. Use this for byte-faithful AV1 transmux: configure the
     * destination muxer to the same {@link Av1CarriageMode} as the source, then
     * feed {@link DemuxEvent.Video#raw()} here instead of
     * {@link #pushVideo(byte[], long, boolean)} — {@code pushVideo} would re-wrap
     * the wire bytes and corrupt an AV1 binding-mode stream (AV1-01).
     *
     * <p>For elementary OBU / Annex-B input (encoding directly, not re-muxing),
     * use {@link #pushVideo} instead.
     *
     * @param wire     the on-wire access unit bytes (e.g. {@code Video.raw()} from
     *                 the demuxer, already in the target carriage framing)
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this AU is a random-access point
     * @throws MuxException {@code INVALID_USAGE} (zero or &gt;1 video stream —
     *     configure exactly one), or {@code BACKPRESSURE} (queue full — drain via
     *     {@link #pull}).
     */
    public void pushVideoWire(byte[] wire, long pts, boolean keyFrame) throws MuxException {
        ensureOpen();
        nPushVideoWire(handle.get(), wire, pts, keyFrame);
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
        nPushKlv(handle.get(), klv, pts, metadataServiceId);
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
        nPushAudio(handle.get(), frames, pts);
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
        nPushSubtitle(handle.get(), pts, payload);
    }

    /**
     * Push one private-data payload onto the lone configured data stream.
     * Pass-through: the muxer applies no AU-cell wrap and no framing (UNLIKE
     * {@link #pushKlv}) — {@code data} lands verbatim as the PES payload, and
     * one push produces exactly one PES packet on stream_id {@code 0xBD}
     * ({@code private_stream_1}).
     *
     * <p>{@code pts} is written into the PES header only when the stream was
     * configured with {@code carriesPts = true}, but it ALWAYS drives PSI/PCR
     * pacing. A {@code carriesPts = false} stream re-demuxes with
     * {@code pts == 0} (this library's no-PTS substitute).
     *
     * <p>Data is the first stream kind with a handle-targeted push on the
     * offline {@code Muxer} (see {@link #pushDataTo}); the other kinds'
     * {@code *To} variants remain sender-only/deferred.
     *
     * @param data raw payload bytes (caller's framing convention; at most
     *             65527 bytes with PTS, 65532 without)
     * @param pts  90&nbsp;kHz presentation timestamp
     * @throws MuxException {@code INPUT_MALFORMED} (payload over the PES
     *     ceiling), {@code INVALID_USAGE} (zero data streams, or &gt;1 —
     *     ambiguous, use {@link #pushDataTo}), or {@code BACKPRESSURE}.
     */
    public void pushData(byte[] data, long pts) throws MuxException {
        ensureOpen();
        nPushData(handle.get(), data, pts);
    }

    /**
     * Push one private-data payload onto a specific data stream. Same
     * pass-through and PTS semantics as {@link #pushData}; obtain {@code h}
     * from {@link #dataHandles()} / {@link #dataStreamHandle(int)}.
     *
     * @param h    handle of the target data stream (from this muxer)
     * @param data raw payload bytes (caller's framing convention; at most
     *             65527 bytes with PTS, 65532 without)
     * @param pts  90&nbsp;kHz presentation timestamp
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range
     *     handle), {@code INPUT_MALFORMED} (payload over the PES ceiling), or
     *     {@code BACKPRESSURE}.
     */
    public void pushDataTo(DataStreamHandle h, byte[] data, long pts) throws MuxException {
        ensureOpen();
        nPushDataTo(handle.get(), h.raw(), data, pts);
    }

    /**
     * Push one H.264/H.265/H.266 access unit (Annex-B framing) or AV1 OBU
     * bitstream onto a specific configured video stream.
     *
     * @param h        handle of the target video stream (from this muxer)
     * @param nal      the access unit bytes (Annex-B start-code prefixed for H.26x)
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this AU is a random-access point
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle),
     *     {@code INPUT_MALFORMED} (not Annex-B for H.26x), or {@code BACKPRESSURE}.
     */
    public void pushVideoTo(VideoStreamHandle h, byte[] nal, long pts, boolean keyFrame)
            throws MuxException {
        ensureOpen();
        nPushVideoTo(handle.get(), h.raw(), nal, pts, keyFrame);
    }

    /**
     * Push one already-carried on-wire video access unit onto a specific configured
     * video stream. Emits {@code wire} verbatim — no Annex-B start-code check, no
     * AV1 OBU re-wrapping. Use for byte-faithful AV1 transmux targeting a specific
     * stream; obtain {@code h} from {@link #videoStreamHandle(int)}.
     *
     * @param h        handle of the target video stream (from this muxer)
     * @param wire     the on-wire access unit bytes
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param keyFrame whether this AU is a random-access point
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle) or
     *     {@code BACKPRESSURE}.
     */
    public void pushVideoWireTo(VideoStreamHandle h, byte[] wire, long pts, boolean keyFrame)
            throws MuxException {
        ensureOpen();
        nPushVideoWireTo(handle.get(), h.raw(), wire, pts, keyFrame);
    }

    /**
     * Push one H.264/H.265/H.266 access unit (Annex-B framing) or AV1 OBU
     * bitstream onto a specific configured video stream, with an explicit decode
     * timestamp for B-frame reordered streams. The PES header will carry
     * {@code PTS_DTS_flags = '11'}; the demuxed {@link DemuxEvent.Video#dts()}
     * will be non-null and equal to the {@code dts} value supplied here.
     *
     * @param h        handle of the target video stream (from this muxer)
     * @param nal      the access unit bytes (Annex-B start-code prefixed for H.26x)
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param dts      90&nbsp;kHz decode timestamp (must be &le; {@code pts})
     * @param keyFrame whether this AU is a random-access point
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle),
     *     {@code INPUT_MALFORMED} (not Annex-B for H.26x), or {@code BACKPRESSURE}.
     */
    public void pushVideoToWithDts(VideoStreamHandle h, byte[] nal, long pts, long dts,
            boolean keyFrame) throws MuxException {
        ensureOpen();
        nPushVideoToWithDts(handle.get(), h.raw(), nal, pts, dts, keyFrame);
    }

    /**
     * Push one already-carried on-wire video access unit onto a specific configured
     * video stream, with an explicit decode timestamp for B-frame reordered streams.
     * Emits {@code wire} verbatim — no Annex-B start-code check, no AV1 OBU
     * re-wrapping. Use for byte-faithful AV1 transmux with DTS preservation; obtain
     * {@code h} from {@link #videoStreamHandle(int)}.
     *
     * @param h        handle of the target video stream (from this muxer)
     * @param wire     the on-wire access unit bytes
     * @param pts      90&nbsp;kHz presentation timestamp
     * @param dts      90&nbsp;kHz decode timestamp (must be &le; {@code pts})
     * @param keyFrame whether this AU is a random-access point
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle) or
     *     {@code BACKPRESSURE}.
     */
    public void pushVideoWireToWithDts(VideoStreamHandle h, byte[] wire, long pts, long dts,
            boolean keyFrame) throws MuxException {
        ensureOpen();
        nPushVideoWireToWithDts(handle.get(), h.raw(), wire, pts, dts, keyFrame);
    }

    /**
     * Push one KLV local-set onto a specific configured KLV stream. Same
     * pass-through and AU-cell-wrap semantics as {@link #pushKlv}; obtain {@code h}
     * from {@link #klvStreamHandle(int)}.
     *
     * @param h                 handle of the target KLV stream (from this muxer)
     * @param klv               raw KLV LS bytes
     * @param pts               90&nbsp;kHz presentation timestamp
     * @param metadataServiceId metadata service selector (0 for the common case)
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle),
     *     {@code INPUT_MALFORMED} (too large for one PES), or {@code BACKPRESSURE}.
     */
    public void pushKlvTo(KlvStreamHandle h, byte[] klv, long pts, int metadataServiceId)
            throws MuxException {
        ensureOpen();
        nPushKlvTo(handle.get(), h.raw(), klv, pts, metadataServiceId);
    }

    /**
     * Push one encoded audio frame buffer onto a specific configured audio stream.
     * Same semantics as {@link #pushAudio}; obtain {@code h} from
     * {@link #audioStreamHandle(int)}.
     *
     * @param h      handle of the target audio stream (from this muxer)
     * @param frames codec-native audio frame bytes
     * @param pts    90&nbsp;kHz presentation timestamp
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle),
     *     {@code INPUT_MALFORMED}, or {@code BACKPRESSURE}.
     */
    public void pushAudioTo(AudioStreamHandle h, byte[] frames, long pts) throws MuxException {
        ensureOpen();
        nPushAudioTo(handle.get(), h.raw(), frames, pts);
    }

    /**
     * Push one subtitle PES payload onto a specific configured subtitle stream.
     * Same semantics as {@link #pushSubtitle}; obtain {@code h} from
     * {@link #subtitleStreamHandle(int)}.
     *
     * @param h       handle of the target subtitle stream (from this muxer)
     * @param pts     90&nbsp;kHz presentation timestamp
     * @param payload subtitle PES payload bytes
     * @throws MuxException {@code INVALID_USAGE} (malformed or out-of-range handle),
     *     {@code INPUT_MALFORMED}, or {@code BACKPRESSURE}.
     */
    public void pushSubtitleTo(SubtitleStreamHandle h, long pts, byte[] payload)
            throws MuxException {
        ensureOpen();
        nPushSubtitleTo(handle.get(), h.raw(), pts, payload);
    }

    /** All configured video-stream handles, in {@code addVideo} order.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.List<VideoStreamHandle> videoHandles() {
        ensureOpen();
        long[] raws = nVideoHandles(handle.get());
        java.util.List<VideoStreamHandle> out = new java.util.ArrayList<>(raws.length);
        for (long r : raws) out.add(VideoStreamHandle.fromRaw(r));
        return java.util.List.copyOf(out);
    }

    /** The {@code index}-th video-stream handle, or empty if out of range.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.Optional<VideoStreamHandle> videoStreamHandle(int index) {
        java.util.List<VideoStreamHandle> hs = videoHandles();
        return (index >= 0 && index < hs.size())
            ? java.util.Optional.of(hs.get(index)) : java.util.Optional.empty();
    }

    /** All configured audio-stream handles, in {@code addAudio} order.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.List<AudioStreamHandle> audioHandles() {
        ensureOpen();
        long[] raws = nAudioHandles(handle.get());
        java.util.List<AudioStreamHandle> out = new java.util.ArrayList<>(raws.length);
        for (long r : raws) out.add(AudioStreamHandle.fromRaw(r));
        return java.util.List.copyOf(out);
    }

    /** The {@code index}-th audio-stream handle, or empty if out of range.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.Optional<AudioStreamHandle> audioStreamHandle(int index) {
        java.util.List<AudioStreamHandle> hs = audioHandles();
        return (index >= 0 && index < hs.size())
            ? java.util.Optional.of(hs.get(index)) : java.util.Optional.empty();
    }

    /** All configured KLV-stream handles, in {@code addKlv} order.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.List<KlvStreamHandle> klvHandles() {
        ensureOpen();
        long[] raws = nKlvHandles(handle.get());
        java.util.List<KlvStreamHandle> out = new java.util.ArrayList<>(raws.length);
        for (long r : raws) out.add(KlvStreamHandle.fromRaw(r));
        return java.util.List.copyOf(out);
    }

    /** The {@code index}-th KLV-stream handle, or empty if out of range.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.Optional<KlvStreamHandle> klvStreamHandle(int index) {
        java.util.List<KlvStreamHandle> hs = klvHandles();
        return (index >= 0 && index < hs.size())
            ? java.util.Optional.of(hs.get(index)) : java.util.Optional.empty();
    }

    /** All configured subtitle-stream handles, in {@code addSubtitle} order.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.List<SubtitleStreamHandle> subtitleHandles() {
        ensureOpen();
        long[] raws = nSubtitleHandles(handle.get());
        java.util.List<SubtitleStreamHandle> out = new java.util.ArrayList<>(raws.length);
        for (long r : raws) out.add(SubtitleStreamHandle.fromRaw(r));
        return java.util.List.copyOf(out);
    }

    /** The {@code index}-th subtitle-stream handle, or empty if out of range.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.Optional<SubtitleStreamHandle> subtitleStreamHandle(int index) {
        java.util.List<SubtitleStreamHandle> hs = subtitleHandles();
        return (index >= 0 && index < hs.size())
            ? java.util.Optional.of(hs.get(index)) : java.util.Optional.empty();
    }

    /** All configured data-stream handles, in {@code addData} order.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.List<DataStreamHandle> dataHandles() {
        ensureOpen();
        long[] raws = nDataHandles(handle.get());
        java.util.List<DataStreamHandle> out = new java.util.ArrayList<>(raws.length);
        for (long r : raws) out.add(DataStreamHandle.fromRaw(r));
        return java.util.List.copyOf(out);
    }

    /** The {@code index}-th data-stream handle, or empty if out of range.
     *  @throws IllegalStateException if the muxer is closed */
    public java.util.Optional<DataStreamHandle> dataStreamHandle(int index) {
        java.util.List<DataStreamHandle> hs = dataHandles();
        return (index >= 0 && index < hs.size())
            ? java.util.Optional.of(hs.get(index)) : java.util.Optional.empty();
    }

    /**
     * Drain ready TS packets into {@code out}. Returns the number of bytes
     * written — always a multiple of 188 — or 0 when the queue is empty or
     * {@code out.length < 188}. Call in a loop until it returns 0.
     */
    public int pull(byte[] out) {
        ensureOpen();
        return nPull(handle.get(), out);
    }

    /** Number of 188-byte TS packets currently queued awaiting {@link #pull}. */
    public long pendingPackets() {
        ensureOpen();
        return nPending(handle.get());
    }

    /** Configured queue capacity in 188-byte TS packets (snapshot of {@code bufferPackets}). */
    public long capacityPackets() {
        ensureOpen();
        return nCapacity(handle.get());
    }

    /**
     * Open {@code path} for writing and return a draining {@link MuxerFileSink}
     * that drains pending TS packets to the file's buffered output stream after each
     * {@code push*} call and on close (bytes reach the file when the buffer fills or
     * on close — not necessarily synchronously per push; see {@link MuxerFileSink}).
     * Mirrors tst-py's {@code Muxer.write_file(path)}. The muxer is borrowed
     * (reusable after the sink closes).
     */
    public MuxerFileSink writeFile(Path path) throws IOException {
        ensureOpen();
        return new MuxerFileSink(this, path, false);
    }

    /**
     * Like {@link #writeFile(Path)} but, when {@code atomic} is true, writes via a
     * {@code *.partial} temp in the destination's directory and promotes it to
     * {@code path} only after {@link MuxerFileSink#commit()} on the success path
     * (mirrors tst-py's {@code Muxer.write_file(path, atomic=True)}; see
     * {@link MuxerFileSink} for the {@code commit()} contract).
     */
    public MuxerFileSink writeFile(Path path, boolean atomic) throws IOException {
        ensureOpen();
        return new MuxerFileSink(this, path, atomic);
    }

    @Override
    public void close() {
        long h = handle.getAndSet(0);
        if (h != 0) nClose(h);
    }

    private void ensureOpen() {
        if (handle.get() == 0) throw new IllegalStateException("Muxer is closed");
    }

    private static native long nOpen(int programNumber, int pmtPid, int pcrPid,
            int pcrIntervalMs, int psiIntervalMs, int bufferPackets, int av1Carriage,
            int[] streamPids, int[] streamKinds, int[] streamCodecs,
            int[] streamTypeCodes, boolean[] streamCarriesPts,
            byte[] dataDescBytes, int[] dataDescLens) throws MuxException;
    private static native void nPushVideo(long handle, byte[] nal, long pts, boolean keyFrame)
            throws MuxException;
    private static native void nPushVideoWire(long handle, byte[] wire, long pts, boolean keyFrame)
            throws MuxException;
    private static native void nPushKlv(long handle, byte[] klv, long pts, int metadataServiceId)
            throws MuxException;
    private static native void nPushAudio(long handle, byte[] frames, long pts) throws MuxException;
    private static native void nPushSubtitle(long handle, long pts, byte[] payload)
            throws MuxException;
    private static native void nPushData(long handle, byte[] data, long pts) throws MuxException;
    private static native void nPushDataTo(long handle, long streamHandleRaw, byte[] data, long pts)
            throws MuxException;
    private static native void nPushVideoTo(long handle, long streamHandleRaw, byte[] nal,
            long pts, boolean keyFrame) throws MuxException;
    private static native void nPushVideoWireTo(long handle, long streamHandleRaw, byte[] wire,
            long pts, boolean keyFrame) throws MuxException;
    private static native void nPushVideoToWithDts(long handle, long streamHandleRaw, byte[] nal,
            long pts, long dts, boolean keyFrame) throws MuxException;
    private static native void nPushVideoWireToWithDts(long handle, long streamHandleRaw,
            byte[] wire, long pts, long dts, boolean keyFrame) throws MuxException;
    private static native void nPushKlvTo(long handle, long streamHandleRaw, byte[] klv,
            long pts, int metadataServiceId) throws MuxException;
    private static native void nPushAudioTo(long handle, long streamHandleRaw, byte[] frames,
            long pts) throws MuxException;
    private static native void nPushSubtitleTo(long handle, long streamHandleRaw,
            long pts, byte[] payload) throws MuxException;
    private static native long[] nVideoHandles(long handle);
    private static native long[] nAudioHandles(long handle);
    private static native long[] nKlvHandles(long handle);
    private static native long[] nSubtitleHandles(long handle);
    private static native long[] nDataHandles(long handle);
    private static native int nPull(long handle, byte[] out);
    private static native long nPending(long handle);
    private static native long nCapacity(long handle);
    private static native void nClose(long handle);
}
