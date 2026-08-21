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
        TRANSPORT, MALFORMED_PACKET, CANCELLED,
        /**
         * Recv deadline expired — retryable; the transport/session is still
         * alive. Raised from two triggers: a persistent deadline configured
         * via the {@code ?recv_timeout=<ms>} URL query key on a receiver URL,
         * or a per-call timeout argument to {@code recv()} / {@code recvAu()}.
         * A receiver with neither configured blocks indefinitely instead of
         * raising this. Mirrors tst-py {@code RtpErrorKind.TIMEOUT}.
         */
        TIMEOUT
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
