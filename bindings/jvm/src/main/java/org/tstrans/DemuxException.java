package org.tstrans;

/**
 * Thrown when the MPEG-TS demuxer rejects input. {@link Kind} mirrors
 * {@code tst_core::mpegts::demux::DemuxError} 1:1.
 */
public final class DemuxException extends BindingException {
    private static final long serialVersionUID = 1L;

    /** Discriminant; values match the Rust {@code DemuxError} variants. */
    // UNEXPECTED_EOF (in tst-py's DemuxErrorKind) is reserved for the file-helper/io wave; the raw demux feed path produces none of it. It returns to this enum when that producer lands.
    public enum Kind {
        SYNC_LOSS, BAD_PMT, BAD_PES, STRICT_REJECTION, INTERNAL
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
