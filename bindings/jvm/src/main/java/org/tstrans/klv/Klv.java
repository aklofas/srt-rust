package org.tstrans.klv;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.List;
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

    /**
     * Encode a {@link SecurityLs} with strict ST 0102 compliance validation.
     *
     * <p>Enforces mandatory-tag presence (Tags 1, 2, 3, 12, 13, 22 per ST 0102.12
     * §6.7 Table 2) before encoding. Mirrors tst-py's
     * {@code encode_security_strict_compliance(record)}.
     *
     * @param record the Security LS to encode
     * @return ST 0102 body bytes
     * @throws org.tstrans.KlvEncodeException with {@code kind = MISSING_MANDATORY_ITEM}
     *                                        if a required tag is absent
     */
    public static byte[] encodeSecurityStrictCompliance(SecurityLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeSecurityStrictCompliance(record);
    }

    private static native SecurityLs nDecodeSecurity(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodeSecurity(SecurityLs record)
            throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeSecurityStrictCompliance(SecurityLs record)
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

    /**
     * Encode a {@link VmtiLs} to ST 0903 body bytes with strict compliance validation.
     *
     * <p>Returns embedded body bytes (no UL, no outer BER length, no Tag 1 checksum).
     * Enforces mandatory items (Tags 4 and 6), non-empty VTargetPacks, and unique
     * target IDs before encoding. Mirrors tst-py's
     * {@code encode_vmti_strict_compliance(record)}.
     *
     * @param record the VMTI LS to encode
     * @return ST 0903 embedded body bytes
     * @throws org.tstrans.KlvEncodeException with {@code kind = MISSING_MANDATORY_ITEM} if
     *                                        a required item is absent, or
     *                                        {@code kind = VTARGET_PACK_EMPTY} if a pack has
     *                                        no TLV items, or
     *                                        {@code kind = DUPLICATE_TARGET_ID} if target IDs
     *                                        are not unique
     */
    public static byte[] encodeVmtiStrictCompliance(VmtiLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeVmtiStrictCompliance(record);
    }

    /**
     * Encode a {@link VmtiLs} as a standalone VMTI wire record with strict compliance validation.
     *
     * <p>Returns the full framing: {@code [VMTI_LS_UL:16][outer BER length][body][Tag1 checksum]}.
     * In addition to the embedded-mode checks, enforces standalone-required items (Tags 2, 11,
     * 12, 13) and rejects offset tags 10/11/13/14/15/16 on any VTargetPack
     * (ST 0903.6-116 forbidden). Mirrors tst-py's
     * {@code encode_vmti_standalone_strict_compliance(record)}.
     *
     * @param record the VMTI LS to encode
     * @return the full standalone VMTI wire record
     * @throws org.tstrans.KlvEncodeException with {@code kind = MISSING_MANDATORY_ITEM} if a
     *                                        required item is absent, or
     *                                        {@code kind = FORBIDDEN_STANDALONE_OFFSET} if any
     *                                        pack carries a forbidden offset tag
     */
    public static byte[] encodeVmtiStandaloneStrictCompliance(VmtiLs record)
            throws org.tstrans.KlvEncodeException {
        return nEncodeVmtiStandaloneStrictCompliance(record);
    }

    private static native VmtiLs nDecodeVmti(byte[] buf, boolean strict)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] nEncodeVmti(VmtiLs record) throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeVmtiStandalone(VmtiLs record)
            throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeVmtiStrictCompliance(VmtiLs record)
            throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeVmtiStandaloneStrictCompliance(VmtiLs record)
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
     * Encode a {@link UasDatalinkLs} to the full ST 0601 wire format using the
     * default {@link OutOfRangePolicy#ERROR} policy.
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
        return encodeUasDatalink(record, OutOfRangePolicy.ERROR);
    }

    /**
     * Encode a {@link UasDatalinkLs} to the full ST 0601 wire format with an
     * explicit {@link OutOfRangePolicy}.
     *
     * <p>Returns the full framing: {@code [UL:16][BER length][body][Tag1 checksum]}.
     * Encoding is lenient (emits only populated fields; no mandatory-tag enforcement).
     * Mirrors tst-py's {@code encode_uas_datalink(record, out_of_range_policy=...)}.
     *
     * <p>When {@code policy} is {@link OutOfRangePolicy#INDICATOR}, out-of-range
     * values on tags whose ST 0601 INT_MIN sentinel means "Out of Range"
     * (Tags 6, 7, 50, 51, 52, 79, 80, 90–93 — all of which are encodable
     * typed fields) are replaced by the spec-defined special value rather
     * than throwing. All other tags and non-finite inputs still throw.
     *
     * @param record the UAS Datalink LS to encode
     * @param policy how to handle out-of-range field values
     * @return ST 0601 wire bytes
     * @throws org.tstrans.KlvEncodeException if any field value is out of range
     *                                        (and not eligible for INDICATOR), or a
     *                                        reserved tag appears in {@code unknown}
     */
    public static byte[] encodeUasDatalink(UasDatalinkLs record, OutOfRangePolicy policy)
            throws org.tstrans.KlvEncodeException {
        int policyInt = switch (policy) {
            case ERROR -> 0;
            case INDICATOR -> 1;
        };
        return nEncodeUasDatalinkWithPolicy(record, policyInt);
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

    private static native byte[] nEncodeUasDatalinkWithPolicy(UasDatalinkLs record, int policy)
            throws org.tstrans.KlvEncodeException;

    private static native byte[] nEncodeUasDatalinkStrictCompliance(UasDatalinkLs record)
            throws org.tstrans.KlvEncodeException;

    // -----------------------------------------------------------------------
    // ST 1204 — MIIS Core Identifier
    // -----------------------------------------------------------------------

    /**
     * Decode a MIIS Core Identifier from its binary wire form (ST 1204.3 §7.3).
     *
     * <p>Expects exactly the bytes of one Core Identifier — no framing, no BER
     * length wrapper. Rejects trailing bytes, unsupported version bytes,
     * reserved-bit violations, and invalid usage-byte combinations.
     * Mirrors tst-py's {@code decode_core_id(buf)}.
     *
     * @param klv raw Core Identifier bytes (no UL / outer BER length)
     * @return the decoded {@link CoreId}
     * @throws org.tstrans.KlvDecodeException if the buffer is malformed or
     *         contains an unsupported version or invalid usage byte
     */
    public static CoreId decodeCoreId(byte[] klv) throws org.tstrans.KlvDecodeException {
        return decodeCoreIdNative(klv);
    }

    /**
     * Encode a {@link CoreId} to its binary wire form (ST 1204.3 §7.3).
     *
     * <p>Returns the two-byte header (version + usage) followed by the UUIDs in
     * EBNF order: sensor, platform, window, minor. Encoding is infallible;
     * the caller is responsible for maintaining the ST 1204.3 EBNF constraint
     * ({@code minorId} must be {@code null} when any other UUID field is present).
     * Mirrors tst-py's {@code encode_core_id(id)}.
     *
     * @param id the Core Identifier to encode
     * @return binary wire bytes (no UL / outer BER length)
     */
    public static byte[] encodeCoreId(CoreId id) {
        return encodeCoreIdNative(id);
    }

    /**
     * Return the ST 1204.3 §7.4.2 textual representation of a {@link CoreId}.
     *
     * <p>Format: {@code VVUU:XXXX-XXXX-…/XXXX-XXXX-…:CC} where {@code VV} and
     * {@code UU} are the version and usage bytes as uppercase hex, each UUID is
     * 8 groups of 4 hex chars dash-separated, multiple UUIDs are {@code /}-separated,
     * and {@code CC} is the Appendix B check value.
     * Mirrors tst-py's {@code core_id_text(id)}.
     *
     * @param id the Core Identifier to format
     * @return the ST 1204.3 textual representation
     */
    public static String coreIdText(CoreId id) {
        return coreIdTextNative(id);
    }

    /**
     * Validate a {@link UasDatalinkLs} record against the ST 0902.8 Minimum
     * Metadata Set (MISMMS, Table 1).
     *
     * <p>Returns all violations found; an empty list means the record satisfies
     * every MISMMS requirement at the record level. Violations are instances of
     * {@link MismmsViolation} with {@code kind} ∈ {@code {"missing",
     * "missing_security", "zero_length", "alternation_conflict"}}.
     *
     * <p>The Tag 48 Security Local Set sub-item check decodes the security bytes via
     * ST 0102 and verifies the 9 required sub-items (classification, classifying
     * country coding method, classifying country, SCI/SHI info, caveats, releasing
     * instructions, object country coding method, object country codes, version).
     *
     * <p>Mirrors tst-py's {@code validate_mismms(record)}.
     *
     * @param record the UAS Datalink LS to validate
     * @return a list of all MISMMS violations (empty if compliant)
     */
    public static List<MismmsViolation> validateMismms(UasDatalinkLs record) {
        return validateMismmsNative(record);
    }

    private static native CoreId decodeCoreIdNative(byte[] klv)
            throws org.tstrans.KlvDecodeException;

    private static native byte[] encodeCoreIdNative(CoreId id);

    private static native String coreIdTextNative(CoreId id);

    private static native List<MismmsViolation> validateMismmsNative(UasDatalinkLs record);

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
        if (nbytes > 8) {
            throw new org.tstrans.KlvDecodeException(
                org.tstrans.KlvDecodeException.Kind.TRUNCATED_SET,
                setLabel + " outer BER length unreadable: long-form length exceeds 8 bytes");
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
