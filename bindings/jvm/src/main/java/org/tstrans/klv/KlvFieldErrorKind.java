package org.tstrans.klv;

/**
 * Non-fatal field-validation error kind. Mirrors {@code tst_core::error::KlvFieldError}
 * variant classification. Field errors surface on each decoded set's
 * {@code fieldErrors()} list rather than throwing; the decode call succeeds
 * with partial data.
 */
public enum KlvFieldErrorKind {
    OUT_OF_RANGE,
    INVALID_UTF8,
    INVALID_UTF16,
    INVALID_LENGTH,
    INVALID_SENTINEL,
    INVALID_CODEPOINT,
    TRUNCATED_FIELD,
    UNSUPPORTED_IMAPB_LENGTH,
    INVALID_IMAPB_PARAMS
}
