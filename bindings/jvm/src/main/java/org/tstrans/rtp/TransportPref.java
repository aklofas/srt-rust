package org.tstrans.rtp;

/**
 * Transport preference at SETUP. {@code AUTO} = UDP-first with TCP fallback on
 * 461; {@code UDP}/{@code TCP} would force a single transport. Mirrors tst-py
 * {@code tstrans.rtp.TransportPref}.
 *
 * <p><b>Currently informational on this binding:</b> {@code RtspClient.connect}
 * does not plumb {@code RtspClientConfig.transportPref} into the native connect —
 * tst-rtp derives the transport from a {@code ?transport=udp|tcp} URL query, not
 * from this enum (matching tst-py). The value round-trips through the config but
 * does not yet change behavior.
 */
public enum TransportPref { AUTO, UDP, TCP }
