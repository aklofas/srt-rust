package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Light-weight H.264 slice header — the fields required for keyframe detection
 * and frame-type classification without walking into slice data.
 * Mirrors {@code tst_core::codec::h264::H264SliceHeaderLight} (and tst-py's
 * {@code tstrans.codec.H264SliceHeaderLight}).
 *
 * @param firstInPic  {@code true} when {@code first_mb_in_slice == 0} (start of a frame)
 * @param sliceType   slice type (normalised via {@code slice_type % 5}, H.264 §7.4.3)
 * @param ppsId       {@code pic_parameter_set_id} linking this slice to a PPS
 * @param frameNum    {@code frame_num}, or {@code null} when no SPS context was supplied
 * @param idr         {@code true} when {@code nal_unit_type == 5} (IDR slice)
 * @param rawRbsp     original RBSP bytes (heap {@code ByteBuffer})
 */
public record H264SliceHeaderLight(
        boolean firstInPic,
        H264SliceType sliceType,
        int ppsId,
        Integer frameNum,
        boolean idr,
        ByteBuffer rawRbsp) {
}
