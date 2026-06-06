package org.tstrans.rtp;

/**
 * Digest auth algorithm selector. {@code MD5} (RFC 7616 §3.4 / RFC 2617 default)
 * or {@code SHA256} (RFC 7616 §3.4 with algorithm=SHA-256). Mirrors tst-py
 * {@code tstrans.rtp.DigestAlgorithm}.
 */
public enum DigestAlgorithm { MD5, SHA256 }
