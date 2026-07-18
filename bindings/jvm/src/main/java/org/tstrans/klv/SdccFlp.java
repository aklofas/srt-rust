package org.tstrans.klv;

/**
 * One parsed MISB ST 1010.3 SDCC-FLP (Standard Deviation and Correlation
 * Coefficient pack, Floating-Point variant, §6-§7): an N&times;N symmetric
 * matrix — N standard deviations on the diagonal, N(N-1)/2 correlation
 * coefficients in the upper triangle (row-major, i&lt;j).
 *
 * <p>{@code correlations} is always the full {@code matrixSize *
 * (matrixSize - 1) / 2}-length triangle regardless of how many the wire
 * actually carried — absent (sparse-mode-omitted) slots are reconstituted
 * as {@code 0.0}; {@code correlationPresent} marks which slots the wire
 * actually carried (all {@code true} in full/non-sparse mode).
 * {@code stdDevs} is empty when the wire carried no standard-deviation data
 * at all ({@code Slen==0} — spec-legal).
 *
 * <p>General-purpose — not ST 0601-specific; usable by any MISB Parent
 * Document. Carried inside ST 0601 Item 102 (see {@link SdccFlpField}), but
 * this type has no ST 0601 dependency. Decode via
 * {@link Klv#decodeSdccFlp(byte[])}, encode Mode 2 via
 * {@link Klv#encodeSdccFlpMode2(double[], double[], int)}.
 *
 * @param matrixSize          N, the matrix dimension
 * @param stdDevs             diagonal &sigma; values, length == matrixSize
 *                            (empty when the wire carried no standard-deviation data)
 * @param correlations        upper-triangle correlations, row-major (i&lt;j),
 *                            always length {@code matrixSize*(matrixSize-1)/2}
 * @param correlationPresent  true where the wire actually carried the slot
 *                            (all true in full mode), same length as {@code correlations}
 */
public record SdccFlp(long matrixSize, double[] stdDevs, double[] correlations, boolean[] correlationPresent) {

    /**
     * &rho;(i,j) with symmetry; &sigma; via {@code stdDevs[i]} on the
     * diagonal. Pure-Java port of the Rust {@code SdccFlp::correlation}
     * (row-major upper-triangle index) — no JNI crossing needed.
     *
     * @param i row index, {@code 0 <= i < matrixSize}
     * @param j column index, {@code 0 <= j < matrixSize}
     * @return &rho;(i,j) off-diagonal, or &sigma;(i) on the diagonal
     * @throws IndexOutOfBoundsException if {@code i} or {@code j} is
     *         {@code >= matrixSize}, or on a diagonal query ({@code i == j})
     *         if this pack has no standard-deviation data at all
     *         ({@code Slen == 0} — spec-legal, see the {@link #stdDevs}
     *         field doc). Off-diagonal queries never hit that second case —
     *         {@link #correlations} is always sized to the full triangle
     *         regardless of {@code Clen}.
     */
    public double correlation(int i, int j) {
        long n = matrixSize;
        if (i < 0 || j < 0 || i >= n || j >= n) {
            throw new IndexOutOfBoundsException(
                    "SdccFlp.correlation index out of bounds: (" + i + ", " + j
                            + ") for matrixSize " + n);
        }
        if (i == j) {
            if (i >= stdDevs.length) {
                throw new IndexOutOfBoundsException(
                        "SdccFlp.correlation(" + i + ", " + i + "): no standard-deviation value "
                                + "available (Slen==0 / an empty stdDevs for this pack)");
            }
            return stdDevs[i];
        }
        int lo = Math.min(i, j);
        int hi = Math.max(i, j);
        int offset = lo * (2 * (int) n - lo - 1) / 2;
        return correlations[offset + (hi - lo - 1)];
    }
}
