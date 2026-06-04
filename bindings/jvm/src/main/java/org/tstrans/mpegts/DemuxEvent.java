package org.tstrans.mpegts;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * A demuxed event. Sealed sum type mirroring
 * {@code tst_core::mpegts::demux::DemuxEvent} (spec §5.2). The sample variant is
 * now split into {@link Video} / {@link Audio} / {@link Subtitle} /
 * {@link UnknownSample} (mirroring tst-py); remaining top-level variants
 * (Metadata, NonConformant, ReconnectDiscontinuity) land in later tasks of this
 * wave-set and are added to {@code permits} then.
 */
public sealed interface DemuxEvent
        permits DemuxEvent.ProgramMap, DemuxEvent.Video, DemuxEvent.Audio,
                DemuxEvent.Subtitle, DemuxEvent.UnknownSample, DemuxEvent.Discontinuity {

    /** PSI program map for one program (mirrors tst-py mpegts.ProgramMap). */
    record ProgramMap(int programNumber, int pcrPid, List<Integer> elementaryPids) implements DemuxEvent {}

    /**
     * A video access unit (concatenated NAL/OBU payload bytes; typed payloads
     * deferred to the codec wave). The codec is carried on
     * {@code stream.kind()}, not duplicated here.
     *
     * @param stream                 the elementary stream this sample belongs to
     * @param pts                    presentation timestamp in 90&nbsp;kHz ticks
     * @param dts                    decode timestamp in 90&nbsp;kHz ticks, or
     *                               {@code null} when absent
     * @param payload                the access-unit bytes as a heap (JVM-owned)
     *                               {@link ByteBuffer} — a copy of the demuxed
     *                               bytes, safe to retain and read at any time
     *                               (including after the next
     *                               {@link Demuxer#nextEvent()} or
     *                               {@link Demuxer#close()}). True zero-copy
     *                               (a direct buffer over native memory) is
     *                               deferred to a future JDK&nbsp;22+ Foreign
     *                               Function &amp; Memory ({@code Arena}/
     *                               {@code MemorySegment}) path; this JDK&nbsp;17
     *                               baseline copies.
     * @param randomAccessIndicator  whether this access unit is a random-access
     *                               point (keyframe)
     */
    record Video(StreamId stream, long pts, Long dts, ByteBuffer payload,
                 boolean randomAccessIndicator) implements DemuxEvent {}

    /**
     * An audio access unit. The codec is carried on {@code stream.kind()}.
     *
     * @param stream  the elementary stream this sample belongs to
     * @param pts     presentation timestamp in 90&nbsp;kHz ticks
     * @param dts     decode timestamp in 90&nbsp;kHz ticks, or {@code null} when absent
     * @param payload the access-unit bytes as a heap (JVM-owned) {@link ByteBuffer}
     *                — a copy, safe to retain and read at any time (including
     *                after the next {@link Demuxer#nextEvent()} or
     *                {@link Demuxer#close()}). True zero-copy is deferred to a
     *                future JDK&nbsp;22+ Foreign Function &amp; Memory path; this
     *                JDK&nbsp;17 baseline copies.
     */
    record Audio(StreamId stream, long pts, Long dts, ByteBuffer payload) implements DemuxEvent {}

    /**
     * A subtitle access unit. The codec is carried on {@code stream.kind()}.
     *
     * @param stream  the elementary stream this sample belongs to
     * @param pts     presentation timestamp in 90&nbsp;kHz ticks
     * @param dts     decode timestamp in 90&nbsp;kHz ticks, or {@code null} when absent
     * @param payload the access-unit bytes as a heap (JVM-owned) {@link ByteBuffer}
     *                — a copy, safe to retain and read at any time (including
     *                after the next {@link Demuxer#nextEvent()} or
     *                {@link Demuxer#close()}). True zero-copy is deferred to a
     *                future JDK&nbsp;22+ Foreign Function &amp; Memory path; this
     *                JDK&nbsp;17 baseline copies.
     */
    record Subtitle(StreamId stream, long pts, Long dts, ByteBuffer payload) implements DemuxEvent {}

    /**
     * An access unit on a stream whose codec the demuxer does not recognize.
     * {@code streamType} is the raw PMT {@code stream_type} byte (0..=255).
     *
     * @param stream     the elementary stream this sample belongs to
     * @param pts        presentation timestamp in 90&nbsp;kHz ticks
     * @param dts        decode timestamp in 90&nbsp;kHz ticks, or {@code null} when absent
     * @param streamType the raw PMT {@code stream_type} byte (0..=255)
     * @param payload    the access-unit bytes as a heap (JVM-owned)
     *                   {@link ByteBuffer} — a copy, safe to retain and read at
     *                   any time (including after the next
     *                   {@link Demuxer#nextEvent()} or {@link Demuxer#close()}).
     *                   True zero-copy is deferred to a future JDK&nbsp;22+
     *                   Foreign Function &amp; Memory path; this JDK&nbsp;17
     *                   baseline copies.
     */
    record UnknownSample(StreamId stream, long pts, Long dts, int streamType,
                         ByteBuffer payload) implements DemuxEvent {}

    /** Continuity-counter / PCR discontinuity on a PID. */
    record Discontinuity(int pid) implements DemuxEvent {}
}
