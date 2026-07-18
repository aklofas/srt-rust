package org.tstrans.klv;

import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.List;

/**
 * One captured MISB ST 0601.19 §8.102 Item 102 (SDCC-FLP) occurrence, on
 * {@link UasDatalinkLs#sdccFlps()}. MULTI-INSTANCE per ST 0601.19 Table 1
 * ("Multiples Allowed" = Yes): each wire occurrence refines the accuracy of
 * the Local Set items that immediately precede it. {@code precedingTags} is
 * the wire-order item tags (known or unknown, but never another Item 102)
 * immediately before this occurrence; {@code bytes} is the raw SDCC-FLP
 * pack — decode it with {@link Klv#decodeSdccFlp(byte[])}.
 *
 * <p>{@code bytes} round-trips byte-exact even for a malformed or
 * foreign-encoder pack this binding cannot parse; re-encoding a record
 * re-emits every {@code sdccFlps} entry verbatim, grouped together in
 * tag-ascending position (the <em>original</em> interleaving recorded by
 * {@code precedingTags} is therefore not guaranteed to be reproduced on
 * re-encode — see the Rust {@code SdccFlpField} rustdoc).
 *
 * @param precedingTags the wire-order item tags immediately preceding this occurrence
 * @param bytes         raw SDCC-FLP pack bytes, exactly as they appeared on the wire
 */
public record SdccFlpField(List<Long> precedingTags, ByteBuffer bytes) {

    /** Compact constructor makes {@code precedingTags} non-null and immutable. */
    public SdccFlpField {
        precedingTags = precedingTags == null
                ? Collections.emptyList()
                : Collections.unmodifiableList(precedingTags);
    }
}
