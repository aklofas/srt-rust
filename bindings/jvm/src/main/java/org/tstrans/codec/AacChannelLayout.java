package org.tstrans.codec;

/**
 * AAC channel layout decoded from the ADTS {@code channel_configuration} field
 * (ISO/IEC 14496-3 Table 1.19). Mirrors
 * {@code tst_core::codec::aac::AacChannelLayout} as flattened by tst-py's
 * {@code tstrans.codec.AacChannelLayout}.
 *
 * <p>The Rust type is a tagged enum ({@code PceDefined | Channels(u8)}). tst-py
 * flattens it to an {@code is_pce_defined} flag plus an optional channel count
 * rather than a tagged union; this binding mirrors that flattened shape exactly:
 *
 * <ul>
 *   <li>{@code pceDefined() == true} — the channel layout is carried in a
 *       Program Config Element (PCE) inside the raw data block; not derivable
 *       from the ADTS header alone, so {@link #channels()} is {@code null}.</li>
 *   <li>{@code pceDefined() == false} — canonical channel count from
 *       {@code channel_configuration} {@code 1..=7} (index {@code 7} is 8
 *       channels / 7.1); {@link #channels()} carries the count.</li>
 * </ul>
 *
 * @param pceDefined {@code true} when the layout is PCE-defined
 *                   ({@code channel_configuration == 0})
 * @param channels   canonical channel count, or {@code null} when PCE-defined
 */
public record AacChannelLayout(boolean pceDefined, Integer channels) {}
