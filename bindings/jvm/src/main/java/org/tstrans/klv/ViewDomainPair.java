package org.tstrans.klv;

/**
 * One {@code (start, range)} pair of MISB ST 0601.19 §8.142 Item 142, View
 * Domain. {@code startDeg} uses the axis-specific range (see
 * {@link ViewDomain}'s azimuth/elevation/roll fields); {@code rangeDeg}
 * always uses IMAPB(0, 360) and is always non-negative — "the angular range
 * specifies the limit from the starting point to the sensor's maximum value".
 *
 * @param startDeg axis start angle, degrees
 * @param rangeDeg angular range from the start, degrees, always &ge; 0
 */
public record ViewDomainPair(double startDeg, double rangeDeg) {}
