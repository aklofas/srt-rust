package org.tstrans.codec;

/**
 * Static facade for typed codec parameter-set / payload-unit parsing.
 * Mirrors tst-py's {@code tstrans.codec} free functions.
 *
 * <p>Parser entry points (H.264 / H.265 / H.266 / AV1 / audio) are added in the
 * follow-on tasks of the codec wave; this shell exists so the shared value types
 * and the native-library load are in place.
 */
public final class Codec {
    private Codec() {}

    static {
        org.tstrans.NativeLoader.load();
    }
}
