package org.tstrans.codec;

/**
 * Sealed marker for a typed audio elementary-stream frame.
 *
 * <p>Permits {@link AdtsFrame} (AAC) and {@link Mpeg2AudioFrame} (MPEG-1/2/2.5
 * Layer I/II/III audio).
 *
 * <p>Use {@code instanceof} pattern matching on JDK 17 to dispatch on the
 * concrete type; {@code switch}-on-sealed requires JDK 21+.
 */
public sealed interface AudioFrame permits AdtsFrame, Mpeg2AudioFrame {}
