package org.tstrans.pipeline;

import java.util.List;
import org.tstrans.codec.VideoUnit;
import org.tstrans.mpegts.StreamId;
import org.tstrans.mpegts.VideoCodec;

/** Projection of a paired/unpaired video access unit. Mirrors
 *  {@code tstrans.pipeline.VideoSample}. {@code dts} is null when absent;
 *  {@code payload} is {@code NalUnit}s (H.26x) or {@code Obu}s (AV1). Each unit's
 *  own byte payload is a heap (JVM-owned) {@link java.nio.ByteBuffer}, safe to
 *  retain (FFM zero-copy deferred to a JDK-22+ path). */
public record VideoSample(StreamId stream, long pts, Long dts, VideoCodec codec, List<VideoUnit> payload) {}
