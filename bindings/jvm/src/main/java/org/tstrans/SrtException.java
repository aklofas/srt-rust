package org.tstrans;

/**
 * Checked exception for the SRT transport surface ({@code org.tstrans.srt}).
 * Mirrors tst-py's {@code tstrans.exceptions.SrtError} / {@code SrtErrorKind}.
 * {@link Kind} maps the Rust {@code tst_srt} error families
 * (UrlError / ConnectError / BindError / AcceptError / IoError / TransportError)
 * onto eight user-facing buckets — see {@code bindings/jvm/src/srt/errors.rs}.
 */
public final class SrtException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** SRT failure category. Names match tst-py {@code SrtErrorKind} 1:1. */
    public enum Kind {
        CONFIG_INVALID, CONNECT_FAILED, ACCEPT_FAILED, TIMEOUT,
        CLOSED, BROKEN, WOULD_BLOCK, IO
    }

    private final Kind kind;

    public SrtException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    /** @return the error discriminant. */
    public Kind kind() {
        return kind;
    }
}
