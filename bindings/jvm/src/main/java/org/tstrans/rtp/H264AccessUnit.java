package org.tstrans.rtp;

/**
 * A fully reassembled H.264 Access Unit, returned by
 * {@link H264Receiver#recvAu()}. Immutable.
 *
 * <p>Mirrors tst-py {@code tstrans.rtp.H264AccessUnit} and
 * {@code tst_rtp::H264Au}.
 *
 * <p><b>Byte-copy posture (JDK 17):</b> {@link #annexb()} returns a fresh heap
 * {@code byte[]} copied from Rust memory. A zero-copy path (FFM
 * {@code MemorySegment}) is JDK-22+ only and will be added in a future release.
 */
public final class H264AccessUnit {
    private final byte[] annexb;
    private final long pts;
    private final boolean keyFrame;
    private final long rtpTimestamp;

    /**
     * Package-private constructor — instances are created only by the JNI layer
     * ({@code H264Receiver.nRecvAu}).
     */
    H264AccessUnit(byte[] annexb, long pts, boolean keyFrame, long rtpTimestamp) {
        // annexb is already a fresh heap copy made by the JNI layer; no additional
        // clone needed here (the JNI native allocates it via env.byte_array_from_slice).
        this.annexb = annexb;
        this.pts = pts;
        this.keyFrame = keyFrame;
        this.rtpTimestamp = rtpTimestamp;
    }

    /**
     * Annex B–framed NALU bytes (one or more NALUs with {@code [0,0,0,1]} start
     * codes). A fresh heap copy is returned on each call. Never {@code null};
     * minimum length is 5 bytes (start code + NALU type byte).
     */
    public byte[] annexb() { return annexb.clone(); }

    /**
     * 90&nbsp;kHz decode-order timestamp (i64 ticks from
     * {@code tst_core::mpegts::common::Pts90khz}). Zero-based at the first
     * emitted AU; unwrapped across the 32-bit RTP timestamp rollover.
     *
     * <p>Values can be negative after an SSRC reset with a lower timestamp origin.
     * B-frame streams may produce non-monotonic values — the depacketizer passes
     * them through unaltered.
     */
    public long pts() { return pts; }

    /** {@code true} if the AU contains at least one IDR slice (NALU type 5). */
    public boolean keyFrame() { return keyFrame; }

    /**
     * Raw 32-bit RTP timestamp from the packet header, widened to {@code long}.
     * The unsigned bit pattern is preserved.
     */
    public long rtpTimestamp() { return rtpTimestamp; }

    @Override public String toString() {
        return "H264AccessUnit(pts=" + pts + ", keyFrame=" + keyFrame
            + ", rtpTimestamp=" + rtpTimestamp + ", annexb=<" + annexb.length + " bytes>)";
    }
}
