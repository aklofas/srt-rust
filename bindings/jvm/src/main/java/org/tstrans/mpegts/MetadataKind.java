package org.tstrans.mpegts;

/** Metadata-stream classification. Mirrors {@code tst_core::mpegts::demux::event::MetadataKind} (collapsed to its variant tag, as tst-py's MetadataKindTag does). */
public enum MetadataKind { KLV_SYNC_AU_CELL, KLV_ASYNC, UNKNOWN }
