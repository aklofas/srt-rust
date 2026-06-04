package org.tstrans.klv;

import java.util.Arrays;
import java.util.HexFormat;
import java.util.Optional;

/**
 * Static facade for MISB typed-KLV decode/encode (ST 0601 / 0102 / 0605 / 0903).
 * Mirrors tst-py's {@code tstrans.klv} free functions.
 *
 * <p>Decode/encode methods for each set are added in Tasks 1–4. The UL accessors
 * and {@link #isSt0601Family} are available immediately.
 */
public final class Klv {
    private Klv() {}

    // Backing arrays are private so the public surface cannot be mutated through
    // a shared reference; accessors below return defensive clones. Internal
    // callers use the backing arrays directly (no clone).
    private static final byte[] ST_0601_UL =
            HexFormat.of().parseHex("060e2b34020b01010e01030101000000");
    private static final byte[] SECURITY_LS_UL =
            HexFormat.of().parseHex("060e2b34020301010e01030302000000");
    private static final byte[] PRECISION_TIMESTAMP_PACK_UL =
            HexFormat.of().parseHex("060e2b34020501010e01010311000000");
    private static final byte[] VMTI_LS_UL =
            HexFormat.of().parseHex("060e2b34020b01010e01030306000000");

    // ST 0601 family prefix (bytes 0–12 must match). Held as a static so
    // isSt0601Family does not re-parse + re-allocate on every call.
    private static final byte[] ST_0601_FAMILY_PREFIX =
            HexFormat.of().parseHex("060e2b34020b01010e01030101");

    /** @return a defensive copy of the 16-byte ST 0601 UAS Datalink LS Universal Label. */
    public static byte[] st0601Ul() {
        return ST_0601_UL.clone();
    }

    /** @return a defensive copy of the 16-byte ST 0102 Security Metadata LS Universal Label. */
    public static byte[] securityLsUl() {
        return SECURITY_LS_UL.clone();
    }

    /** @return a defensive copy of the 16-byte ST 0605 Precision Time Stamp Pack Universal Label. */
    public static byte[] precisionTimestampPackUl() {
        return PRECISION_TIMESTAMP_PACK_UL.clone();
    }

    /** @return a defensive copy of the 16-byte ST 0903 VMTI LS Universal Label. */
    public static byte[] vmtiLsUl() {
        return VMTI_LS_UL.clone();
    }

    /**
     * Return {@code true} if {@code buf} has the ST 0601 family UL prefix.
     * Mirrors Rust {@code UniversalLabel::is_st0601_family}: bytes 0–12 match
     * the ST 0601 canonical prefix and byte 15 is {@code 0x00}.
     */
    public static boolean isSt0601Family(byte[] buf) {
        if (buf.length < 16) return false;
        for (int i = 0; i < 13; i++) {
            if (buf[i] != ST_0601_FAMILY_PREFIX[i]) return false;
        }
        return buf[15] == 0x00;
    }

    // -----------------------------------------------------------------------
    // ST 0605 — Precision Time Stamp Pack
    // -----------------------------------------------------------------------

    /**
     * Decode a full 26-byte ST 0605 Precision Time Stamp Pack (16-byte UL +
     * 1-byte BER length + 1-byte TimeStatus + 8-byte big-endian microsecond
     * timestamp).
     *
     * @param buf the 26-byte wire-format pack
     * @return the decoded {@link PrecisionTimeStampPack}
     * @throws org.tstrans.KlvDecodeException if the buffer is malformed or has the wrong UL
     */
    public static PrecisionTimeStampPack decodePrecisionTimestamp(byte[] buf)
            throws org.tstrans.KlvDecodeException {
        return nDecodePrecisionTimestamp(buf);
    }

    /**
     * Encode a {@link PrecisionTimeStampPack} to the 26-byte wire format.
     * Mirrors tst-py's {@code encode_precision_timestamp}; encoding is infallible.
     *
     * @param pack the pack to encode
     * @return the 26-byte wire-format buffer
     */
    public static byte[] encodePrecisionTimestamp(PrecisionTimeStampPack pack) {
        return nEncodePrecisionTimestamp(pack);
    }

    private static native PrecisionTimeStampPack nDecodePrecisionTimestamp(byte[] buf)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodePrecisionTimestamp(PrecisionTimeStampPack pack);

    // -----------------------------------------------------------------------
    // ST 0102 — Security Metadata LS
    // -----------------------------------------------------------------------

