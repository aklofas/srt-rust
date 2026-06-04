package org.tstrans.klv;

import java.util.HexFormat;

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
