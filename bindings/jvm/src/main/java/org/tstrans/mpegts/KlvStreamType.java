package org.tstrans.mpegts;

/**
 * Transport-stream type for a KLV metadata elementary stream. Mirrors
 * {@code tst_core::mpegts::mux::KlvStreamType}.
 *
 * <p>{@code SYNCHRONOUS_METADATA} (PMT stream_type 0x15) is strict ST 1402
 * sync KLV — the muxer auto-prepends the 5-byte {@code Metadata_AU_cell}
 * header per ITU-T H.222.0 V9 §2.12.4.2 on every push (callers pass raw KLV
 * LS bytes; do NOT pre-wrap). {@code PRIVATE_DATA} (stream_type 0x06) is the
 * broadly-recognized form and passes the payload through as-is.
 *
 * <p>Ordinal contract: the JNI bridge maps this enum by ORDINAL — keep this
 * declaration order in lockstep with the Rust mapping in the {@code Muxer} JNI
 * ({@code 0 => SynchronousMetadata, 1 => PrivateData}).
 */
public enum KlvStreamType {
    SYNCHRONOUS_METADATA,
    PRIVATE_DATA
}
