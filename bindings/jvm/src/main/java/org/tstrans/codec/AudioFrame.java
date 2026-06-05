package org.tstrans.codec;

/**
 * Sealed marker for a typed audio elementary-stream frame.
 *
 * <p>Currently permits {@link AdtsFrame} (AAC). The MPEG-2 audio frame type
 * ({@code Mpeg2AudioFrame}) lands in a follow-on task of the codec wave and will
 * be added to the {@code permits} clause then.
 *
 * <p>Use {@code instanceof} pattern matching on JDK 17 to dispatch on the
 * concrete type; {@code switch}-on-sealed requires JDK 21+.
 */
public sealed interface AudioFrame permits AdtsFrame {}
