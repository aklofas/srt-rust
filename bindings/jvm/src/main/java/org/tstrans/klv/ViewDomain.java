package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.142 Item 142 — up to three {@link ViewDomainPair}s
 * (azimuth, elevation, roll, in that fixed order); a pair absent from the
 * wire (either an explicit zero-length marker or trailing truncation)
 * decodes to {@code null}.
 *
 * @param azimuth   azimuth (start, range) pair, or {@code null} if absent
 * @param elevation elevation (start, range) pair, or {@code null} if absent
 * @param roll      roll (start, range) pair, or {@code null} if absent
 */
public record ViewDomain(ViewDomainPair azimuth, ViewDomainPair elevation, ViewDomainPair roll) {}
