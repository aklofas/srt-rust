package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.126 Tag 126 Sensor Control Mode — indicates what is
 * currently controlling the sensor.
 *
 * <p>Codepoints are 0-6. A wire-unknown codepoint surfaces as a
 * {@code null} typed accessor ({@link UasDatalinkLs#sensorControlMode()}
 * returns {@code null}) while the raw code is preserved in
 * {@link UasDatalinkLs#sensorControlModeCode()} — mirrors the
 * {@link IcingDetected} precedent. Java enum ordinals are NOT the wire
 * codepoints — use {@link #code()} for serialisation.
 */
public enum SensorControlMode {
    OFF(0),
    HOME_POSITION(1),
    UNCONTROLLED(2),
    MANUAL_CONTROL(3),
    CALIBRATING(4),
    AUTO_HOLDING_POSITION(5),
    AUTO_TRACKING(6);

    private final int code;

    SensorControlMode(int code) {
        this.code = code;
    }

    /** @return the ST 0601.19 §8.126 wire codepoint for this state. */
    public int code() {
        return code;
    }

    /**
     * Look up a {@code SensorControlMode} by ST 0601.19 wire codepoint.
     *
     * @param c the wire codepoint (0-6 for known values)
     * @return the matching constant, or {@code null} for an unknown codepoint
     */
    public static SensorControlMode fromCode(int c) {
        for (SensorControlMode v : values()) {
            if (v.code == c) return v;
        }
        return null;
    }
}
