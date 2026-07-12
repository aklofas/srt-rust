package org.tstrans.klv;

/**
 * A violation of the ST 0902.8 Minimum Metadata Set (MISMMS) requirements,
 * as returned by {@link Klv#validateMismms(UasDatalinkLs)}.
 *
 * <p>The {@code kind} field discriminates the violation type:
 * <ul>
 *   <li>{@code "missing"} — a required MISMMS item (Tag {@link #tag()}, human-readable
 *       {@link #name()}) is absent from the record. For Tag 48, this also covers the case
 *       where the Security Local Set bytes are present but fail to decode.</li>
 *   <li>{@code "missing_security"} — a required sub-item of the ST 0102 Security Local
 *       Set (Tag 48) is absent. {@link #tag()} is the ST 0102 item number;
 *       {@link #name()} is its label.</li>
 *   <li>{@code "zero_length"} — Tag {@link #tag()} is present in the record's unknown
 *       list but its wire value is zero-length, which does NOT satisfy MISMMS presence
 *       (ST 0902.8-05). {@link #name()} is {@code null}.</li>
 *   <li>{@code "alternation_conflict"} — Tags 75 and 104 are both present. Within the
 *       {@code 15|75|104} group, Tags 75 and 104 are exclusive-or. {@link #tag()} is
 *       the first conflicting tag (75); {@link #tagB()} is the second (104);
 *       {@link #name()} is {@code null}.</li>
 * </ul>
 *
 * @param kind  violation discriminant: {@code "missing"}, {@code "missing_security"},
 *              {@code "zero_length"}, or {@code "alternation_conflict"}
 * @param tag   primary tag number involved in the violation
 * @param name  human-readable tag name; non-null for {@code missing} and
 *              {@code missing_security} kinds; {@code null} otherwise
 * @param tagB  second tag number; non-zero only for {@code "alternation_conflict"}
 *              (carries 104); zero for all other kinds
 */
public record MismmsViolation(
        String kind,
        int tag,
        String name,
        int tagB
) {}
