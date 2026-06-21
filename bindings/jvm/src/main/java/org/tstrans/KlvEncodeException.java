package org.tstrans;

import java.util.Optional;

/**
 * Thrown when typed-KLV encode fails. {@link Kind} mirrors
 * {@code tst_core::error::KlvEncodeError}; {@link #tag()} carries the
 * offending KLV tag for the tag-bearing variants.
 */
public final class KlvEncodeException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** Discriminant; values map from the Rust {@code KlvEncodeError} variants. */
    public enum Kind {
        BUFFER_TOO_SMALL, RECORD_TOO_LARGE, OUT_OF_RANGE, STRING_TOO_LONG,
        UNSUPPORTED_IMAPB_LENGTH, INVALID_IMAPB_PARAMS,
        MISSING_MANDATORY_ITEM, RESERVED_TAG_IN_UNKNOWN,
        VTARGET_PACK_EMPTY, DUPLICATE_TARGET_ID, FORBIDDEN_STANDALONE_OFFSET
    }

    private final Kind kind;
    private final Long tag; // nullable — present only for tag-bearing variants

    /** Construct with no associated tag (e.g. {@code BUFFER_TOO_SMALL}). */
    public KlvEncodeException(Kind kind, String message) {
        this(kind, null, message);
    }

    /** Construct with an associated KLV tag (e.g. {@code OUT_OF_RANGE}). */
    public KlvEncodeException(Kind kind, Long tag, String message) {
        super(message);
        this.kind = kind;
        this.tag = tag;
    }

    /** @return the error discriminant. */
    public Kind kind() {
        return kind;
    }

    /** @return the offending KLV tag, or empty for variants that carry none. */
    public Optional<Long> tag() {
        return Optional.ofNullable(tag);
    }
}
