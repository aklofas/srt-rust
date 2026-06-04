package org.tstrans.klv;

/**
 * Four corner {@link LatLon} points of the sensor footprint (upper-left looking
 * forward). Derived from ST 0601 absolute corner tags 82–89 when fully populated,
 * otherwise from frame-center + offset tags 26–33. Mirrors tst-py's
 * {@code tstrans.klv.Corners}.
 *
 * @param p1 upper-left corner
 * @param p2 upper-right corner
 * @param p3 lower-right corner
 * @param p4 lower-left corner
 */
public record Corners(LatLon p1, LatLon p2, LatLon p3, LatLon p4) {}
