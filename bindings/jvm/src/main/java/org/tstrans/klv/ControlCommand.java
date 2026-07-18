package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.115 Item 115 — Control Command. MULTI-INSTANCE per
 * ST 0601.19 Table 1 ("Multiples Allowed" = Yes): every wire occurrence
 * appends one {@code ControlCommand} to {@link UasDatalinkLs#controlCommands()}.
 *
 * @param id      BER-OID command id
 * @param command command text, UTF-8, at most 127 bytes
 * @param timeUs  time the command was issued/executed, microseconds, or
 *                {@code null} if the wire pack ended before this trailing field
 */
public record ControlCommand(long id, String command, Long timeUs) {}
