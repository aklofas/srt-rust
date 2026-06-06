package org.tstrans.rtp;

/**
 * Transport preference at SETUP. {@code AUTO} = UDP-first with TCP fallback on
 * 461; {@code UDP}/{@code TCP} force a single transport. Mirrors tst-py
 * {@code tstrans.rtp.TransportPref}.
 */
public enum TransportPref { AUTO, UDP, TCP }
