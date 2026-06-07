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
 * <p>The fat JAR ships {@code native/<triple>/libtstjni.<ext>} for every
 * Tier 1 platform; {@link #resourcePath} resolves the running host to that
 * path. The library filename is always {@code libtstjni.<ext>} on every
 * platform (Windows's cargo {@code tstjni.dll} is renamed to
 * {@code libtstjni.dll} at packaging time); {@code System.load} takes an
 * absolute path so the on-disk name need not match the module's internal name.
 */
public final class NativeLoader {
    private static volatile boolean loaded = false;

    private NativeLoader() {}

    public static synchronized void load() {
        if (loaded) {
            return;
        }
        String osName = System.getProperty("os.name", "");
        String resource = resourcePath(osName, System.getProperty("os.arch", ""));
        String ext = libExtension(osName);
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

    /**
     * JAR-internal resource path for the given host. Pure function of
     * {@code os.name}/{@code os.arch} so it is testable on any host.
     */
    static String resourcePath(String osName, String osArch) {
        return "/native/" + triple(osName, osArch) + "/libtstjni." + libExtension(osName);
    }

    /** Normalize {@code os.name}/{@code os.arch} to the JAR triple (e.g. {@code linux-x86_64}). */
    static String triple(String osName, String osArch) {
        String os = osName.toLowerCase();
        String arch = osArch.toLowerCase();
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

    static String libExtension(String osName) {
        String os = osName.toLowerCase();
        if (os.contains("win")) {
            return "dll";
        }
        if (os.contains("mac") || os.contains("darwin")) {
            return "dylib";
        }
        return "so";
    }
}
