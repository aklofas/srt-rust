package org.tstrans.mpegts;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * A demuxed event. Sealed sum type mirroring
 * {@code tst_core::mpegts::demux::DemuxEvent} (spec §5.2). The keystone ships
 * ProgramMap / Sample / Discontinuity; remaining variants (Metadata,
 * NonConformant, UnknownSample, ReconnectDiscontinuity) land in the
 * mpegts-completion wave and are added to {@code permits} then.
 */
public sealed interface DemuxEvent
        permits DemuxEvent.ProgramMap, DemuxEvent.Sample, DemuxEvent.Discontinuity {

    /** PSI program map for one program (mirrors tst-py mpegts.ProgramMap). */
    record ProgramMap(int programNumber, int pcrPid, List<Integer> elementaryPids) implements DemuxEvent {}

    /** Elementary-stream access unit. {@code kind} is the stream class. */
    record Sample(int pid, long pts, SampleKind kind, ByteBuffer payload) implements DemuxEvent {}

    /** Continuity-counter / PCR discontinuity on a PID. */
    record Discontinuity(int pid) implements DemuxEvent {}

    /** Coarse payload class for the keystone; full typing in the codec wave. */
    enum SampleKind { VIDEO, AUDIO, KLV, SUBTITLE, OTHER }
}
