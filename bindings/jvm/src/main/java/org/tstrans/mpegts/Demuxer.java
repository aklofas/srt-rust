package org.tstrans.mpegts;

import java.util.Iterator;
import java.util.NoSuchElementException;
import org.tstrans.DemuxException;

/**
 * Streaming MPEG-TS demuxer. Feed TS bytes, pull {@link DemuxEvent}s.
 * Mirrors {@code tstrans.mpegts.Demuxer}. One demuxer is single-threaded;
 * the consumer owns concurrency (spec §5.5).
 *
 * <pre>{@code
 * try (Demuxer d = new Demuxer()) {
 *     d.feed(tsBytes);
 *     for (DemuxEvent e : d) { ... }
 * }
 * }</pre>
 *
 * <p><strong>Keystone event subset.</strong> This wave surfaces only
 * {@link DemuxEvent.ProgramMap}, the sample records ({@link DemuxEvent.Video},
 * {@link DemuxEvent.Audio}, {@link DemuxEvent.Subtitle},
 * {@link DemuxEvent.UnknownSample}), and
 * {@link DemuxEvent.Discontinuity}. Other demuxer events — KLV/{@code Metadata},
 * {@code NonConformant} (recoverable stream-quality issues), and
 * {@code ReconnectDiscontinuity} — are <em>silently skipped</em> for now; they
 * are added in the mpegts-completion wave. Do not rely on this demuxer to
 * surface KLV metadata or non-conformance until then.
 */
public final class Demuxer implements AutoCloseable, Iterable<DemuxEvent> {
    static { org.tstrans.NativeLoader.load(); }

    private long handle; // Box<tst_core::...::Demuxer> pointer; 0 = closed

    public Demuxer() {
        this.handle = nOpen();
    }

    /** Feed TS bytes. @throws DemuxException on non-conformant input. */
    public void feed(byte[] bytes) throws DemuxException {
        ensureOpen();
        nFeed(handle, bytes);
    }

    /** Flush buffered partial units (call at end of stream). */
    public void flush() {
        ensureOpen();
        nFlush(handle);
    }

    /**
     * Pull the next ready event, or null if none queued. Skips event types not
     * yet mapped in this keystone wave (see the class doc) — so a returned
     * {@code null} means "no <em>keystone</em> event is currently queued", and
     * KLV/{@code Metadata} / {@code NonConformant} / {@code ReconnectDiscontinuity}
     * events are dropped rather than returned.
     */
    public DemuxEvent nextEvent() throws DemuxException {
        ensureOpen();
        return nNextEvent(handle);
    }

    /** Iterate already-queued events (drains the current queue; does not feed). */
    @Override
    public Iterator<DemuxEvent> iterator() {
        return new Iterator<>() {
            private DemuxEvent peeked;
            @Override public boolean hasNext() {
                if (peeked != null) return true;
                try { peeked = nextEvent(); } catch (DemuxException e) { throw new RuntimeException(e); }
                return peeked != null;
            }
            @Override public DemuxEvent next() {
                if (!hasNext()) throw new NoSuchElementException();
                DemuxEvent e = peeked; peeked = null; return e;
            }
        };
    }

    @Override
    public void close() {
        if (handle != 0) { nClose(handle); handle = 0; }
    }

    private void ensureOpen() {
        if (handle == 0) throw new IllegalStateException("Demuxer is closed");
    }

    private static native long nOpen();
    private static native void nFeed(long handle, byte[] bytes) throws DemuxException;
    private static native void nFlush(long handle);
    private static native DemuxEvent nNextEvent(long handle) throws DemuxException;
    private static native void nClose(long handle);
}
