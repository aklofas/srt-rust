package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Light-weight AV1 Frame Header (AV1 Bitstream Spec §5.9).
 * Mirrors {@code tst_core::codec::av1::Av1FrameHeaderLight} (and tst-py's
 * {@code tstrans.codec.Av1FrameHeaderLight}).
 *
 * <p>Light scope: {@code frameType} + {@code showFrame} +
 * {@code showExistingFrame} only. {@link #frameSize} is always {@code null} in
 * the current scope — full per-frame size extraction requires reference-frame
 * management beyond this parser. (The Rust field is {@code Option<(u32, u32)>};
 * tst-py exposes it as an optional {@code (width, height)} tuple, modelled here
 * as the nullable nested {@link FrameSize} record.)
 *
 * @param frameType         {@code frame_type} per §5.9.1: 0=KEY_FRAME,
 *                          1=INTER_FRAME, 2=INTRA_ONLY_FRAME, 3=SWITCH_FRAME
 * @param showFrame         true when the decoded frame is displayed immediately
 * @param showExistingFrame true when this OBU references a previously decoded
 *                          frame for display
 * @param frameSize         per-frame size override, or {@code null} in the
 *                          current light scope
 * @param raw               original payload bytes (heap {@code ByteBuffer})
 */
public record Av1FrameHeaderLight(
        int frameType,
        boolean showFrame,
        boolean showExistingFrame,
        FrameSize frameSize,
        ByteBuffer raw) {

    /**
     * A per-frame size override. Mirrors the Rust {@code (u32, u32)} tuple;
     * the dimensions are Java {@code long} because the underlying fields are
     * {@code u32}.
     *
     * @param width  override frame width in luma samples
     * @param height override frame height in luma samples
     */
    public record FrameSize(long width, long height) {
    }
}
