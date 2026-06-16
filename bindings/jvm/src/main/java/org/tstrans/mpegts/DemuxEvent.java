package org.tstrans.mpegts;

import java.nio.ByteBuffer;
import java.util.List;
import org.tstrans.CodecParseException;
import org.tstrans.codec.AudioFrame;
import org.tstrans.codec.VideoUnit;

/**
 * A demuxed event. Sealed sum type mirroring
 * {@code tst_core::mpegts::demux::DemuxEvent} (spec §5.2). The full top-level
 * event set is now surfaced: {@link ProgramMap}, the four sample records
 * ({@link Video} / {@link Audio} / {@link Subtitle} / {@link UnknownSample},
 * mirroring tst-py), {@link Metadata} (KLV), {@link NonConformant}
 * (stream-conformance diagnostics), {@link Discontinuity} (continuity /
 * PES-reassembly discontinuity, carrying a {@link StreamId} and a typed
 * {@link DiscontinuityKind}), and {@link ReconnectDiscontinuity} (transport-level
 * reconnect). No variant is skipped.
 */
public sealed interface DemuxEvent
        permits DemuxEvent.ProgramMap, DemuxEvent.Video, DemuxEvent.Audio,
                DemuxEvent.Subtitle, DemuxEvent.UnknownSample, DemuxEvent.Metadata,
                DemuxEvent.NonConformant, DemuxEvent.Discontinuity,
                DemuxEvent.ReconnectDiscontinuity {

    /**
     * PSI program map for one program (mirrors tst-py mpegts.ProgramMap).
     *
     * @param programNumber  the MPEG-TS program number, from the PAT entry
     * @param pcrPid         PID carrying the program's PCR (the PMT's PCR_PID field)
     * @param pmtPid         PID carrying this program's PMT, from the PAT entry
     *                       that declared the program
     * @param elementaryPids PIDs of the program's elementary streams, in PMT order
     */
    record ProgramMap(int programNumber, int pcrPid, int pmtPid, List<Integer> elementaryPids) implements DemuxEvent {}

    /**
     * A video access unit, carrying the raw encoded bytes and typed codec
     * units. {@code raw} is the exact encoded access unit; {@code payload}
     * is its parsed {@link List} of {@link VideoUnit} — {@code NalUnit}s for
     * H.264/H.265/H.266, {@code Obu}s for AV1. The {@code codec} field
     * disambiguates which.
     *
     * @param stream                 the elementary stream this sample belongs to
     * @param pts                    presentation timestamp in 90&nbsp;kHz ticks
     * @param dts                    decode timestamp in 90&nbsp;kHz ticks, or
     *                               {@code null} when absent
     * @param codec                  the video codec (also on {@code stream.kind()})
     * @param payload                the typed access-unit units —
     *                               {@code List<NalUnit>} for H.264/H.265/H.266,
     *                               {@code List<Obu>} for AV1. Each unit's own
     *                               byte payload is a heap (JVM-owned)
     *                               {@link ByteBuffer}, safe to retain past the
     *                               next {@link Demuxer#nextEvent()} or
     *                               {@link Demuxer#close()}.
     * @param raw                    the exact encoded access unit — Annex-B byte
     *                               stream for H.264/H.265/H.266, on-wire PES
     *                               payload for AV1 — as a heap (JVM-owned)
     *                               {@link ByteBuffer} copy, safe to retain (true
     *                               zero-copy is deferred to a JDK&nbsp;22+ FFM
     *                               path). For H.264/H.265/H.266, feed it back to
     *                               {@link Muxer#pushVideo(byte[], long, boolean)}
     *                               for byte-faithful transmux. For AV1, use
     *                               {@link Muxer#pushVideoWire(byte[], long, boolean)}
     *                               instead (with a destination muxer configured to
     *                               the same {@link #av1Carriage()} mode) — {@code
     *                               pushVideo} would re-wrap the wire bytes and
     *                               corrupt the stream. Mirrors tst-py's {@code .raw}.
     * @param randomAccessIndicator  whether this access unit is a random-access
     *                               point (keyframe)
     * @param codecParseError        always {@code null} for video — the units in
     *                               {@code payload} were split by the binding, so
     *                               typed payload construction cannot fail at
     *                               this layer
     * @param av1Carriage            AV1 carriage provenance
     *                               ({@link Av1CarriageMode#MPEG2_TS_BINDING} or
     *                               {@link Av1CarriageMode#INTEROP_RAW_OBU});
     *                               {@code null} for non-AV1 codecs. For
     *                               byte-faithful re-mux, configure the destination
     *                               muxer to this carriage mode and push {@code raw}
     *                               via {@link Muxer#pushVideoWire}.
     */
    record Video(StreamId stream, long pts, Long dts, VideoCodec codec,
                 List<VideoUnit> payload, ByteBuffer raw, boolean randomAccessIndicator,
                 CodecParseException codecParseError, Av1CarriageMode av1Carriage) implements DemuxEvent {}

    /**
     * An audio access unit. On a clean AAC / MP2 parse the {@code payload} is a
     * typed {@link List} of {@link AudioFrame} ({@code AdtsFrame} for AAC,
     * {@code Mpeg2AudioFrame} for MP2), {@code rawPayload} and
     * {@code codecParseError} are {@code null}. On a mid-stream parse failure the
     * {@code payload} is an empty list, {@code rawPayload} carries the raw frame
     * bytes (heap {@link ByteBuffer}), and {@code codecParseError} describes the
     * failure. For deferred codecs (AAC-LATM, AC-3) the {@code payload} is empty,
     * {@code rawPayload} carries the bytes, and {@code codecParseError} is
     * {@code null} (silent fallback). Mirrors tst-py's audio event.
     *
     * @param stream          the elementary stream this sample belongs to
     * @param pts             presentation timestamp in 90&nbsp;kHz ticks
     * @param dts             decode timestamp in 90&nbsp;kHz ticks, or {@code null} when absent
     * @param codec           the audio codec (also on {@code stream.kind()})
     * @param payload         typed frames on a clean parse, else an empty list
     * @param rawPayload      raw frame bytes (heap {@link ByteBuffer}) on a
     *                        bytes-fallback path, else {@code null}
     * @param codecParseError the parse failure on a mid-stream error, else
     *                        {@code null} (also {@code null} for the silent
     *                        deferred-codec fallback)
     */
    record Audio(StreamId stream, long pts, Long dts, AudioCodec codec,
                 List<AudioFrame> payload, ByteBuffer rawPayload,
                 CodecParseException codecParseError) implements DemuxEvent {}

    /**
     * A subtitle access unit.
     *
     * @param stream  the elementary stream this sample belongs to
     * @param pts     presentation timestamp in 90&nbsp;kHz ticks
     * @param dts     decode timestamp in 90&nbsp;kHz ticks, or {@code null} when absent
     * @param codec   the subtitle codec (also on {@code stream.kind()})
     * @param payload the access-unit bytes as a heap (JVM-owned) {@link ByteBuffer}
     *                — a copy, safe to retain and read at any time (including
     *                after the next {@link Demuxer#nextEvent()} or
     *                {@link Demuxer#close()}). True zero-copy is deferred to a
     *                future JDK&nbsp;22+ Foreign Function &amp; Memory path; this
     *                JDK&nbsp;17 baseline copies.
     */
    record Subtitle(StreamId stream, long pts, Long dts, SubtitleCodec codec,
                    ByteBuffer payload) implements DemuxEvent {}

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

    /**
     * A stream-conformance diagnostic emitted by the demuxer. Mirrors tst-py's
     * NonConformant event: Rust's 30+ {@code NonConformantIssue} variants are
     * collapsed to a {@link NonConformantKind} discriminator plus the
     * human-readable {@code issue} detail string.
     *
     * @param stream            the elementary stream the diagnostic concerns
     * @param issue             the human-readable detail string (Rust
     *                          {@code NonConformantIssue}'s {@code Display})
     * @param kind              the collapsed diagnostic discriminator
     * @param multiCellAuReason the multi-cell AU reassembly failure reason —
     *                          non-{@code null} only when
     *                          {@code kind == MULTI_CELL_AU}, {@code null} otherwise
     * @param observedCfi       the wire {@code cell_fragment_indication} bits —
     *                          non-{@code null} only when
     *                          {@code kind == CFI_TOLERATED}, {@code null} otherwise
     * @param treatedAs         the {@code cell_fragment_indication} the demuxer
     *                          substituted — non-{@code null} only when
     *                          {@code kind == CFI_TOLERATED}, {@code null} otherwise
     */
    record NonConformant(StreamId stream, String issue, NonConformantKind kind,
                         MultiCellAuReason multiCellAuReason, CellFragmentIndication observedCfi,
                         CellFragmentIndication treatedAs) implements DemuxEvent {}

    /**
     * A continuity-counter / PES-reassembly discontinuity on a stream. Mirrors
     * {@code tst_core::...::DemuxEvent::Discontinuity}.
     *
     * @param stream the elementary stream the discontinuity concerns
     * @param kind   the discontinuity classification (see {@link DiscontinuityKind})
     */
    record Discontinuity(StreamId stream, DiscontinuityKind kind) implements DemuxEvent {}

    /** Transport-level reconnect occurred between the prior event and this one; all per-stream state was dropped (re-derived from the next PAT/PMT). Mirrors {@code tst_core::...::DemuxEvent::ReconnectDiscontinuity}. */
    record ReconnectDiscontinuity() implements DemuxEvent {}
}
