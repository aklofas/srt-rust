package org.tstrans.klv;

/**
 * One record of MISB ST 0601.19 §8.128 Item 128, Wavelengths List — a sensor
 * wavelength band definition. {@code minNm}/{@code maxNm} are ST 1201.5
 * IMAPB(0, 1e9, 4)-decoded, giving ~&frac12; nm precision across the full
 * X-ray-to-VHF spectrum span the spec cites.
 *
 * @param id    BER-OID wavelength band id
 * @param minNm band minimum wavelength, nanometres
 * @param maxNm band maximum wavelength, nanometres
 * @param name  band name
 */
public record WavelengthRecord(long id, double minNm, double maxNm, String name) {}
