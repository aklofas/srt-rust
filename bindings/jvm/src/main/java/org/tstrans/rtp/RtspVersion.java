package org.tstrans.rtp;

/**
 * Wire-time RTSP version. Mirrors tst-py {@code tstrans.rtp.RtspVersion}.
 *
 * <p><b>Currently informational on this binding:</b> {@code RtspClient.connect}
 * does not plumb {@code RtspClientConfig.rtspVersion} into the native builder —
 * tst-rtp derives the version from the {@code rtsp://} vs {@code rtsps://} URL
 * scheme, not from this enum (matching tst-py). The value round-trips through the
 * config but does not yet change behavior.
 */
public enum RtspVersion { V1_0, V2_0 }
