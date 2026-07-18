package org.tstrans.klv;

/**
 * One record of MISB ST 0601.19 §8.138 Item 138, Payload List.
 *
 * <p>{@code payloadTypeCode} is the raw wire codepoint (authoritative
 * representation, including forward-compat unknown codes); use
 * {@link #payloadType()} for the typed enum view — {@code null} for a
 * wire-unknown codepoint, same asymmetric pattern as {@link IcingDetected}.
 *
 * @param id              BER-OID payload id
 * @param payloadTypeCode raw ST 0601.19 §8.138 Table 17 wire codepoint
 * @param name            payload name
 */
public record PayloadRecord(long id, long payloadTypeCode, String name) {

    /** Tag Table 17 payload type as a typed enum, or {@code null} (wire-unknown). */
    public PayloadType payloadType() {
        return PayloadType.fromCode(payloadTypeCode);
    }
}
