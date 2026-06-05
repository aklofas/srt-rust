package org.tstrans.mpegts;

/**
 * Opaque handle for an audio elementary stream within a configured muxer.
 *
 * <p>Obtain one from {@code MuxSender.audioHandle()} and pass it to
 * {@code MuxSender.pushAudioTo(...)} to target a specific stream. The
 * {@code raw} value is the muxer's packed {@code u32} stream identifier widened
 * to {@code long}; it is meaningful only to the muxer that minted it. A handle
 * from a different muxer (or an out-of-range raw value) is rejected by the
 * native {@code try_from_raw} path and surfaces as
 * {@code SrtException(CONFIG_INVALID)} at push time.
 *
 * <p>Mirrors {@code tstrans.mpegts.AudioStreamHandle}. Being a record, equality
 * and hashing are by {@code raw}.
 *
 * @param raw the packed {@code u32} stream identifier, widened to {@code long}
 */
public record AudioStreamHandle(long raw) {
    /**
     * Reconstruct a handle from a raw value previously returned by
     * {@code MuxSender.audioHandle().raw()}. No validation occurs here —
     * a forged or cross-muxer value is rejected by the native push path.
     *
     * @param raw the packed {@code u32} value, widened to {@code long}
     * @return an {@code AudioStreamHandle} wrapping {@code raw}
     */
    public static AudioStreamHandle fromRaw(long raw) {
        return new AudioStreamHandle(raw);
    }
}
