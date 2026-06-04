package org.tstrans.klv;

/**
 * MISB ST 0605 §7 Precision Time Stamp Pack typed view.
 *
 * <p>Wire form is 26 bytes: 16-byte UL + 1-byte BER length ({@code 0x09}) +
 * 1-byte {@link TimeStatus} + 8-byte big-endian microsecond timestamp.
 *
 * <p>Mirrors {@code tst_core::klv::st0605::PrecisionTimeStampPack}.
 *
 * @param timeStatus  time-lock and discontinuity flags
 * @param timestampUs microseconds since 1970-01-01T00:00:00Z (POSIX epoch)
 */
public record PrecisionTimeStampPack(TimeStatus timeStatus, long timestampUs)
        implements KlvSet {}
