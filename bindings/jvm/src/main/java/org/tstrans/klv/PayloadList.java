package org.tstrans.klv;

import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0601.19 §8.138 Item 138 — Payload List: a declared {@code count}
 * plus the payload records themselves.
 *
 * @param count   declared BER-OID Payload Count
 * @param records the payload records
 */
public record PayloadList(long count, List<PayloadRecord> records) {

    /** Compact constructor makes {@code records} non-null and immutable. */
    public PayloadList {
        records = records == null ? Collections.emptyList() : Collections.unmodifiableList(records);
    }
}
