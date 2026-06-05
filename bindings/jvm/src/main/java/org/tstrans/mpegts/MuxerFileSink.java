package org.tstrans.mpegts;

import java.io.BufferedOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import org.tstrans.MuxException;

/**
 * A context-managed sink that drains a {@link Muxer}'s output to a file. Mirrors
 * tst-py's {@code MuxerFileSink}: after each {@code push*} call (and on
 * {@link #close()}) it pulls pending TS packets and writes them to disk. Construct
 * via {@link Muxer#writeFile}.
 *
 * <p>The {@link Muxer} is <b>borrowed, not owned</b> — it remains usable after the
 * try-with-resources block (including for further {@code writeFile} calls).
 *
 * <p>Unlike the bare {@link Muxer} push methods, the sink's {@code push*} methods
 * also declare {@link java.io.IOException} because each drains pending packets to
 * disk.
 *
 * <p><b>Atomic mode.</b> tst-py infers success from Python's
 * {@code __exit__(exc_type)}; Java's {@link AutoCloseable#close()} has no exception
 * hook, so atomic mode uses an explicit {@link #commit()} marker: only a committed
 * sink promotes the {@code *.partial} temp to the destination on close; otherwise
 * the temp is discarded (nothing appears at the destination). Non-atomic mode
 * always preserves whatever was written (partial output on a body exception),
 * matching tst-py's non-atomic contract.
 */
public final class MuxerFileSink implements AutoCloseable {
    private static final int DRAIN_CHUNK_PACKETS = 7;
    private static final int DRAIN_CHUNK_BYTES = DRAIN_CHUNK_PACKETS * 188; // 1316

    private final Muxer muxer; // borrowed
    private final Path dest;
    private final boolean atomic;
    private final Path tmpPath; // non-null iff atomic
    private final OutputStream out;
    private final byte[] buf = new byte[DRAIN_CHUNK_BYTES];
    private boolean committed = false;
    private boolean closed = false;

    MuxerFileSink(Muxer muxer, Path dest, boolean atomic) throws IOException {
        this.muxer = muxer;
        this.dest = dest;
        this.atomic = atomic;
        if (atomic) {
            Path dir = dest.toAbsolutePath().getParent();
            this.tmpPath = Files.createTempFile(dir, dest.getFileName().toString(), ".partial");
            OutputStream raw;
            try {
                raw = Files.newOutputStream(tmpPath);
            } catch (IOException e) {
                Files.deleteIfExists(tmpPath);
                throw e;
            }
            this.out = new BufferedOutputStream(raw);
        } else {
            this.tmpPath = null;
            this.out = new BufferedOutputStream(Files.newOutputStream(dest));
        }
    }

    /**
     * Push one H.264/H.265/H.266 access unit or AV1 OBU stream. Drains pending
     * TS packets to the file after pushing.
     */
    public void pushVideo(byte[] nal, long pts, boolean keyFrame) throws MuxException, IOException {
        muxer.pushVideo(nal, pts, keyFrame);
        drain();
    }

    /**
     * Push one KLV local-set payload. Drains pending TS packets to the file after
     * pushing.
     */
    public void pushKlv(byte[] klv, long pts, int metadataServiceId) throws MuxException, IOException {
        muxer.pushKlv(klv, pts, metadataServiceId);
        drain();
    }

    /**
     * Push one audio frame. Drains pending TS packets to the file after pushing.
     */
    public void pushAudio(byte[] frames, long pts) throws MuxException, IOException {
        muxer.pushAudio(frames, pts);
        drain();
    }

    /**
     * Push one subtitle PES payload. Note the {@code (pts, payload)} argument order
     * (matches {@code Muxer#pushSubtitle}). Drains pending TS packets to the file
     * after pushing.
     */
    public void pushSubtitle(long pts, byte[] payload) throws MuxException, IOException {
        muxer.pushSubtitle(pts, payload);
        drain();
    }

    /**
     * Mark the write successful. In atomic mode, only a committed sink promotes the
     * {@code *.partial} temp to the destination on {@link #close()}; without it,
     * close discards the temp. A no-op in non-atomic mode. Calling this after
     * {@link #close()} has no effect.
     */
    public void commit() {
        this.committed = true;
    }

    private void drain() throws IOException {
        while (muxer.pendingPackets() > 0) {
            int n = muxer.pull(buf);
            if (n == 0) {
                break;
            }
            out.write(buf, 0, n);
        }
    }

    @Override
    public void close() throws IOException {
        if (closed) {
            return;
        }
        closed = true;
        boolean drainAndCloseOk = false;
        try {
            try {
                drain();
            } finally {
                out.close();
            }
            drainAndCloseOk = true;
        } finally {
            if (atomic && (!drainAndCloseOk || !committed)) {
                Files.deleteIfExists(tmpPath);
            }
        }
        if (atomic && committed) {
            // ATOMIC_MOVE + REPLACE_EXISTING: intra-directory rename on the same
            // filesystem. Some Windows filesystems reject ATOMIC_MOVE
            // (AtomicMoveNotSupportedException); if the windows matrix is ever
            // promoted to gating, fall back to a two-step move or accept
            // non-atomic semantics there.
            Files.move(tmpPath, dest,
                StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        }
    }
}
