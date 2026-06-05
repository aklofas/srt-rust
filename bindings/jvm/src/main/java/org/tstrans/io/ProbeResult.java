package org.tstrans.io;

import java.util.List;
import org.tstrans.mpegts.AudioCodec;
import org.tstrans.mpegts.DemuxEvent;
import org.tstrans.mpegts.SubtitleCodec;
import org.tstrans.mpegts.VideoCodec;

/**
 * First-N-MiB scan summary produced by {@link Io#probe}. Mirrors tst-py's
 * {@code tstrans.io.ProbeResult}. Cheaper than a full parse — enough for
 * "what's in this file?" introspection. Does NOT compute duration.
 *
 * <p><b>Classification source.</b> Unlike tst-py (which reads codec/KLV info from
 * the PMT in {@code ProgramMap.streams}), the JVM {@link DemuxEvent.ProgramMap}
 * exposes only {@code elementaryPids}; codec sets and {@link #hasKlv()} are
 * therefore derived from the {@link DemuxEvent.Video}/{@code Audio}/{@code Subtitle}/
 * {@code Metadata} events observed during the scan. Equivalent for any file with
 * samples in the probe window.
 */
public record ProbeResult(
    long sizeBytes,
    List<DemuxEvent.ProgramMap> programs,
    List<Integer> pids,
    List<VideoCodec> videoCodecs,
    List<AudioCodec> audioCodecs,
    List<SubtitleCodec> subtitleCodecs,
    boolean hasKlv,
    long packetCount
) {}
