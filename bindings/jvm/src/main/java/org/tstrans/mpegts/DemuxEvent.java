package org.tstrans.mpegts;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * A demuxed event. Sealed sum type mirroring
 * {@code tst_core::mpegts::demux::DemuxEvent} (spec §5.2). The keystone ships
 * ProgramMap / Sample / Discontinuity; remaining top-level variants (Metadata,
 * NonConformant, ReconnectDiscontinuity) land in the mpegts-completion wave and
 * are added to {@code permits} then. (An UnknownSample event is built from
 * {@code SamplePayload::Unknown}, a Sample payload — not a top-level variant.)
 */
public sealed interface DemuxEvent
        permits DemuxEvent.ProgramMap, DemuxEvent.Sample, DemuxEvent.Discontinuity {

    /** PSI program map for one program (mirrors tst-py mpegts.ProgramMap). */
    record ProgramMap(int programNumber, int pcrPid, List<Integer> elementaryPids) implements DemuxEvent {}

    /**
     * Elementary-stream access unit. {@code kind} is the stream class.
     *
     * @param pid     the elementary-stream PID this sample belongs to
     * @param pts     presentation timestamp in 90&nbsp;kHz ticks
     * @param kind    coarse payload class (see {@link SampleKind})
     * @param payload the access-unit bytes as a heap (JVM-owned)
     *                {@link ByteBuffer} — a copy of the demuxed bytes. Because
     *                the JVM owns the backing array, this buffer is safe to
     *                retain and read at any time (including after the next
     *                {@link Demuxer#nextEvent()} pull or after {@link
     *                Demuxer#close()}). True zero-copy (a direct buffer over
     *                native memory with a <em>defined</em> stale-read error,
     *                spec&nbsp;§5.4) is deferred to a future JDK&nbsp;22+ path
     *                built on the Foreign Function &amp; Memory API
     *                ({@code Arena}/{@code MemorySegment}); on the JDK&nbsp;17
     *                baseline this binding copies.
     */
    record Sample(int pid, long pts, SampleKind kind, ByteBuffer payload) implements DemuxEvent {}

    /** Continuity-counter / PCR discontinuity on a PID. */
    record Discontinuity(int pid) implements DemuxEvent {}

    /** Coarse payload class for the keystone; full typing in the codec wave. */
    enum SampleKind { VIDEO, AUDIO, KLV, SUBTITLE, OTHER }
}
