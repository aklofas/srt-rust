package org.tstrans.klv;

/**
 * Horizontal and vertical sensor field-of-view in degrees.
 *
 * <p>Derived from ST 0601 Items 19 and 20. Mirrors tst-py's
 * {@code tstrans.klv.FieldOfView}.
 *
 * @param horizontalDeg horizontal sensor FOV in degrees
 * @param verticalDeg   vertical sensor FOV in degrees
 */
public record FieldOfView(double horizontalDeg, double verticalDeg) {}
