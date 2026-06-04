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
val triple = "linux-x86_64"
val nativeOut = layout.projectDirectory.dir("src/main/resources/native/$triple")

// 1. Build the Rust cdylib (release). Bootstrap deps are jni-only, so this is
//    a fast pure-Rust build — no libsrt/librist/meson.
val cargoBuild = tasks.register<Exec>("cargoBuild") {
    workingDir = workspaceRoot
    commandLine("cargo", "build", "--release", "-p", "tst-jni")
}

// 2. Copy the cdylib into JAR resources at native/<triple>/libtstjni.so.
val copyNative = tasks.register<Copy>("copyNative") {
    dependsOn(cargoBuild)
    from(File(workspaceRoot, "target/release/libtstjni.so"))
    into(nativeOut)
}

tasks.named("processResources") { dependsOn(copyNative) }

tasks.test {
    dependsOn(copyNative)
    useJUnitPlatform()
    testLogging { showStandardStreams = true }
}
