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
     * @param payload the access-unit bytes as a DIRECT (off-heap)
     *                {@link ByteBuffer} backed by native memory (zero-copy,
     *                spec&nbsp;§5.4). The buffer is valid <strong>only until the
     *                next {@link Demuxer#nextEvent()} pull</strong> on the
     *                owning demuxer — pulling the next event overwrites the
     *                backing storage, so any earlier {@code payload} buffer is
     *                invalidated. To retain the bytes past that point, copy them
     *                while this sample is current, e.g.
     *                {@code ByteBuffer copy = ByteBuffer.allocate(payload.remaining()).put(payload.duplicate()).flip();}
     *                or {@code byte[] b = new byte[payload.remaining()]; payload.duplicate().get(b);}.
     *                (A generation-counter guard that turns a stale read into a
     *                defined {@link IllegalStateException} arrives in the
     *                mpegts-completion wave; until then a stale read is
     *                undefined.)
     */
    record Sample(int pid, long pts, SampleKind kind, ByteBuffer payload) implements DemuxEvent {}

    /** Continuity-counter / PCR discontinuity on a PID. */
    record Discontinuity(int pid) implements DemuxEvent {}

    /** Coarse payload class for the keystone; full typing in the codec wave. */
    enum SampleKind { VIDEO, AUDIO, KLV, SUBTITLE, OTHER }
}
