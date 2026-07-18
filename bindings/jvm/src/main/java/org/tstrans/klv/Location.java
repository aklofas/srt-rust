package org.tstrans.klv;

/**
 * A WGS84 geodetic point: latitude, longitude, and Height Above Ellipsoid
 * (HAE). Shared by MISB ST 0601.19 §8.130 (Airbase Locations, see
 * {@link AirbaseLocations}) and §8.141 (Waypoint List, see {@link Waypoint}).
 *
 * <p>Wire shape (truncatable DLP, per §8.130.1 bullet 4): {@code lat} and
 * {@code lon} are a MANDATORY both-or-neither pair once any bytes are
 * present on the wire; only the trailing {@code hae} truncates
 * independently — see the Rust {@code Location} rustdoc for the full
 * wire-shape contract the encoder enforces.
 *
 * @param latDeg latitude in degrees, or {@code null} if this whole point is absent
 * @param lonDeg longitude in degrees, or {@code null} if this whole point is absent
 * @param haeM   Height Above Ellipsoid in metres, or {@code null} if truncated/absent
 */
public record Location(Double latDeg, Double lonDeg, Double haeM) {

    /** All three fields absent — the "fully unknown" point. */
    public Location() {
        this(null, null, null);
    }
}
