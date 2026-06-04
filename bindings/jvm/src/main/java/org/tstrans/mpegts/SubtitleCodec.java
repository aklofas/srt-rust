package org.tstrans.mpegts;

/**
 * Subtitle codec tag for a demuxed elementary stream. Mirrors the Rust enum
 * {@code tst_core::mpegts::demux::event::SubtitleCodec}.
 */
public enum SubtitleCodec {
    DVB_SUBTITLING,
    DVB_TELETEXT,
    CEA708_STANDALONE,
    WEBVTT_IN_TS
}
