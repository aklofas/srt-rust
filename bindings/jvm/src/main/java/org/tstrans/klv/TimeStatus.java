package org.tstrans.klv;

/**
 * MISB ST 0603 §7.4 Table 3 Time Status byte wrapper.
 *
 * <p>Bit layout (MSB first):
 * <ul>
 *   <li>bit 7: 0 = Locked, 1 = Lock Unknown</li>
 *   <li>bit 6: 0 = Normal, 1 = Discontinuity</li>
 *   <li>bit 5: 0 = Forward, 1 = Reverse (only meaningful when bit 6 = 1)</li>
 *   <li>bits 4–0: reserved, must be {@code 0b11111}</li>
 * </ul>
 *
 * <p>Mirrors {@code tst_core::klv::st0605::TimeStatus}.
 */
public record TimeStatus(int raw) {

    /** Compact constructor: validates that {@code raw} is a single-byte value (0–255). */
    public TimeStatus {
        if (raw < 0 || raw > 0xFF) {
            throw new IllegalArgumentException(
                    "TimeStatus.raw must be 0..=255; got " + raw);
        }
    }

    /**
     * Returns {@code true} if bit 7 = 0 (clock locked to absolute time reference).
     * Mirrors Rust {@code TimeStatus::is_locked}.
     */
    public boolean isLocked() {
        return (raw & 0x80) == 0;
    }

    /**
     * Returns {@code true} if bit 6 = 1 (time has not incremented linearly
     * forward — i.e. a reset, jump, or correction occurred).
     * Mirrors Rust {@code TimeStatus::has_discontinuity}.
     */
    public boolean hasDiscontinuity() {
        return (raw & 0x40) != 0;
    }

    /**
     * Returns {@code true} if bit 5 = 1. Only meaningful when
     * {@link #hasDiscontinuity()} — indicates a backward time jump rather than
     * forward. Mirrors Rust {@code TimeStatus::is_reverse_jump}.
     */
    public boolean isReverseJump() {
        return (raw & 0x20) != 0;
    }

    /**
     * Returns {@code true} if reserved bits 4–0 are the spec-required
     * {@code 0b11111 = 0x1F}.
     * Mirrors Rust {@code TimeStatus::reserved_bits_valid}.
     */
    public boolean reservedBitsValid() {
        return (raw & 0x1F) == 0x1F;
    }
}
