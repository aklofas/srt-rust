package org.tstrans.pipeline;

import org.tstrans.mpegts.DemuxerConfig;

/** Bundles the two halves' configs. {@code demuxer} may be null → demuxer
 *  defaults. Mirrors {@code tstrans.pipeline.PairingDemuxerConfig}. */
public record PairingDemuxerConfig(PairerConfig pairer, DemuxerConfig demuxer) {
    public PairingDemuxerConfig {
        if (pairer == null) throw new IllegalArgumentException("pairer must be non-null");
    }
}
