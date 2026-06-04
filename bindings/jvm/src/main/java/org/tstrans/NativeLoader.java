package org.tstrans;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;

/**
 * Loads {@code libtstjni} by extracting the platform-matched native library
 * from the JAR's {@code native/<triple>/} resources to a temp file and
 * {@code System.load}-ing it. Same pattern as JNA / sqlite-jdbc / LWJGL.
 *
 * <p>Bootstrap: only {@code linux-x86_64} is shipped; the triple/ext logic is
 * written generally so the multi-platform wave only adds resources, not code.
 */
public final class NativeLoader {
    private static volatile boolean loaded = false;

    private NativeLoader() {}

    public static synchronized void load() {
        if (loaded) {
            return;
        }
        String ext = libExtension();
        String resource = "/native/" + triple() + "/libtstjni." + ext;
        try (InputStream in = NativeLoader.class.getResourceAsStream(resource)) {
            if (in == null) {
                throw new UnsatisfiedLinkError("native library not found on classpath: " + resource);
            }
            // suffix arg includes the leading dot; "libtstjni" + random + ".so"
            Path tmp = Files.createTempFile("libtstjni", "." + ext);
            tmp.toFile().deleteOnExit();
            Files.copy(in, tmp, StandardCopyOption.REPLACE_EXISTING);
            System.load(tmp.toAbsolutePath().toString());
            loaded = true;
        } catch (IOException e) {
            throw new UnsatisfiedLinkError("failed to extract native library " + resource + ": " + e.getMessage());
        }
    }

    private static String osName() {
        return System.getProperty("os.name", "").toLowerCase();
    }

    /** Normalize {@code os.name}/{@code os.arch} to the JAR triple (e.g. {@code linux-x86_64}). */
    private static String triple() {
        String os = osName();
        String arch = System.getProperty("os.arch", "").toLowerCase();
        String o;
        if (os.contains("win")) {
            o = "windows";
        } else if (os.contains("mac") || os.contains("darwin")) {
            o = "macos";
        } else {
            o = "linux";
        }
        String a = (arch.equals("amd64") || arch.equals("x86_64")) ? "x86_64"
                : (arch.equals("aarch64") || arch.equals("arm64")) ? "aarch64"
                : arch;
        return o + "-" + a;
    }

    private static String libExtension() {
        String os = osName();
        if (os.contains("win")) {
            return "dll";
        }
        if (os.contains("mac") || os.contains("darwin")) {
            return "dylib";
        }
        return "so";
    }
}
