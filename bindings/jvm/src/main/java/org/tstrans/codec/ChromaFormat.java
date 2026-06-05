package org.tstrans.codec;

/**
 * Chroma subsampling format. From H.264 / H.265 {@code chroma_format_idc}.
 * Mirrors {@code tstrans.codec.ChromaFormat}.
 *
 * <p>{@link #INVALID} is the catch-all for spec-reserved / future extension
 * values (the underlying Rust enum is open).
 */
public enum ChromaFormat {
    MONOCHROME, YUV420, YUV422, YUV444, INVALID
}
