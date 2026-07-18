package org.tstrans.klv;

/**
 * One record of MISB ST 0601.19 §8.141 Item 141, Waypoint List.
 *
 * @param id               BER-OID waypoint id
 * @param prosecutionOrder 0 = current, &gt;0 = planned, &lt;0 = historical,
 *                         {@code 0x7FFF} = cancelled
 * @param info             Mode/Source bitfield (b0 = manual mode, b1 = adhoc
 *                         source), or {@code null} if absent
 * @param location         waypoint location, or {@code null} if absent
 */
public record Waypoint(long id, int prosecutionOrder, Long info, Location location) {}
