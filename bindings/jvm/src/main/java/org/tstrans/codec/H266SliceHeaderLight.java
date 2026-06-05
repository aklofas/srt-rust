package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * Light-weight H.266 / VVC slice header — the fields required for keyframe
 * detection and frame-type classification without walking into slice data
 * (H.266 V4 §7.3.7.1).
 * Mirrors {@code tst_core::codec::h266::H266SliceHeaderLight} (and tst-py's
 * {@code tstrans.codec.H266SliceHeaderLight}).
 *
 * <p><b>Known limitations</b> (inherited from the Rust light parser):
 * {@code sliceType} always returns {@link H266SliceType#I} and {@code ppsId}
 * always returns {@code 0} as sentinels — accurate extraction requires walking
 * through {@code picture_header_rbsp()}, whose length is governed by SPS / PPS
 * context this light parser does not carry. {@code idr} and {@code firstInPic}
 * are accurate.
 *
 * @param firstInPic     {@code true} when
 *                       {@code picture_header_in_slice_header_flag == 1} (start
 *                       of a new picture)
 * @param sliceType      slice type — always {@link H266SliceType#I} (sentinel)
 * @param ppsId          PPS id — always {@code 0} (sentinel)
 * @param picOrderCntLsb {@code slice_pic_order_cnt_lsb}; {@code 0} for IDR
 *                       slices (implicit per spec); {@code null} for non-IDR
 *                       slices where SPS context is required
 * @param idr            {@code true} when {@code nal_unit_type} is IDR_W_RADL
 *                       (7) or IDR_N_LP (8)
 * @param rawRbsp        original RBSP bytes (heap {@code ByteBuffer})
 */
public record H266SliceHeaderLight(
        boolean firstInPic,
        H266SliceType sliceType,
        int ppsId,
        Integer picOrderCntLsb,
        boolean idr,
        ByteBuffer rawRbsp) {
}
