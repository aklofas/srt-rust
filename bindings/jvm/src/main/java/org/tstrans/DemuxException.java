package org.tstrans;

/**
 * Thrown when the MPEG-TS demuxer rejects input. {@link Kind} mirrors
 * {@code tst_core::mpegts::demux::DemuxError} 1:1.
 */
public final class DemuxException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** Discriminant; values match the Rust {@code DemuxError} variants. */
    public enum Kind {
        SYNC_LOSS, BAD_PMT, BAD_PES,
        /**
         * Parity-only constant mirroring tst-py's vestigial {@code DemuxErrorKind.UNEXPECTED_EOF}:
         * there is NO producer in {@code tst_core::DemuxError} (the file path treats truncation as
         * clean EOF and surfaces read failures as native {@code IOException}). Exempted in
         * {@code scripts/check/jvm/error-mapping-coverage.sh}.
         */
        UNEXPECTED_EOF,
        STRICT_REJECTION, INTERNAL
    }

    private final Kind kind;

    public DemuxException(Kind kind, String message) {
        super(message);
        this.kind = kind;
    }

    /** @return the error discriminant. */
    public Kind kind() {
        return kind;
    }
}
