package org.tstrans.mpegts;

/**
 * Video codec tag for a demuxed elementary stream. Mirrors the Rust enum
 * {@code tst_core::mpegts::demux::event::VideoCodec}.
 */
public enum VideoCodec {
    H264,
    H265,
    H266,
    AV1
}
