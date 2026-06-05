package org.tstrans.codec;

/**
 * Sealed marker for a typed elementary-stream payload unit — either a
 * {@link NalUnit} (H.264/H.265/H.266) or an {@link Obu} (AV1).
 *
 * <p>Use {@code instanceof} pattern matching on JDK 17 to dispatch on the
 * concrete type; {@code switch}-on-sealed requires JDK 21+.
 */
public sealed interface VideoUnit permits NalUnit, Obu {}
