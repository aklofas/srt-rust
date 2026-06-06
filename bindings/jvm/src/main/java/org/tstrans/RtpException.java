package org.tstrans;

/**
 * Checked exception for the RTP transport surface ({@code org.tstrans.rtp}).
 * Mirrors tst-py's {@code tstrans.exceptions.RtpError} / {@code RtpErrorKind}.
 * {@link Kind} maps the Rust {@code tst_core::transport::TransportError} and
 * {@code tst_rtp::ConnectError} families onto three user-facing buckets — see
 * {@code bindings/jvm/src/rtp/errors.rs}.
 */
public final class RtpException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** RTP failure category. Names match tst-py {@code RtpErrorKind} 1:1. */
    public enum Kind {
        TRANSPORT, MALFORMED_PACKET, CANCELLED
    }

    private final Kind kind;

    public RtpException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    /** @return the error discriminant. */
    public Kind kind() {
        return kind;
    }
}
