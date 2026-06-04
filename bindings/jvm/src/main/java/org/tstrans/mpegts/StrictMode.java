package org.tstrans.mpegts;

/** Demuxer strictness ladder. Mirrors {@code tst_core::mpegts::demux::StrictMode} (PSI_ONLY ↔ Rust DescriptorsOnly). */
public enum StrictMode { OFF, TIMING_ONLY, PSI_ONLY, FULL }
