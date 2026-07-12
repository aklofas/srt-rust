package org.tstrans.klv;

/**
 * Source-type discriminant for a sensor or platform UUID within a {@link CoreId}.
 *
 * <p>Maps to the two-bit field in the MISB ST 1204.3 §7.3.1 Table 3 usage byte:
 * {@code 11} → Physical, {@code 10} → Virtual, {@code 01} → Managed.
 * Mirrors Rust {@code tst_core::klv::st1204::IdType}.
 */
public enum IdType {
    /** Identifies a physical (hardware) sensor or platform. */
    PHYSICAL,
    /** Identifies a virtual (software-defined) sensor or platform. */
    VIRTUAL,
    /** Identifies a managed (assigned/registered) sensor or platform. */
    MANAGED
}
