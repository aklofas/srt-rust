package org.tstrans.pipeline;

import org.tstrans.mpegts.DemuxEvent;

/** One emission from {@link Pairer#feed}/{@link Pairer#flush}. Match on the
 *  nested records (JDK 17 pattern matching). Mirrors
 *  {@code tstrans.pipeline.PairerOutput}. */
public sealed interface PairerOutput
        permits PairerOutput.Paired, PairerOutput.UnpairedVideo,
                PairerOutput.UnpairedKlv, PairerOutput.PassThrough {
    /** Video and KLV matched within tolerance. */
    record Paired(VideoSample video, KlvSample klv) implements PairerOutput {}
    /** A video sample with no KLV match within tolerance/buffer. */
    record UnpairedVideo(VideoSample video) implements PairerOutput {}
    /** A KLV sample with no video match. */
    record UnpairedKlv(KlvSample klv) implements PairerOutput {}
    /** Any {@code DemuxEvent} not on the configured video/klv PID, or a
     *  shape-mismatched sample on a configured PID. */
    record PassThrough(DemuxEvent event) implements PairerOutput {}
}
