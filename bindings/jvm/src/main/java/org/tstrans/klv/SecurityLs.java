package org.tstrans.klv;

import java.util.Collections;
import java.util.List;
import java.util.Optional;

/**
 * MISB ST 0102.12 Security Metadata Local Set typed view.
 *
 * <p>Required tags per ST 0102.12 §6.7 Table 1: Tags 1, 2, 3, 12, 13, 22.
 * Lenient decode ({@link Klv#decodeSecurity(byte[])}) tolerates missing
 * required tags, unknown enum codepoints, and malformed Tag 13 UTF-16
 * (surfaced in {@link #fieldErrors()}). Strict decode
 * ({@link Klv#decodeSecurity(byte[], boolean) decodeSecurity(buf, true)})
 * rejects all of the above.
 *
 * <p>The three enum-typed fields ({@link #securityClassification()},
 * {@link #classifyingCountryCodingMethod()}, {@link #objectCountryCodingMethod()})
 * are stored internally as nullable {@code Integer} raw codepoints so that
 * unknown/forward-compat codepoints round-trip faithfully. Use the typed
 * {@code Optional} accessors for normal dispatch; use the raw-code accessors
 * (e.g. {@link #securityClassificationCode()}) to inspect an unknown codepoint.
 *
 * <p>Instances are immutable; use {@link Builder} to construct.
 */
