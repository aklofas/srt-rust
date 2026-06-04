package org.tstrans.mpegts;

/** Elementary-stream classification. Mirrors {@code tst_core::...::StreamKind}. */
public sealed interface StreamKind
        permits StreamKind.Video, StreamKind.Audio, StreamKind.Subtitle,
                StreamKind.KlvSync, StreamKind.KlvAsync, StreamKind.Unknown {
    record Video(VideoCodec codec) implements StreamKind {}
    record Audio(AudioCodec codec) implements StreamKind {}
    record Subtitle(SubtitleCodec codec) implements StreamKind {}
    /** Synchronous KLV; {@code declaredLink} is the linked video PID, or null. */
    record KlvSync(Integer declaredLink) implements StreamKind {}
    record KlvAsync() implements StreamKind {}
    /** Unrecognized stream_type; {@code streamTypeByte} is the raw PMT byte. */
    record Unknown(int streamTypeByte) implements StreamKind {}
}
