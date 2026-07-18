package org.tstrans.klv;

/**
 * MISB ST 0601.19 §8.81 Item 81 — Image Horizon Pixels: screen-space horizon
 * line endpoints as percentages of image width/height, plus an optional
 * geodetic pair for each endpoint.
 *
 * <p>The four {@code *Deg} fields are a truncatable trailing group (ST 0107
 * DLP convention) — any of them may be absent independently of the others,
 * per the Rust {@code ImageHorizonPixels} rustdoc's sentinel-fill contract.
 *
 * <p>Instances are immutable; use {@link Builder} to construct. The
 * canonical constructor is two runs of four same-typed positional
 * arguments (four {@code int} percentages, then four boxed {@code Double}
 * degrees) — easy to silently transpose (e.g. {@code x0Pct}/{@code y0Pct},
 * or a start/end lat/lon swap) in a bare positional call. The Builder's
 * named setters remove that risk; prefer it over the canonical constructor.
 *
 * @param x0Pct       start-point x, percent of image width (0-255 wire byte)
 * @param y0Pct       start-point y, percent of image height
 * @param x1Pct       end-point x, percent of image width
 * @param y1Pct       end-point y, percent of image height
 * @param startLatDeg start-point latitude, or {@code null} if absent
 * @param startLonDeg start-point longitude, or {@code null} if absent
 * @param endLatDeg   end-point latitude, or {@code null} if absent
 * @param endLonDeg   end-point longitude, or {@code null} if absent
 */
public record ImageHorizonPixels(
        int x0Pct,
        int y0Pct,
        int x1Pct,
        int y1Pct,
        Double startLatDeg,
        Double startLonDeg,
        Double endLatDeg,
        Double endLonDeg
) {

    /**
     * Fluent mutable builder for {@link ImageHorizonPixels}. No field is
     * mandatory at construction time (unset percentages default to 0,
     * matching the Rust side's own default); the point is naming every
     * field explicitly rather than relying on positional order.
     */
    public static final class Builder {
        private int x0Pct;
        private int y0Pct;
        private int x1Pct;
        private int y1Pct;
        private Double startLatDeg;
        private Double startLonDeg;
        private Double endLatDeg;
        private Double endLonDeg;

        public Builder() {}

        public Builder x0Pct(int v) { this.x0Pct = v; return this; }
        public Builder y0Pct(int v) { this.y0Pct = v; return this; }
        public Builder x1Pct(int v) { this.x1Pct = v; return this; }
        public Builder y1Pct(int v) { this.y1Pct = v; return this; }
        public Builder startLatDeg(double v) { this.startLatDeg = v; return this; }
        public Builder startLonDeg(double v) { this.startLonDeg = v; return this; }
        public Builder endLatDeg(double v) { this.endLatDeg = v; return this; }
        public Builder endLonDeg(double v) { this.endLonDeg = v; return this; }

        /** Build an immutable {@link ImageHorizonPixels}. */
        public ImageHorizonPixels build() {
            return new ImageHorizonPixels(
                    x0Pct, y0Pct, x1Pct, y1Pct,
                    startLatDeg, startLonDeg, endLatDeg, endLonDeg);
        }
    }
}
