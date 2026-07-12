package org.tstrans.klv;

/**
 * A decoded MISB ST 1204.3 MIIS Core Identifier.
 *
 * <p>Contains the wire version byte plus up to four optional UUID components.
 * Absent components are {@code null}. The ST 1204.3 EBNF constraint —
 * {@code minor} XOR any-of-(sensorId/platformId/windowId) — is enforced by the
 * Rust decoder; callers constructing a {@code CoreId} for encoding must maintain it.
 *
 * <p>Decode via {@link Klv#decodeCoreId(byte[])}; encode via
 * {@link Klv#encodeCoreId(CoreId)}; render as text via {@link Klv#coreIdText(CoreId)}.
 *
 * @param version     wire version byte; always {@code 1} for decoded values
 * @param sensorType  sensor UUID IdType, or {@code null} when sensor UUID is absent
 * @param sensorId    16-byte sensor UUID, or {@code null} when absent
 * @param platformType platform UUID IdType, or {@code null} when platform UUID is absent
 * @param platformId  16-byte platform UUID, or {@code null} when absent
 * @param windowId    16-byte window UUID, or {@code null} when absent (no type bits)
 * @param minorId     16-byte minor Core Identifier UUID, or {@code null} when absent;
 *                    when present, all other UUID fields must be {@code null}
 */
public record CoreId(
        int version,
        IdType sensorType,
        byte[] sensorId,
        IdType platformType,
        byte[] platformId,
        byte[] windowId,
        byte[] minorId
) {}
