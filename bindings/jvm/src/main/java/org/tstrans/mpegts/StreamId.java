package org.tstrans.mpegts;

/** Identity of an elementary stream. Mirrors {@code tst_core::...::StreamId}. */
public record StreamId(int pid, StreamKind kind, int programNumber) {}