    /**
     * Decode an ST 0102 Security Metadata LS body (lenient mode).
     *
     * <p>{@code buf} is body-only — no Universal Label or outer BER length wrapper.
     * Lenient mode tolerates missing required tags, unknown enum codepoints
     * (surfaced as null typed accessors with raw code preserved), and malformed
     * Tag 13 UTF-16 (surfaced in {@link SecurityLs#fieldErrors()}).
     * Mirrors tst-py's {@code decode_security(buf)}.
     *
     * @param buf ST 0102 body bytes (no UL / outer BER length)
     * @return the decoded {@link SecurityLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is structurally malformed
     */
    public static SecurityLs decodeSecurity(byte[] buf)
            throws org.tstrans.KlvDecodeException {
        return nDecodeSecurity(buf, false);
    }

    /**
     * Decode an ST 0102 Security Metadata LS body with optional strict mode.
     *
     * <p>{@code buf} is body-only — no Universal Label or outer BER length wrapper.
     * When {@code strict = true}, rejects missing required tags (1, 2, 3, 12, 13, 22),
     * unknown enum codepoints, omitted-value codepoints, non-canonical BER, and
     * malformed UTF-16. Mirrors tst-py's {@code decode_security(buf, strict=True)}.
     *
     * @param buf    ST 0102 body bytes (no UL / outer BER length)
     * @param strict {@code true} for strict validation; {@code false} for lenient
     * @return the decoded {@link SecurityLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is rejected (including missing
     *                                        required tags in strict mode)
     */
    public static SecurityLs decodeSecurity(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException {
        return nDecodeSecurity(buf, strict);
    }

    /**
     * Encode a {@link SecurityLs} to ST 0102 body bytes.
     *
     * <p>Returns body-only bytes — no Universal Label or outer BER length wrapper.
     * Encoding is lenient (emits only populated fields; no mandatory-tag enforcement).
     * Mirrors tst-py's {@code encode_security(record)}.
     *
     * @param record the Security LS to encode
     * @return ST 0102 body bytes
     * @throws org.tstrans.KlvEncodeException if any field value is out of range or
     *                                        a reserved tag appears in {@code unknown}
     */
    public static byte[] encodeSecurity(SecurityLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeSecurity(record);
    }

    private static native SecurityLs nDecodeSecurity(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodeSecurity(SecurityLs record)
            throws org.tstrans.KlvEncodeException;

    // -----------------------------------------------------------------------
    // ST 0903 — VMTI LS + VTargetPack
    // -----------------------------------------------------------------------

    /**
     * Decode an ST 0903 VMTI LS body (lenient mode).
     *
     * <p>{@code buf} is body-only — no Universal Label or outer BER length wrapper.
     * Lenient mode tolerates missing required tags and surfaces per-field decode
     * failures in {@link VmtiLs#fieldErrors()}. Mirrors tst-py's
     * {@code decode_vmti(buf)}.
     *
     * @param buf ST 0903 body bytes (no UL / outer BER length)
     * @return the decoded {@link VmtiLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is structurally malformed
     */
    public static VmtiLs decodeVmti(byte[] buf) throws org.tstrans.KlvDecodeException {
        return nDecodeVmti(buf, false);
    }

    /**
     * Decode an ST 0903 VMTI LS body with optional strict mode.
     *
     * <p>{@code buf} is body-only — no Universal Label or outer BER length wrapper.
     * When {@code strict = true}, rejects missing required tags per ST 0903.6 §6
     * Table 1, duplicate tags, and malformed UTF-8. Mirrors tst-py's
     * {@code decode_vmti(buf, strict=True)}.
     *
     * @param buf    ST 0903 body bytes (no UL / outer BER length)
     * @param strict {@code true} for strict validation; {@code false} for lenient
     * @return the decoded {@link VmtiLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is rejected
     */
    public static VmtiLs decodeVmti(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException {
        return nDecodeVmti(buf, strict);
    }

    /**
     * Encode a {@link VmtiLs} to ST 0903 body bytes (embedded form).
     *
     * <p>Returns body-only bytes — no Universal Label, no outer BER length, and
     * no Tag 1 checksum (per ST 0903.6-120). For standalone carriage on a
     * dedicated KLV PID, use {@link #encodeVmtiStandalone(VmtiLs)}.
     * Mirrors tst-py's {@code encode_vmti(record)}.
     *
     * @param record the VMTI LS to encode
     * @return ST 0903 embedded body bytes
     * @throws org.tstrans.KlvEncodeException if any field value is out of range or
     *                                        a reserved tag appears in {@code unknown}
     */
    public static byte[] encodeVmti(VmtiLs record) throws org.tstrans.KlvEncodeException {
        return nEncodeVmti(record);
    }

    /**
     * Encode a {@link VmtiLs} as a standalone VMTI wire record.
     *
     * <p>Returns the full framing: {@code [VMTI_LS_UL:16][outer BER length][body][Tag1 checksum]}
     * per ST 0903.4-17 / ST 0903.6-119. The Tag 1 checksum is computed from the
     * assembled framing; any value in {@link VmtiLs#checksum()} is ignored.
     * Mirrors tst-py's {@code encode_vmti_standalone(record)}.
     *
     * @param record the VMTI LS to encode
     * @return the full standalone VMTI wire record
     * @throws org.tstrans.KlvEncodeException if any field value is out of range or
     *                                        a reserved tag appears in {@code unknown}
     */
    public static byte[] encodeVmtiStandalone(VmtiLs record) throws org.tstrans.KlvEncodeException {
        return nEncodeVmtiStandalone(record);
    }

    private static native VmtiLs nDecodeVmti(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodeVmti(VmtiLs record) throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeVmtiStandalone(VmtiLs record)
            throws org.tstrans.KlvEncodeException;

    // -----------------------------------------------------------------------
    // ST 0601 — UAS Datalink Local Set
    // -----------------------------------------------------------------------

    /**
     * Decode an ST 0601 UAS Datalink LS (lenient mode).
     *
     * <p>{@code buf} is the full wire-format payload starting with the 16-byte
     * Universal Label. Lenient mode accepts any 16-byte UL, verifies the checksum,
     * and collects per-field decode failures in {@link UasDatalinkLs#fieldErrors()}.
     * Mirrors tst-py's {@code decode_uas_datalink(buf)}.
     *
     * @param buf full ST 0601 wire bytes (UL + BER length + body)
     * @return the decoded {@link UasDatalinkLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is structurally malformed
     */
    public static UasDatalinkLs decodeUasDatalink(byte[] buf)
            throws org.tstrans.KlvDecodeException {
        return nDecodeUasDatalink(buf, false, false);
    }

    /**
     * Decode an ST 0601 UAS Datalink LS with optional strict / compliance mode.
     *
     * <p>{@code buf} is the full wire-format payload starting with the 16-byte UL.
     * When {@code compliance = true}, also enforces Tag 2 first / Tag 1 last /
     * Tag 65 present / no duplicate tags / canonical BER (implies {@code strict}).
     * When {@code strict = true} only, requires the ST 0601 family UL pattern.
     * Mirrors tst-py's {@code decode_uas_datalink(buf, strict=True, compliance=True)}.
     *
     * @param buf        full ST 0601 wire bytes (UL + BER length + body)
     * @param strict     {@code true} for strict UL validation
     * @param compliance {@code true} for full compliance validation (implies strict)
     * @return the decoded {@link UasDatalinkLs}
     * @throws org.tstrans.KlvDecodeException if the buffer is rejected
     */
    public static UasDatalinkLs decodeUasDatalink(byte[] buf, boolean strict, boolean compliance)
            throws org.tstrans.KlvDecodeException {
        return nDecodeUasDatalink(buf, strict, compliance);
    }

    /**
     * Encode a {@link UasDatalinkLs} to the full ST 0601 wire format.
     *
     * <p>Returns the full framing: {@code [UL:16][BER length][body][Tag1 checksum]}.
     * Encoding is lenient (emits only populated fields; no mandatory-tag enforcement).
     * Mirrors tst-py's {@code encode_uas_datalink(record)}.
     *
     * @param record the UAS Datalink LS to encode
     * @return ST 0601 wire bytes
     * @throws org.tstrans.KlvEncodeException if any field value is out of range or a
     *                                        reserved tag appears in {@code unknown}
     */
    public static byte[] encodeUasDatalink(UasDatalinkLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeUasDatalink(record);
    }

    /**
     * Encode a {@link UasDatalinkLs} with strict ST 0601 compliance validation.
     *
     * <p>Enforces mandatory-tag presence (Tag 2 precision timestamp, Tag 65 LS
     * version, Tag 1 checksum) and structural ordering rules. Mirrors tst-py's
     * {@code encode_uas_datalink_strict_compliance(record)}.
     *
     * @param record the UAS Datalink LS to encode
     * @return ST 0601 wire bytes with checksum
     * @throws org.tstrans.KlvEncodeException with {@code kind = MISSING_MANDATORY_ITEM}
     *                                        if a required tag is absent
     */
    public static byte[] encodeUasDatalinkStrictCompliance(UasDatalinkLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeUasDatalinkStrictCompliance(record);
    }

    private static native UasDatalinkLs nDecodeUasDatalink(byte[] buf, boolean strict, boolean compliance)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodeUasDatalink(UasDatalinkLs record)
            throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeUasDatalinkStrictCompliance(UasDatalinkLs record)
            throws org.tstrans.KlvEncodeException;

    // -----------------------------------------------------------------------
    // UL dispatcher
    // -----------------------------------------------------------------------

    /**
     * Inspect the first 16 bytes of {@code buf} (the SMPTE Universal Label)
     * and route to the matching typed decoder.
     *
     * <p>Returns:
     * <ul>
     *   <li>{@code Optional<UasDatalinkLs>} when the UL is in the ST 0601 family
     *       (first 13 bytes match + byte 15 is {@code 0x00}).</li>
     *   <li>{@code Optional<PrecisionTimeStampPack>} for the ST 0605 UL.</li>
     *   <li>{@code Optional<SecurityLs>} for the ST 0102 UL (peels UL + outer BER
     *       length, then calls {@link #decodeSecurity(byte[])} on the body).</li>
     *   <li>{@code Optional<VmtiLs>} for the ST 0903 UL (same peel + body decode).</li>
     *   <li>{@code Optional.empty()} for an unrecognised UL.</li>
     * </ul>
     *
     * <p>Mirrors {@code tstrans.klv.parse_klv_universal} in tst-py, including the
     * BER-peel error semantics for the body-only sets:
     * <ul>
     *   <li>{@code buf.length < 16}: throws {@code KlvDecodeException(BAD_UNIVERSAL_LABEL)}.</li>
     *   <li>BER-peel failure (truncated / indefinite-length): throws
     *       {@code KlvDecodeException(TRUNCATED_SET)}.</li>
     *   <li>Body overflow ({@code body_end > buf.length}): throws
     *       {@code KlvDecodeException(TRUNCATED_SET)}.</li>
     *   <li>Trailing bytes ({@code body_end < buf.length}): throws
     *       {@code KlvDecodeException(MALFORMED_BYTES)}.</li>
     * </ul>
     *
     * @param buf the full wire-format KLV record starting with the 16-byte UL
     * @return the decoded {@link KlvSet}, or {@code Optional.empty()} for unknown UL
     * @throws org.tstrans.KlvDecodeException if the buffer is too short for a UL,
     *         or if the BER-length peel or per-set decode fails
     */
    public static Optional<KlvSet> parseUniversal(byte[] buf)
            throws org.tstrans.KlvDecodeException {
        if (buf.length < 16) {
            throw new org.tstrans.KlvDecodeException(
                org.tstrans.KlvDecodeException.Kind.BAD_UNIVERSAL_LABEL,
                "buffer too short for 16-byte UL: have " + buf.length + " bytes");
        }
        byte[] ul = Arrays.copyOf(buf, 16);

        if (isSt0601Family(ul)) {
            return Optional.of(decodeUasDatalink(buf));
        }
        if (Arrays.equals(ul, PRECISION_TIMESTAMP_PACK_UL)) {
            return Optional.of(decodePrecisionTimestamp(buf));
        }
        if (Arrays.equals(ul, SECURITY_LS_UL)) {
            long[] berResult = peelBer(buf, 16, "ST 0102");
            long valueLen = berResult[0];
            long berBytes = berResult[1];
            long bodyStart = 16L + berBytes;
            long bodyEnd = bodyStart + valueLen;
            if (bodyEnd > buf.length) {
                throw new org.tstrans.KlvDecodeException(
                    org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                    "ST 0102 declared body length " + valueLen
                        + " exceeds available " + (buf.length - bodyStart));
            }
            if (bodyEnd < buf.length) {
                throw new org.tstrans.KlvDecodeException(
                    org.tstrans.KlvDecodeException.Kind.MALFORMED_BYTES,
                    "ST 0102 universal record has " + (buf.length - bodyEnd)
                        + " trailing bytes after declared body length " + valueLen);
            }
            byte[] body = Arrays.copyOfRange(buf, (int) bodyStart, (int) bodyEnd);
            return Optional.of(decodeSecurity(body));
        }
        if (Arrays.equals(ul, VMTI_LS_UL)) {
            long[] berResult = peelBer(buf, 16, "ST 0903");
            long valueLen = berResult[0];
            long berBytes = berResult[1];
            long bodyStart = 16L + berBytes;
            long bodyEnd = bodyStart + valueLen;
            if (bodyEnd > buf.length) {
                throw new org.tstrans.KlvDecodeException(
                    org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                    "ST 0903 declared body length " + valueLen
                        + " exceeds available " + (buf.length - bodyStart));
            }
            if (bodyEnd < buf.length) {
                throw new org.tstrans.KlvDecodeException(
                    org.tstrans.KlvDecodeException.Kind.MALFORMED_BYTES,
                    "ST 0903 universal record has " + (buf.length - bodyEnd)
                        + " trailing bytes after declared body length " + valueLen);
            }
            byte[] body = Arrays.copyOfRange(buf, (int) bodyStart, (int) bodyEnd);
            return Optional.of(decodeVmti(body));
        }

        return Optional.empty();
    }

    /**
     * Read a BER short/long-form length starting at {@code offset}.
     * Returns {@code long[] {value, bytesConsumed}}.
     * Mirrors {@code tstrans.klv._read_ber_length}.
     *
     * @throws org.tstrans.KlvDecodeException with {@code TRUNCATED_SET}
     *         if the BER encoding is truncated or uses indefinite-length form
     */
    private static long[] peelBer(byte[] buf, int offset, String setLabel)
            throws org.tstrans.KlvDecodeException {
        if (offset >= buf.length) {
            throw new org.tstrans.KlvDecodeException(
                org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                setLabel + " outer BER length unreadable: truncated BER length");
        }
        int first = buf[offset] & 0xFF;
        if (first < 0x80) {
            return new long[] {first, 1L};
        }
        int nbytes = first & 0x7F;
        if (nbytes == 0) {
            throw new org.tstrans.KlvDecodeException(
                org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                setLabel + " outer BER length unreadable: indefinite-length BER not permitted in KLV");
        }
        if (offset + 1 + nbytes > buf.length) {
            throw new org.tstrans.KlvDecodeException(
                org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                setLabel + " outer BER length unreadable: truncated BER long-form length");
        }
        long value = 0;
        for (int i = 0; i < nbytes; i++) {
            value = (value << 8) | (buf[offset + 1 + i] & 0xFF);
        }
        return new long[] {value, 1L + nbytes};
    }

    // -----------------------------------------------------------------------
    // Test-only forced-throw helpers (package-private)
    // -----------------------------------------------------------------------

    // Native entry points for test-only forced-throw paths. Package-private so
    // test classes in the same package can reach them; not part of the public API.
    static native void nRaiseDecodeForTest(String kind);
    static native void nRaiseEncodeForTest(String kind);

    /**
     * Force a {@link org.tstrans.KlvDecodeException} with the given
     * {@code Kind} name. Used by {@code KlvErrorModelTest} to exercise the
     * error-mapping wiring before real decode entry points exist.
     *
     * @param kind the {@code KlvDecodeException.Kind} constant name (e.g. {@code "TRUNCATED_SET"})
     * @throws org.tstrans.KlvDecodeException always
     */
    @SuppressWarnings("RedundantThrows")
    static void raiseDecodeForTest(String kind) throws org.tstrans.KlvDecodeException {
        nRaiseDecodeForTest(kind);
    }

    /**
     * Force a {@link org.tstrans.KlvEncodeException} with the given
     * {@code Kind} name. Used by {@code KlvErrorModelTest} to exercise the
     * error-mapping wiring before real encode entry points exist.
     *
     * @param kind the {@code KlvEncodeException.Kind} constant name (e.g. {@code "OUT_OF_RANGE"})
     * @throws org.tstrans.KlvEncodeException always
     */
    @SuppressWarnings("RedundantThrows")
    static void raiseEncodeForTest(String kind) throws org.tstrans.KlvEncodeException {
        nRaiseEncodeForTest(kind);
    }

    static {
        org.tstrans.NativeLoader.load();
    }
}
