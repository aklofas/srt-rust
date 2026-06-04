package org.tstrans.klv;

import java.nio.ByteBuffer;
import java.util.Collections;
import java.util.List;

/**
 * MISB ST 0903.6 VMTI (Video Moving Target Indicator) Local Set typed view.
 *
 * <p>Required tags per ST 0903.6 §6 Table 1: {@code precisionTimeStamp},
 * {@code vmtiSystemName} (when applicable), {@code versionNumber},
 * {@code frameWidth}, {@code frameHeight}. Lenient decode
 * ({@link Klv#decodeVmti(byte[])}) tolerates missing required tags and surfaces
 * per-field decode failures in {@link #fieldErrors()}. Strict decode
 * ({@link Klv#decodeVmti(byte[], boolean) decodeVmti(buf, true)}) rejects missing
 * required tags.
 *
 * <p>{@code algorithmSeries} and {@code ontologySeries} are top-level nested LS
 * pass-through bytes (typed inner layers deferred). {@code miisId} is the MISB
 * ST 1204 Minor Item Identification System Core Identifier — pass-through bytes
 * (typed layer deferred).
 *
 * <p>Integer widths match {@code tst_core::klv::st0903::VmtiLs}: u32 fields are
 * stored as {@code Long} to avoid sign issues; u16 fields are {@code Integer}.
 *
 * <p>Instances are immutable; use {@link Builder} to construct.
 */
public record VmtiLs(
        // Tag 1 — checksum (u16; observability for standalone VMTI; dropped by encode)
        Integer checksum,
        // Tag 2 — precisionTimeStamp (u64, microseconds)
        Long precisionTimeStamp,
        // Tag 3 — vmtiSystemName (UTF-8)
        String vmtiSystemName,
        // Tag 4 — versionNumber (u16)
        Integer versionNumber,
        // Tag 5 — totalTargetsInFrame (u32)
        Long totalTargetsInFrame,
        // Tag 6 — numTargetsReported (u32)
        Long numTargetsReported,
        // Tag 8 — frameWidth (u32)
        Long frameWidth,
        // Tag 9 — frameHeight (u32)
        Long frameHeight,
        // Tag 10 — sourceSensor (UTF-8)
        String sourceSensor,
        // Tag 11 — horizontalFov (IMAPB f64)
        Double horizontalFov,
        // Tag 12 — verticalFov (IMAPB f64)
        Double verticalFov,
        // Tag 13 — miisId (pass-through bytes, MISB ST 1204)
        ByteBuffer miisId,
        // Tag 101 — VTargetSeries decoded targets
        List<VTargetPack> targets,
        // Tag 102 — algorithmSeries (pass-through)
        ByteBuffer algorithmSeries,
        // Tag 103 — ontologySeries (pass-through)
        ByteBuffer ontologySeries,
        // Forward-compat unknown TLVs
        List<KlvUnknownField> unknown,
        // Non-fatal per-field decode errors
        List<KlvFieldError> fieldErrors
) implements KlvSet {

    /** Compact constructor: make list fields truly immutable. */
    public VmtiLs {
        targets = targets != null ? Collections.unmodifiableList(targets) : Collections.emptyList();
        unknown = unknown != null ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Fluent mutable builder for {@link VmtiLs}.
     *
     * <p>All fields are optional. List fields default to empty immutable lists.
     */
    public static final class Builder {
        private Integer checksum;
        private Long precisionTimeStamp;
        private String vmtiSystemName;
        private Integer versionNumber;
        private Long totalTargetsInFrame;
        private Long numTargetsReported;
        private Long frameWidth;
        private Long frameHeight;
        private String sourceSensor;
        private Double horizontalFov;
        private Double verticalFov;
        private ByteBuffer miisId;
        private List<VTargetPack> targets = Collections.emptyList();
        private ByteBuffer algorithmSeries;
        private ByteBuffer ontologySeries;
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        /** Create an empty Builder. */
        public Builder() {}

        public Builder checksum(int v) { this.checksum = v; return this; }
        public Builder precisionTimeStamp(long v) { this.precisionTimeStamp = v; return this; }
        public Builder vmtiSystemName(String v) { this.vmtiSystemName = v; return this; }
        public Builder versionNumber(int v) { this.versionNumber = v; return this; }
        public Builder totalTargetsInFrame(long v) { this.totalTargetsInFrame = v; return this; }
        public Builder numTargetsReported(long v) { this.numTargetsReported = v; return this; }
        public Builder frameWidth(long v) { this.frameWidth = v; return this; }
        public Builder frameHeight(long v) { this.frameHeight = v; return this; }
        public Builder sourceSensor(String v) { this.sourceSensor = v; return this; }
        public Builder horizontalFov(double v) { this.horizontalFov = v; return this; }
        public Builder verticalFov(double v) { this.verticalFov = v; return this; }
        public Builder miisId(ByteBuffer v) { this.miisId = v; return this; }
        public Builder targets(List<VTargetPack> v) { this.targets = v; return this; }
        public Builder algorithmSeries(ByteBuffer v) { this.algorithmSeries = v; return this; }
        public Builder ontologySeries(ByteBuffer v) { this.ontologySeries = v; return this; }
        public Builder unknown(List<KlvUnknownField> v) { this.unknown = v; return this; }
        public Builder fieldErrors(List<KlvFieldError> v) { this.fieldErrors = v; return this; }

        /** Build an immutable {@link VmtiLs}. */
        public VmtiLs build() {
            return new VmtiLs(
                    checksum, precisionTimeStamp, vmtiSystemName, versionNumber,
                    totalTargetsInFrame, numTargetsReported,
                    frameWidth, frameHeight,
                    sourceSensor, horizontalFov, verticalFov,
                    miisId, targets, algorithmSeries, ontologySeries,
                    unknown, fieldErrors
            );
        }
    }
}
