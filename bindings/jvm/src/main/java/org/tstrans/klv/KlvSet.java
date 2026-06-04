package org.tstrans.klv;

/**
 * Sealed marker for the four typed KLV sets returned by
 * {@link Klv#parseUniversal(byte[])}.
 *
 * <p>The four permitted types are the MISB typed sets:
 * {@link UasDatalinkLs} (ST 0601), {@link SecurityLs} (ST 0102),
 * {@link PrecisionTimeStampPack} (ST 0605), and {@link VmtiLs} (ST 0903).
 * Use {@code instanceof} pattern matching on JDK 17 to dispatch on the
 * concrete type; {@code switch}-on-sealed requires JDK 21+.
 */
public sealed interface KlvSet permits UasDatalinkLs, SecurityLs, PrecisionTimeStampPack, VmtiLs {}
