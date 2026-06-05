package org.tstrans.io;

import org.tstrans.klv.KlvSet;

/**
 * One item yielded by {@link Io#extractKlv}. Models tst-py's four return modes
 * (raw bytes / {@code (pts, bytes)} / typed set / {@code (pts, typed)}) as a single
 * record:
 * <ul>
 *   <li>{@code pts} — the 90&nbsp;kHz timestamp, or {@code null} when
 *       {@code withPts=false}.</li>
 *   <li>{@code raw} — the raw KLV LS bytes when {@code parsed=false}, else
 *       {@code null}.</li>
 *   <li>{@code parsed} — the typed {@link KlvSet} when {@code parsed=true} (may be
 *       {@code null} for an unrecognized UL when {@code skipUnknown=false}), else
 *       {@code null}.</li>
 * </ul>
 */
public record KlvEntry(Long pts, byte[] raw, KlvSet parsed) {}
