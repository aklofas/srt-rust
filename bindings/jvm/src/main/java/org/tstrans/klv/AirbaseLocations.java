package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.130 Item 130 — the take-off and recovery site locations.
 *
 * <p>Per §8.130.1: a {@code recovery} absent entirely from the wire means
 * "same as take-off" — decode reflects that by setting {@code recovery}
 * equal to {@code takeOff}; only an explicit, <em>different</em>
 * {@code recovery} (including {@code null} while {@code takeOff} is set —
 * deliberately unknown, not "same as take-off") is distinguishable from
 * that default.
 *
 * @param takeOff  take-off site location, or {@code null} if unknown
 * @param recovery recovery site location, or {@code null} if unknown
 */
public record AirbaseLocations(Location takeOff, Location recovery) {}
