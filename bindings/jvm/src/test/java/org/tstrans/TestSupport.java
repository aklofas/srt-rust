package org.tstrans;

import java.io.IOException;
import java.net.DatagramSocket;
import java.net.InetSocketAddress;
import java.nio.ByteBuffer;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.List;
import org.tstrans.codec.NalUnit;
import org.tstrans.codec.VideoUnit;
import org.tstrans.mpegts.MuxerConfig;
import org.tstrans.mpegts.VideoCodec;

/**
 * Test-only fixture builders shared across the JUnit suite (never shipped —
 * lives under {@code src/test/java}). Each method here previously existed as
 * an identical (or, for {@link #roundtripConfig()} /
 * {@link #roundtripConfigWithData()}, a two-shape) private copy in every test
 * class that needed it; import statically with the same unqualified name so
 * existing call sites are unchanged.
 */
public final class TestSupport {
    private TestSupport() {}

    /** Synthetic Annex-B IDR NAL: start code + IDR header + 15 filler bytes. */
    public static byte[] syntheticH264Idr() {
        byte[] buf = new byte[20];
        buf[0] = 0x00;
        buf[1] = 0x00;
        buf[2] = 0x00;
        buf[3] = 0x01;
        buf[4] = 0x65;
        for (int i = 0; i < 15; i++) {
            buf[5 + i] = (byte) (0xA5 ^ i);
        }
        return buf;
    }

    /** Whether the current OS is Linux, for gating live-socket fixtures. */
    public static boolean isLinux() {
        return System.getProperty("os.name", "").toLowerCase().contains("linux");
    }

    /** Pack `int` literals (0-255) into a `byte[]`, one cast per element. */
    public static byte[] unsigned(int... vals) {
        byte[] out = new byte[vals.length];
        for (int i = 0; i < vals.length; i++) {
            out[i] = (byte) vals[i];
        }
        return out;
    }

    /**
     * Lowercase hex SHA-256 over the concatenated payload bytes of every
     * {@link NalUnit} in {@code units} (Annex-B start codes already stripped
     * by the demuxer).
     */
    public static String sha256Units(List<VideoUnit> units) throws NoSuchAlgorithmException {
        MessageDigest md = MessageDigest.getInstance("SHA-256");
        for (VideoUnit u : units) {
            NalUnit n = (NalUnit) u;
            ByteBuffer view = n.payload().duplicate();
            byte[] bytes = new byte[view.remaining()];
            view.get(bytes);
            md.update(bytes);
        }
        byte[] digest = md.digest();
        StringBuilder sb = new StringBuilder(digest.length * 2);
        for (byte b : digest) {
            sb.append(Character.forDigit((b >> 4) & 0xF, 16));
            sb.append(Character.forDigit(b & 0xF, 16));
        }
        return sb.toString();
    }

    /** Single-program H.264-only {@link MuxerConfig} shared by roundtrip fixtures. */
    public static MuxerConfig roundtripConfig() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .build();
    }

    /**
     * {@link #roundtripConfig()} plus one private-data stream (user-private
     * {@code stream_type} 0xF0, PID 0x0100) — the shape the SRT/RTP managed
     * and mux/demux loopback fixtures need.
     */
    public static MuxerConfig roundtripConfigWithData() {
        return MuxerConfig.builder()
            .programNumber(1).pmtPid(0x1000)
            .addVideo(0x1011, VideoCodec.H264)
            .addData(0x0100, 0xF0, true)
            .build();
    }

    /** Bind a throwaway UDP socket to :0, read the kernel-picked port, release it. */
    public static int freeUdpPort() throws IOException {
        try (DatagramSocket s = new DatagramSocket(new InetSocketAddress("127.0.0.1", 0))) {
            return s.getLocalPort();
        }
    }
}
