package org.tstrans.klv;

import java.nio.ByteBuffer;

/**
 * MISB ST 0806.4 Table 8-4 User Defined Local Set, carried on RVT Tag 11
 * (repeatable — ST 0806.4-25). {@code numericIdRaw} packs the Tag 1 byte
 * verbatim (bits 8-7 = data type, bits 6-1 = numeric id 0-63);
 * {@link #dataType()}/{@link #numericId()} are computed accessors (mirrors
 * the Rust {@code RvtUserData::data_type()}/{@code numeric_id()} methods and
 * the {@link WeaponsStore} bitfield-accessor pattern) — only the two wire
 * fields cross the binding.
 *
 * <p>Only two differently-typed fields (an {@code int} and a
 * {@code ByteBuffer}) — no positional-transposition hazard, so this is a
 * plain record (no {@code Builder}), matching the {@link CoreId} /
 * {@link GeoPoint} precedent rather than the multi-{@code String}-field
 * {@link RvtPoi}/{@link RvtAoi}/{@link RvtLs} pattern.
 *
 * @param numericIdRaw the packed Tag 1 byte (data-type bits + numeric id)
 * @param data         the Tag 2 payload bytes
 */
public record RvtUserData(int numericIdRaw, ByteBuffer data) {

    /** Compact constructor: default a {@code null} payload to empty (mirrors the Rust default). */
    public RvtUserData {
        if (data == null) {
            data = ByteBuffer.wrap(new byte[0]);
        }
    }

    /** @return the ST 0806.4 Table 8-4 data-type code (top 2 bits of {@code numericIdRaw}). */
    public RvtUserDataType dataType() {
        return RvtUserDataType.fromCode((numericIdRaw >> 6) & 0x3);
    }

    /** @return the numeric id (bottom 6 bits of {@code numericIdRaw}, 0-63). */
    public int numericId() {
        return numericIdRaw & 0x3F;
    }
}
