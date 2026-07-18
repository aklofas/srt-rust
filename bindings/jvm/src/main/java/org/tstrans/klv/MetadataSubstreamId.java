package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.143 Item 143 — Metadata Substream Id. Per §8.143,
 * {@code uuid} is REQUIRED when {@code localId == 0} and OMITTED when
 * {@code localId > 0}; this binding is lenient and stores whatever
 * combination the wire carries.
 *
 * @param localId BER-OID local substream id
 * @param uuid    16-byte RFC 4122 UUID, or {@code null} if absent
 */
public record MetadataSubstreamId(long localId, byte[] uuid) {

    /** Compact constructor validates {@code uuid} is exactly 16 bytes when present. */
    public MetadataSubstreamId {
        if (uuid != null && uuid.length != 16) {
            throw new IllegalArgumentException(
                    "MetadataSubstreamId.uuid must be 16 bytes; got " + uuid.length);
        }
    }
}
