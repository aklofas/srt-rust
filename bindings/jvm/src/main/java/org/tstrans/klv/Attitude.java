package org.tstrans.klv;

/**
 * Three-axis attitude in degrees derived from ST 0601 typed fields.
 *
 * <p>ST 0601 surfaces multiple {@code Attitude} views — sensor relative
 * attitude ({@link UasDatalinkLs#sensorAttitude()}) and platform attitude
 * ({@link UasDatalinkLs#platformAttitude()}). Mirrors tst-py's
 * {@code tstrans.klv.Attitude}.
 *
 * @param headingDeg heading (or relative azimuth) in degrees
 * @param pitchDeg   pitch (or relative elevation) in degrees
 * @param rollDeg    roll in degrees
 */
public record Attitude(double headingDeg, double pitchDeg, double rollDeg) {}
