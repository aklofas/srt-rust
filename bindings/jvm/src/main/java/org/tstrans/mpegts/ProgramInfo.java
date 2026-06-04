package org.tstrans.mpegts;

import java.util.List;

/** One program's PMT summary. Mirrors a tst-py ProgramMap entry. */
public record ProgramInfo(int programNumber, int pmtPid, List<Integer> elementaryPids) {}
