package org.tstrans.mpegts;

/**
 * Opaque handle for a video elementary stream within a configured muxer.
 *
 * <p>Obtain one from {@code MuxSender.videoHandle()} (or the per-program handle
 * accessors) and pass it to {@code MuxSender.sendVideoTo(...)} to target a
 * specific stream. The {@code raw} value is the muxer's packed {@code u32}
 * stream identifier widened to {@code long}; it is meaningful only to the muxer
 * that minted it. A handle from a different muxer (or an out-of-range raw value)
 * is rejected by the native {@code try_from_raw} path and surfaces as
 * {@code SrtException(CONFIG_INVALID)} at push time.
 *
 * <p>Mirrors {@code tstrans.mpegts.VideoStreamHandle}. Being a record, equality
 * and hashing are by {@code raw}.
 *
 * @param raw the packed {@code u32} stream identifier, widened to {@code long}
 */
public record VideoStreamHandle(long raw) {
    /**
     * Reconstruct a handle from a raw value previously returned by
     * {@code MuxSender.videoHandle().raw()}. No validation occurs here —
     * a malformed or out-of-range handle is rejected by the native push path.
     *
     * @param raw the packed {@code u32} value, widened to {@code long}
     * @return a {@code VideoStreamHandle} wrapping {@code raw}
     */
    public static VideoStreamHandle fromRaw(long raw) {
        return new VideoStreamHandle(raw);
    }
}
