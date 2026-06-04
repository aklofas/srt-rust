package org.tstrans.mpegts;

/**
 * Audio codec tag for a demuxed elementary stream. Mirrors the Rust enum
 * {@code tst_core::mpegts::demux::event::AudioCodec}.
 */
public enum AudioCodec {
    MP2,
    AAC,
    AAC_LATM,
    AC3
}
