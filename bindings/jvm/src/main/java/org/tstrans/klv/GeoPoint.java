package org.tstrans.klv;

/**
 * Lat / lon / altitude triple derived from ST 0601 typed fields.
 *
 * <p>ST 0601 surfaces multiple {@code GeoPoint} views — sensor position
 * ({@link UasDatalinkLs#sensorPosition()}) and frame center
 * ({@link UasDatalinkLs#frameCenter()}). Altitude is metres AMSL unless the
 * source field specifies WGS-84 ellipsoid height. Mirrors tst-py's
 * {@code tstrans.klv.GeoPoint}.
 *
 * @param latDeg latitude in degrees (WGS-84)
 * @param lonDeg longitude in degrees (WGS-84)
 * @param altM   altitude in metres AMSL
 */
public record GeoPoint(double latDeg, double lonDeg, double altM) {}
