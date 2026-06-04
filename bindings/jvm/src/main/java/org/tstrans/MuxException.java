package org.tstrans;

/**
 * Thrown when the MPEG-TS muxer rejects a config or push. {@link Kind} mirrors
 * the 5-variant {@code tst_core::error::MuxSenderErrorKind} coarse classification
 * (the same buckets tst-py's {@code MuxErrorKind} uses).
 */
public final class MuxException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** Discriminant; values match the Rust {@code MuxSenderErrorKind} variants. */
    public enum Kind {
        INPUT_MALFORMED, CONFIG_INVALID, INVALID_USAGE, BACKPRESSURE, INTERNAL
    }

    private final Kind kind;

    public MuxException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    /** @return the error discriminant. */
    public Kind kind() {
        return kind;
    }
}
