package org.tstrans.klv;

/**
 * One record of MISB ST 0601.19 §8.140 Item 140, Weapons Stores — a single
 * weapon's physical address, status, and type. {@code statusRaw} packs the
 * spec's 14-bit Status BER-OID value verbatim: the low 8 bits are the
 * §Table 21 General Status enumeration, the next 4 bits are the §Table 22
 * Engagement Status flags, and any remaining high bits are spec-reserved
 * (preserved verbatim, not masked away, in case a future revision widens
 * the field). The accessor methods below decode its sub-fields.
 *
 * @param stationId   BER-OID station id
 * @param hardpointId BER-OID hardpoint id
 * @param carriageId  BER-OID carriage id
 * @param storeId     BER-OID store id
 * @param statusRaw   raw 14-bit Status BER-OID value
 * @param weaponType  weapon type name
 */
public record WeaponsStore(
        long stationId,
        long hardpointId,
        long carriageId,
        long storeId,
        long statusRaw,
        String weaponType
) {

    /** §Table 21 General Status code (low 8 bits of {@code statusRaw}). */
    public int generalStatus() {
        return (int) (statusRaw & 0xFF);
    }

    /** §Table 22 bit position 1. */
    public boolean fuzeEnabled() {
        return (statusRaw & 0x100) != 0;
    }

    /** §Table 22 bit position 2. */
    public boolean laserEnabled() {
        return (statusRaw & 0x200) != 0;
    }

    /** §Table 22 bit position 3. */
    public boolean targetEnabled() {
        return (statusRaw & 0x400) != 0;
    }

    /** §Table 22 bit position 4. */
    public boolean weaponArmed() {
        return (statusRaw & 0x800) != 0;
    }
}
