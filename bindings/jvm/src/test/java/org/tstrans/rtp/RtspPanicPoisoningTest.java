package org.tstrans.rtp;

import static org.junit.jupiter.api.Assertions.assertNotEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.Timeout;
import org.junit.jupiter.api.condition.EnabledIfSystemProperty;
import org.tstrans.internal.PanicProbe;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.VideoCodec;

/**
 * Proves the RTSP <em>mutating</em> natives (the {@code pushX}/{@code flush}/
 * {@code stop}/{@code addX} family) actually route through
 * {@code with_mount_poisoning}/{@code with_server_poisoning}, so a panic torn
 * mid-mutation POISONS the leased handle and every later op throws
 * {@link IllegalStateException} instead of reusing torn native state (JNI-01).
 *
 * <p>The generic {@code PanicProbe} (see {@link org.tstrans.PanicIsolationTest})
 * proves {@code with_poisoning} in isolation; these tests force a panic THROUGH
 * the real {@code REGISTRY_MOUNT}/{@code REGISTRY_SERVER} on a REAL leased handle
 * — the same registry + code path a real {@code pushVideo}/{@code stop} uses —
 * so they pin the wiring of THIS surface, not just the primitive.
 *
 * <p>Runs only when the cdylib was built with the {@code jni-test-hooks} cargo
 * feature (Gradle {@code -PjniTestHooks=true}); the shipped JAR never carries the
 * probe symbols. The server binds {@code 127.0.0.1:0} (ephemeral loopback) and no
 * peer ever connects, so there is no network flakiness.
 */
@EnabledIfSystemProperty(named = "tst.jniTestHooks", matches = "true")
class RtspPanicPoisoningTest {

    private static MuxerConfig videoCfg() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264).build();
    }

    private static byte[] idr() {
        byte[] b = new byte[20];
        b[0] = 0; b[1] = 0; b[2] = 0; b[3] = 1; b[4] = 0x65;
        for (int i = 0; i < 15; i++) b[5 + i] = (byte) (0xA5 ^ i);
        return b;
    }

    /**
     * A panic torn inside a leased {@code MountHandle} mutation poisons the mount:
     * the panic surfaces as a {@link RuntimeException}, and a subsequent REAL mount
     * push then throws {@link IllegalStateException} (handle gone), proving the
     * mount push family routes through {@code with_mount_poisoning}.
     */
    @Test @Timeout(15)
    void mountMutatorPanicPoisonsMountHandle() throws Exception {
        try (RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"));
             MountHandle m = s.addUnicastMount("/poison-mount", videoCfg())) {

            // Sanity: a real push works BEFORE the torn mutation.
            m.pushVideo(idr(), 0L, true);

            long raw = m.nativeHandleForTest();
            assertNotEquals(0L, raw, "leased mount handle must be non-zero");

            // (1) Force a panic through the real REGISTRY_MOUNT.with_poisoning on
            //     this exact handle — surfaces as a RuntimeException (jni_catch).
            assertThrows(RuntimeException.class,
                () -> PanicProbe.nForcePanicThroughMount(raw));

            // (2) The mount entry is now poisoned: a REAL mount op leases None and
            //     throws IllegalStateException, NOT a reuse of torn state.
            assertThrows(IllegalStateException.class,
                () -> m.pushVideo(idr(), 90_000L, true));
            // A second mutating native (flush) is equally dead.
            assertThrows(IllegalStateException.class, m::flush);
        }
        // try-with-resources close() on the poisoned MountHandle is a safe no-op
        // (the native entry is already gone) — reaching here without throwing
        // confirms that.
    }

    /**
     * A panic torn inside a leased {@code RtspServer} mutation poisons the server:
     * the panic surfaces as a {@link RuntimeException}, and a subsequent REAL
     * server op then throws {@link IllegalStateException}, proving the server
     * mutators ({@code stop}/{@code addX}) route through
     * {@code with_server_poisoning}.
     */
    @Test @Timeout(15)
    void serverMutatorPanicPoisonsRtspServer() throws Exception {
        RtspServer s = RtspServer.start(RtspServerConfig.of("127.0.0.1:0"));
        try {
            long raw = s.nativeHandleForTest();
            assertNotEquals(0L, raw, "leased server handle must be non-zero");

            // (1) Force a panic through the real REGISTRY_SERVER.with_poisoning.
            assertThrows(RuntimeException.class,
                () -> PanicProbe.nForcePanicThroughServer(raw));

            // (2) The server entry is poisoned: a REAL server op (a mutator —
            //     addUnicastMount) now leases None → IllegalStateException.
            assertThrows(IllegalStateException.class,
                () -> s.addUnicastMount("/after-poison", videoCfg()));
            // A getter is equally dead (the entry is gone, not just the resource).
            assertThrows(IllegalStateException.class, s::stats);
        } finally {
            // close() on a poisoned server is a safe idempotent no-op.
            s.close();
        }
    }
}
