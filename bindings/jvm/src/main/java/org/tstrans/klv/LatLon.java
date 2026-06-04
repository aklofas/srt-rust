package org.tstrans.klv;

/**
 * A (lat, lon) point in degrees. Used as the corner-point type in
 * {@link Corners}. Mirrors tst-py's {@code tuple[float, float]} corner tuples.
 *
 * @param latDeg latitude in degrees (WGS-84)
 * @param lonDeg longitude in degrees (WGS-84)
 */
public record LatLon(double latDeg, double lonDeg) {}
