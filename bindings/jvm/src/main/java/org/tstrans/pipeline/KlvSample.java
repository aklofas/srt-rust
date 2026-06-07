package org.tstrans.pipeline;

import java.nio.ByteBuffer;
import org.tstrans.mpegts.MetadataKind;
import org.tstrans.mpegts.StreamId;

/** Projection of a KLV metadata unit (raw LS bytes; AU-cell header already
 *  peeled). Mirrors {@code tstrans.pipeline.KlvSample}. The {@code payload} is
 *  the raw KLV LS bytes as a heap (JVM-owned) {@link ByteBuffer} — the Rust side
 *  copies the {@code Vec<u8>} into the heap buffer, so it is safe to retain (FFM
 *  zero-copy deferred to a JDK-22+ path). */
public record KlvSample(StreamId stream, long pts, MetadataKind kind, ByteBuffer payload) {}
