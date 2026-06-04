package org.tstrans.mpegts;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * A demuxed event. Sealed sum type mirroring
 * {@code tst_core::mpegts::demux::DemuxEvent} (spec §5.2). The sample variant is
 * now split into {@link Video} / {@link Audio} / {@link Subtitle} /
 * {@link UnknownSample} (mirroring tst-py), and {@link Metadata} (KLV) is now
 * surfaced; the remaining top-level variants (NonConformant,
 * ReconnectDiscontinuity) land in later tasks of this wave-set and are added to
 * {@code permits} then.
 */
public sealed interface DemuxEvent
        permits DemuxEvent.ProgramMap, DemuxEvent.Video, DemuxEvent.Audio,
                DemuxEvent.Subtitle, DemuxEvent.UnknownSample, DemuxEvent.Metadata,
                DemuxEvent.Discontinuity {

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

    /**
     * Standalone metadata — KLV (sync AU-cell or async) or an unrecognized
     * metadata stream. Mirrors tst-py's metadata event.
     *
     * @param stream         the elementary stream this metadata belongs to
     * @param pts            presentation timestamp in 90&nbsp;kHz ticks
     * @param kind           the metadata classification (see {@link MetadataKind})
     * @param payload        Raw KLV LS bytes; the H.222.0 §2.12.4.2 AU-cell header
     *                       is already stripped. Heap-copied / JVM-owned (safe to
     *                       retain; FFM zero-copy deferred to a JDK-22+ path).
     *                       Decode with the (future) {@code org.tstrans.klv} module.
     * @param wasReassembled {@code true} if a multi-cell AU was reassembled from
     *                       First + 0..n Middle + Last cells (always {@code false}
     *                       for async / unknown metadata)
     * @param cellCount      number of AU cells that contributed to this event
     *                       ({@code 1} for single-cell / async / unknown)
     */
    record Metadata(StreamId stream, long pts, MetadataKind kind, ByteBuffer payload,
                    boolean wasReassembled, int cellCount) implements DemuxEvent {}

    /** Continuity-counter / PCR discontinuity on a PID. */
    record Discontinuity(int pid) implements DemuxEvent {}
}
