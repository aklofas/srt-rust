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
) {}
