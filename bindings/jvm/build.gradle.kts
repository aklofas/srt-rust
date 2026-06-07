plugins {
    java
}

java {
    // §8.3 RESOLVED: JDK 17 MSRV.
    toolchain { languageVersion.set(JavaLanguageVersion.of(17)) }
}

repositories { mavenCentral() }

dependencies {
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.2")
    testRuntimeOnly("org.junit.platform:junit-platform-launcher")
}

// Workspace root is two levels up from bindings/jvm.
val workspaceRoot = layout.projectDirectory.dir("../..").asFile
val resourcesNativeRoot = layout.projectDirectory.dir("src/main/resources/native")

// Triples that MUST be present in the assembled fat JAR (match ci.yml's build
// matrix). macos-x86_64 (Intel) is deferred — see docs/deferred-features.md.
val gatingTriples = listOf("linux-x86_64", "linux-aarch64", "macos-aarch64", "windows-x86_64")

// When set (CI assemble job), skip cargo and stage pre-built libs from this dir
// (expected layout: <dir>/native/<triple>/libtstjni.<ext>). When null (dev),
// build the current platform's cdylib via cargo.
val nativeStagingDir = project.findProperty("nativeStagingDir") as String?

// Resolve the running host to the canonical <os>-<arch> triple that
// NativeLoader.triple() produces at runtime. MUST stay in sync with
// NativeLoader.triple() — any os/arch normalization change there must be
// mirrored here, or dev-mode staging puts the lib in the wrong dir and tests
// fail with UnsatisfiedLinkError.
fun currentTriple(): String {
    val os = System.getProperty("os.name").lowercase()
    val arch = System.getProperty("os.arch").lowercase()
    val o = when {
        os.contains("win") -> "windows"
        os.contains("mac") || os.contains("darwin") -> "macos"
        else -> "linux"
    }
    val a = when (arch) {
        "amd64", "x86_64" -> "x86_64"
        "aarch64", "arm64" -> "aarch64"
        else -> arch
    }
    return "$o-$a"
}

// cargo's cdylib basename differs by OS: tstjni.dll on Windows (no lib prefix),
// libtstjni.dylib on macOS, libtstjni.so on Linux.
fun cargoLibName(triple: String): String = when {
    triple.startsWith("windows") -> "tstjni.dll"
    triple.startsWith("macos") -> "libtstjni.dylib"
    else -> "libtstjni.so"
}

// JAR resource name: ALWAYS libtstjni.<ext> on every platform (NativeLoader
// expects the lib prefix everywhere; Windows tstjni.dll is renamed on copy).
fun resourceLibName(triple: String): String = when {
    triple.startsWith("windows") -> "libtstjni.dll"
    triple.startsWith("macos") -> "libtstjni.dylib"
    else -> "libtstjni.so"
}

// --- Dev mode: build current platform's cdylib (release) via cargo. As of the
//     srt wave this links vendored libsrt + mbedTLS, so SRT_FORCE_VENDORED=1 is
//     required; the first cold build compiles libsrt (~3-5 min), warm is seconds.
val cargoBuild = tasks.register<Exec>("cargoBuild") {
    onlyIf { nativeStagingDir == null }
    workingDir = workspaceRoot
    environment("SRT_FORCE_VENDORED", "1")
    commandLine("cargo", "build", "--release", "-p", "tst-jni")
}

// --- Dev mode: stage the just-built cdylib into native/<currentTriple>/,
//     renaming to the JAR resource name (matters on Windows).
val copyNative = tasks.register<Copy>("copyNative") {
    onlyIf { nativeStagingDir == null }
    dependsOn(cargoBuild)
    val t = currentTriple()
    from(File(workspaceRoot, "target/release/${cargoLibName(t)}"))
    into(resourcesNativeRoot.dir(t))
    rename { resourceLibName(t) }
}

// --- Staging mode (CI assemble): copy ALL pre-built native/<triple>/* libs
//     from the staging dir into resources. No cargo, no tests.
val stageNative = tasks.register<Copy>("stageNative") {
    onlyIf { nativeStagingDir != null }
    // config-time guard: prevents File(null, "native") NPE during configuration; onlyIf gates execution
    if (nativeStagingDir != null) {
        from(File(nativeStagingDir, "native"))
        into(resourcesNativeRoot)
    }
}

tasks.named("processResources") { dependsOn(copyNative, stageNative) }

tasks.test {
    dependsOn(copyNative) // dev-mode native; assemble job runs `jar`, not `test`.
    useJUnitPlatform()
    testLogging { showStandardStreams = true }
}

// --- Completeness guard: in staging mode, fail `jar` if any gating native lib
//     is missing (no silent truncation).
tasks.named<Jar>("jar") {
    doFirst {
        if (nativeStagingDir != null) {
            val missing = gatingTriples.filter {
                !resourcesNativeRoot.dir(it).file(resourceLibName(it)).asFile.exists()
            }
            if (missing.isNotEmpty()) {
                throw GradleException("fat JAR missing gating native libs for: $missing")
            }
            logger.lifecycle("fat JAR native libs: all gating triples present: $gatingTriples")
        }
    }
}
