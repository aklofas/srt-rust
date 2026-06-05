package org.tstrans.codec;

import java.nio.ByteBuffer;

/**
 * One AV1 Open Bitstream Unit (AV1 Bitstream Spec §5.3.2).
 * Mirrors {@code tstrans.codec.Obu}.
 *
 * <p>The OBU header byte, any extension byte, and the LEB128 {@code obu_size}
 * field are stripped; {@code payload} carries only the OBU body (a heap
 * {@code ByteBuffer}). {@code extension} is {@code null} when
 * {@code obu_extension_flag = 0}.
 *
 * @param obuType   4-bit {@code obu_type}
 * @param extension optional extension header, or {@code null}
 * @param payload   OBU body bytes (heap {@code ByteBuffer})
 */
public record Obu(int obuType, ObuExtension extension, ByteBuffer payload) implements VideoUnit {
}
