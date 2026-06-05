package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Light-weight H.265 slice segment header — the fields required for keyframe
 * detection and frame-type classification without walking into slice data
 * (H.265 §7.3.6.1).
 * Mirrors {@code tst_core::codec::h265::H265SliceHeaderLight} (and tst-py's
 * {@code tstrans.codec.H265SliceHeaderLight}).
 *
 * @param firstInPic     {@code true} when {@code first_slice_segment_in_pic_flag == 1}
 *                       (start of a frame)
 * @param sliceType      slice type (B / P / I); {@code I} as a conservative
 *                       fallback for continuation slices without PPS context
 * @param ppsId          {@code slice_pic_parameter_set_id} linking this slice to a PPS
 * @param picOrderCntLsb {@code pic_order_cnt_lsb} read using the SPS bit width
 *                       ({@code log2MaxPicOrderCntLsbMinus4 + 4}); {@code 0} for IDR
 *                       slices; {@code null} when no SPS context was supplied
 * @param idr            {@code true} when {@code nal_unit_type} is IDR_W_RADL (19)
 *                       or IDR_N_LP (20)
 * @param rawRbsp        original RBSP bytes (heap {@code ByteBuffer})
 */
public record H265SliceHeaderLight(
        boolean firstInPic,
        H265SliceType sliceType,
        int ppsId,
        Integer picOrderCntLsb,
        boolean idr,
        ByteBuffer rawRbsp) {
}
