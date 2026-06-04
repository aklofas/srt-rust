package org.tstrans.klv;

/**
 * A non-fatal field-level validation error collected during lenient KLV decode.
 *
 * @param kind    classification of the validation failure
 * @param tag     the KLV tag (BER-OID value) for which the error occurred;
 *                0 for errors not tied to a specific tag (e.g. IMAPB params)
 * @param message human-readable detail from the Rust {@code Display} of the error
 */
public record KlvFieldError(KlvFieldErrorKind kind, long tag, String message) {}