public record SecurityLs(
        // Tag 1 — stored as raw code (null = absent); typed via securityClassification()
        Integer securityClassificationCode,
        // Tag 2 — stored as raw code (null = absent)
        Integer classifyingCountryCodingMethodCode,
        // Tag 3
        String classifyingCountry,
        // Tag 12 — stored as raw code (null = absent)
        Integer objectCountryCodingMethodCode,
        // Tag 13 (UTF-16)
        String objectCountryCodes,
        // Tag 22
        Integer version,
        // Context fields (Tags 4–11, 14, 23, 24)
        String sciShiInfo,
        String caveats,
        String releasingInstructions,
        String classifiedBy,
        String derivedFrom,
        String classificationReason,
        String declassificationDate,
        String classificationMarkingSystem,
        String classificationComments,
        String classifyingCountryCodingMethodVersionDate,
        String objectCountryCodingMethodVersionDate,
        // Forward-compat unknown TLVs and non-fatal per-field errors
        List<KlvUnknownField> unknown,
        List<KlvFieldError> fieldErrors
) implements KlvSet {

    // Compact constructor: make the list fields truly immutable.
    public SecurityLs {
        unknown = unknown != null ? Collections.unmodifiableList(unknown) : Collections.emptyList();
        fieldErrors = fieldErrors != null ? Collections.unmodifiableList(fieldErrors) : Collections.emptyList();
    }

    // -----------------------------------------------------------------------
    // Typed enum accessors
    // -----------------------------------------------------------------------

    /**
     * Tag 1 Security Classification as a typed enum.
     * Returns {@link Optional#empty()} when the tag was absent or carries an
     * unknown/forward-compat codepoint; use {@link #securityClassificationCode()}
     * to inspect the raw codepoint in the latter case.
     */
    public Optional<SecurityClassification> securityClassification() {
        if (securityClassificationCode == null) return Optional.empty();
        return SecurityClassification.fromCode(securityClassificationCode);
    }

    /**
     * Tag 2 Classifying Country Coding Method as a typed enum.
     * Returns {@link Optional#empty()} when absent or an unknown codepoint.
     */
    public Optional<ClassifyingCountryCodingMethod> classifyingCountryCodingMethod() {
        if (classifyingCountryCodingMethodCode == null) return Optional.empty();
        return ClassifyingCountryCodingMethod.fromCode(classifyingCountryCodingMethodCode);
    }

    /**
     * Tag 12 Object Country Coding Method as a typed enum.
     * Returns {@link Optional#empty()} when absent or an unknown codepoint.
     */
    public Optional<ObjectCountryCodingMethod> objectCountryCodingMethod() {
        if (objectCountryCodingMethodCode == null) return Optional.empty();
        return ObjectCountryCodingMethod.fromCode(objectCountryCodingMethodCode);
    }

    // -----------------------------------------------------------------------
    // Builder
    // -----------------------------------------------------------------------

    /**
     * Mutable builder for {@link SecurityLs}. All fields default to null /
     * empty list. Setters return {@code this} for fluent chaining; the three
     * enum-typed setters accept either an {@code int} raw codepoint or a typed
     * enum constant.
     */
    public static final class Builder {
        private Integer securityClassificationCode;
        private Integer classifyingCountryCodingMethodCode;
        private String classifyingCountry;
        private Integer objectCountryCodingMethodCode;
        private String objectCountryCodes;
        private Integer version;
        private String sciShiInfo;
        private String caveats;
        private String releasingInstructions;
        private String classifiedBy;
        private String derivedFrom;
        private String classificationReason;
        private String declassificationDate;
        private String classificationMarkingSystem;
        private String classificationComments;
        private String classifyingCountryCodingMethodVersionDate;
        private String objectCountryCodingMethodVersionDate;
        private List<KlvUnknownField> unknown = Collections.emptyList();
        private List<KlvFieldError> fieldErrors = Collections.emptyList();

        public Builder() {}

        // Tag 1 — raw int overload (stores the codepoint directly)
        public Builder securityClassification(int code) {
            this.securityClassificationCode = code;
            return this;
        }

        // Tag 1 — typed enum overload (stores .code())
        public Builder securityClassification(SecurityClassification v) {
            this.securityClassificationCode = v.code();
            return this;
        }

        // Tag 2 — raw int overload
        public Builder classifyingCountryCodingMethod(int code) {
            this.classifyingCountryCodingMethodCode = code;
            return this;
        }

        // Tag 2 — typed enum overload
        public Builder classifyingCountryCodingMethod(ClassifyingCountryCodingMethod v) {
            this.classifyingCountryCodingMethodCode = v.code();
            return this;
        }

        public Builder classifyingCountry(String v) {
            this.classifyingCountry = v;
            return this;
        }

        // Tag 12 — raw int overload
        public Builder objectCountryCodingMethod(int code) {
            this.objectCountryCodingMethodCode = code;
            return this;
        }

        // Tag 12 — typed enum overload
        public Builder objectCountryCodingMethod(ObjectCountryCodingMethod v) {
            this.objectCountryCodingMethodCode = v.code();
            return this;
        }

        public Builder objectCountryCodes(String v) {
            this.objectCountryCodes = v;
            return this;
        }

        public Builder version(int v) {
            this.version = v;
            return this;
        }

        public Builder sciShiInfo(String v) {
            this.sciShiInfo = v;
            return this;
        }

        public Builder caveats(String v) {
            this.caveats = v;
            return this;
        }

        public Builder releasingInstructions(String v) {
            this.releasingInstructions = v;
            return this;
        }

        public Builder classifiedBy(String v) {
            this.classifiedBy = v;
            return this;
        }

        public Builder derivedFrom(String v) {
            this.derivedFrom = v;
            return this;
        }

        public Builder classificationReason(String v) {
            this.classificationReason = v;
            return this;
        }

        public Builder declassificationDate(String v) {
            this.declassificationDate = v;
            return this;
        }

        public Builder classificationMarkingSystem(String v) {
            this.classificationMarkingSystem = v;
            return this;
        }

        public Builder classificationComments(String v) {
            this.classificationComments = v;
            return this;
        }

        public Builder classifyingCountryCodingMethodVersionDate(String v) {
            this.classifyingCountryCodingMethodVersionDate = v;
            return this;
        }

        public Builder objectCountryCodingMethodVersionDate(String v) {
            this.objectCountryCodingMethodVersionDate = v;
            return this;
        }

        public Builder unknown(List<KlvUnknownField> v) {
            this.unknown = v != null ? v : Collections.emptyList();
            return this;
        }

        public Builder fieldErrors(List<KlvFieldError> v) {
            this.fieldErrors = v != null ? v : Collections.emptyList();
            return this;
        }

        /** Build an immutable {@link SecurityLs} record. */
        public SecurityLs build() {
            return new SecurityLs(
                    securityClassificationCode,
                    classifyingCountryCodingMethodCode,
                    classifyingCountry,
                    objectCountryCodingMethodCode,
                    objectCountryCodes,
                    version,
                    sciShiInfo,
                    caveats,
                    releasingInstructions,
                    classifiedBy,
                    derivedFrom,
                    classificationReason,
                    declassificationDate,
                    classificationMarkingSystem,
                    classificationComments,
                    classifyingCountryCodingMethodVersionDate,
                    objectCountryCodingMethodVersionDate,
                    unknown,
                    fieldErrors
            );
        }
    }
}
