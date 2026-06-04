# JVM bindings (`org.tstrans`)

> **Who this is for:** You write Java (or any JVM language — Kotlin, Scala,
> Clojure) and want to demux MPEG-TS + KLV streams into typed events — or mux
> them back into a transport stream — on JDK 17+.

> **You will learn:**
> - How to build the JVM binding from source today (Maven Central is the planned distribution)
> - How to read a `.ts` file and dispatch typed `DemuxEvent` items
> - How to mux a single-program `.ts` offline with the `Muxer` + config builder
> - How to configure the demuxer with a fluent `DemuxerConfig` builder
> - How to decode / encode typed KLV sets (ST 0601 / 0102 / 0605 / 0903) under `org.tstrans.klv`
> - The JVM-specific gotchas: heap-copied `ByteBuffer` payloads, nullable `Long` DTS, codec on `StreamId`
> - How this binding differs from the Rust core

> **Status (mpegts demux + offline mux + typed KLV surfaces shipped):** the
> JVM binding ships the bootstrap `org.tstrans.Version` hello-world; the
> complete `org.tstrans.mpegts` **demux** surface (`Demuxer`,
> `DemuxerConfig`, the sealed `DemuxEvent` hierarchy, `StreamId`, codec /
> kind enums); the offline **mux** surface (`Muxer`, `MuxerConfig`, push
> family + `pull`); and the full **typed KLV** surface (`org.tstrans.klv`
> — decode/encode for ST 0601 / 0102 / 0605 / 0903, the `parseUniversal`
> dispatcher, and the field-error model). Codec parsers (`org.tstrans.codec`),
> typed NAL/OBU/ADTS payloads, and SRT / RTP transport are on the roadmap.
> This page documents only what exists today.

## Install

The JVM binding is **pre-release and not yet published to Maven Central.**
Build it from source via Gradle.

```bash
# From the workspace, build the binding and run its JUnit5 tests:
cd bindings/jvm
./gradlew test
```

The Gradle build (JDK 17 toolchain, wrapper 9.5.1) drives
`cargo build -p tst-jni` to produce the native library
(`libtstjni.so` / `.dylib` / `.dll`), copies it into JAR resources under
`native/<triple>/`, then compiles and tests the Java surface. A
`NativeLoader` extracts the right native library for the running platform
at runtime.

When it is published, the **planned** Maven coordinate is
`org.tstrans:tstrans-jvm` (JDK 17+):

```xml
<!-- PLANNED — not yet on Maven Central -->
<dependency>
  <groupId>org.tstrans</groupId>
  <artifactId>tstrans-jvm</artifactId>
  <version>0.1.0</version>
</dependency>
```

**Minimum JDK is 17.** The native code is delivered as a single fat JAR
(planned) bundling the per-platform native library; the consumer picks no
classifier.

## Hello world

The smallest thing that proves the native library loads and the JNI bridge
works — print the version string:

```java
import org.tstrans.Version;

System.out.println(Version.versionString());  // e.g. "0.1.0"
```

## First send

Build a single-program H.264 transport stream offline: configure the muxer,
push one access unit, then drain assembled TS packets with `pull`. The muxer
is deterministic — identical inputs produce byte-identical output across the
Rust, Python, and JVM bindings.

```java
import org.tstrans.mpegts.*;

MuxerConfig cfg = MuxerConfig.builder()
    .programNumber(1).pmtPid(0x1000)
    .addVideo(0x1011, VideoCodec.H264)
    .build();

byte[] out = new byte[8192];
try (Muxer m = new Muxer(cfg);
     var sink = java.nio.file.Files.newOutputStream(java.nio.file.Path.of("out.ts"))) {
    // pts is a 90 kHz tick count; keyFrame marks a random-access point.
    m.pushVideo(annexBNal, /*pts=*/ 0L, /*keyFrame=*/ true);
    int n;
    while ((n = m.pull(out)) > 0) {   // drain in a loop until pull returns 0
        sink.write(out, 0, n);        // n is always a multiple of 188
    }
}
```

`Muxer implements AutoCloseable` — the native allocation is reclaimed by
`close()`, so use try-with-resources. The push family mirrors the Rust core:

- `pushVideo(byte[] nal, long pts, boolean keyFrame)` — Annex-B H.264/H.265/H.266 (or AV1 OBU bitstream).
- `pushKlv(byte[] klv, long pts, int metadataServiceId)` — raw KLV LS bytes; for a `SYNCHRONOUS_METADATA` stream the muxer auto-prepends the 5-byte AU-cell header (do **not** pre-wrap).
- `pushAudio(byte[] frames, long pts)` — codec-native audio frames (ADTS for AAC, raw for MP2 / AC-3 / LATM).
- `pushSubtitle(long pts, byte[] payload)` — note the `(pts, payload)` argument order.

Each `push*` targets the lone stream of that kind; a muxer configured with
zero or more than one stream of the kind throws `MuxException(INVALID_USAGE)`.
Build the `MuxerConfig` with `addVideo` / `addKlv` / `addAudio` / `addSubtitle`
on `MuxerConfig.builder()`; the builder is single-program. Deep config
validation (PID collisions, PMT-size budget, sync-KLV-without-PTS, …) runs in
the native `Muxer` constructor and surfaces as `MuxException(CONFIG_INVALID)`.

> **Scope.** This binding's `MuxerConfig` is single-program; multi-program
> configs, per-stream/program descriptors, the `*_to(handle, …)` multi-stream
> variants, and DVB-subtitle codec configuration are deferred. `addSubtitle`
> accepts the no-config codecs (`CEA708_STANDALONE` / `WEBVTT_IN_TS`) today.

## First receive

Demux a `.ts` file and dispatch on typed events. The JVM binding's
baseline is **JDK 17**, where `instanceof` pattern matching is the portable
idiom for a sealed hierarchy:

```java
import org.tstrans.mpegts.*;

byte[] ts = java.nio.file.Files.readAllBytes(java.nio.file.Path.of("capture.ts"));
try (Demuxer d = new Demuxer()) {
    d.feed(ts);
    d.flush();
    for (DemuxEvent e : d) {
        if (e instanceof DemuxEvent.ProgramMap pm) {
            System.out.println("PSI: program " + pm.programNumber() + ", " + pm.elementaryPids().size() + " streams");
        } else if (e instanceof DemuxEvent.Video v) {
            System.out.println("Video pid=" + v.stream().pid() + " pts=" + v.pts() + " len=" + v.payload().remaining());
        } else if (e instanceof DemuxEvent.Metadata m) {
            System.out.println("KLV pid=" + m.stream().pid() + " kind=" + m.kind() + " len=" + m.payload().remaining());
        } else if (e instanceof DemuxEvent.NonConformant nc) {
            System.out.println("non-conformant: " + nc.kind() + " — " + nc.issue());
        }
        // Audio / Subtitle / UnknownSample / Discontinuity / ReconnectDiscontinuity handled similarly.
    }
}
```

`Demuxer` `implements AutoCloseable, Iterable<DemuxEvent>`. The shape is:
`feed(byte[])` enqueues parsed events, `flush()` drains any buffered
partial PES, and iterating (or calling `nextEvent()`) pulls the
currently-queued events. `nextEvent()` returns `null` when the queue is
empty; the `for`-each loop stops at the same point. Call `feed` / `flush`
again to enqueue more, then iterate again.

On **JDK 21+** you can `switch` on the sealed `DemuxEvent` hierarchy with
pattern matching, but this binding targets JDK 17 where `instanceof`
patterns are the portable form — the examples here stay on 17.

### Configured demuxer

Pass a `DemuxerConfig` built with the fluent builder to tighten parsing
behavior — for example, enable full strict mode and turn off CFI
tolerance:

```java
import org.tstrans.mpegts.*;

DemuxerConfig cfg = DemuxerConfig.builder()
    .strictMode(StrictMode.FULL)
    .cfiTolerance(false)
    .pesCapPerPid(4_000_000)
    .build();

try (Demuxer d = new Demuxer(cfg)) {
    d.feed(ts);
    d.flush();
    for (DemuxEvent e : d) {
        // ...
    }
}
```

The 7 config knobs:

| Knob | Type | Default | Effect |
|---|---|---|---|
| `strictMode` | `StrictMode` | `OFF` | Strictness ladder: `OFF` / `TIMING_ONLY` / `PSI_ONLY` / `FULL`. |
| `cfiTolerance` | `boolean` | `true` | Tolerate cell-fragment-indication producer bugs. |
| `pesCapPerPid` | `long` | `0` (Rust default) | Per-PID PES reassembly byte cap. |
| `pesCapTotal` | `long` | `0` (Rust default) | Total PES reassembly byte cap. |
| `auCellCapPerPid` | `long` | `0` (Rust default) | Per-PID AU-cell reassembly byte cap. |
| `av1Carriage` | `Av1CarriageMode` | `MPEG2_TS_BINDING` | AV1 carriage: `MPEG2_TS_BINDING` or `INTEROP_RAW_OBU`. |
| `lenientPsiReassembly` | `boolean` | `false` | Relax PSI section reassembly. |

A `long` knob of `0` means "use the Rust core's default cap."

### The `DemuxEvent` hierarchy

`DemuxEvent` is a JDK-17 `sealed interface` whose variants are `record`s:

- `ProgramMap(int programNumber, int pcrPid, List<Integer> elementaryPids)` — PSI / PMT.
- `Video(StreamId stream, long pts, Long dts, ByteBuffer payload, boolean randomAccessIndicator)`
- `Audio(StreamId stream, long pts, Long dts, ByteBuffer payload)`
- `Subtitle(StreamId stream, long pts, Long dts, ByteBuffer payload)`
- `UnknownSample(StreamId stream, long pts, Long dts, int streamType, ByteBuffer payload)`
- `Metadata(StreamId stream, long pts, MetadataKind kind, ByteBuffer payload, boolean wasReassembled, int cellCount)` — KLV.
- `NonConformant(StreamId stream, String issue, NonConformantKind kind, MultiCellAuReason multiCellAuReason, CellFragmentIndication observedCfi, CellFragmentIndication treatedAs)`
- `Discontinuity(StreamId stream, DiscontinuityKind kind)`
- `ReconnectDiscontinuity()`

`dts` is a nullable boxed `Long` (null when the PES carried no DTS). On
`NonConformant`, the trailing three fields are `null` except for the
relevant kind: `multiCellAuReason` is non-null only when
`kind == MULTI_CELL_AU`, and `observedCfi` / `treatedAs` are non-null only
when `kind == CFI_TOLERATED`.

`StreamId(int pid, StreamKind kind, int programNumber)` carries the source
PID, the typed stream kind, and the owning program number. `StreamKind` is
itself a sealed interface: `Video(VideoCodec codec)`,
`Audio(AudioCodec codec)`, `Subtitle(SubtitleCodec codec)`,
`KlvSync(Integer declaredLink)`, `KlvAsync()`, and
`Unknown(int streamTypeByte)`.

**Enums:**

- `VideoCodec` — `H264`, `H265`, `H266`, `AV1`
- `AudioCodec` — `MP2`, `AAC`, `AAC_LATM`, `AC3`
- `SubtitleCodec` — `DVB_SUBTITLING`, `DVB_TELETEXT`, `CEA708_STANDALONE`, `WEBVTT_IN_TS`
- `MetadataKind` — `KLV_SYNC_AU_CELL`, `KLV_ASYNC`, `UNKNOWN`
- `DiscontinuityKind` — `CONTINUITY_JUMP`, `PES_OVERSIZE`, `PES_TOTAL_OVERSIZE`, `ADAPTATION_FIELD_FLAG`
- `CellFragmentIndication` — `MIDDLE`, `LAST`, `FIRST`, `COMPLETE`
- `MultiCellAuReason` — `ORPHAN`, `SEQUENCE_GAP`, `CONCURRENT_FIRST`, `OVERFLOW`
- `StrictMode` — `OFF`, `TIMING_ONLY`, `PSI_ONLY`, `FULL`
- `Av1CarriageMode` — `MPEG2_TS_BINDING`, `INTEROP_RAW_OBU`
- `NonConformantKind` — a collapsed discriminant; the `issue` String carries the detail (see the gotcha below).

## Typed KLV (`org.tstrans.klv`)

The `org.tstrans.klv` package exposes fully typed decode and encode for the
four MISB KLV set families that the demuxer surfaces on `DemuxEvent.Metadata`
payloads. All types are immutable Java `record`s; all decode / encode goes
through the static `Klv` façade.

### Decode an ST 0601 UAS Datalink LS

Pass the raw `ByteBuffer` payload bytes from a `DemuxEvent.Metadata` event
directly to `Klv.decodeUasDatalink`. The buffer includes the 16-byte SMPTE
Universal Label — the decoder reads it as part of its verification.

```java
import org.tstrans.klv.*;

// Inside a demux loop where `e` is a DemuxEvent.Metadata:
if (e instanceof DemuxEvent.Metadata m) {
    // Copy the heap ByteBuffer to a byte[] for Klv.decodeUasDatalink.
    java.nio.ByteBuffer view = m.payload().duplicate();
    byte[] klvBytes = new byte[view.remaining()];
    view.get(klvBytes);

    if (Klv.isSt0601Family(klvBytes)) {
        UasDatalinkLs ls = Klv.decodeUasDatalink(klvBytes);  // throws KlvDecodeException

        // Composite accessor: sensor GPS position (lat/lon/alt).
        ls.sensorPosition().ifPresent(pos ->
            System.out.printf("sensor: %.6f, %.6f, %.1fm%n",
                pos.latDeg(), pos.lonDeg(), pos.altM()));

        // Composite accessor: frame-center coordinates (falls back to
        // offset calculations when absolute coordinates are absent).
        ls.frameCenter().ifPresent(fc ->
            System.out.printf("frame center: %.6f, %.6f%n",
                fc.latDeg(), fc.lonDeg()));

        // Non-fatal field errors: tags that decoded partially.
        for (KlvFieldError fe : ls.fieldErrors()) {
            System.out.println("field error tag=" + fe.tag() + " " + fe.kind());
        }
    }
}
```

`decodeUasDatalink` is **lenient by default**: it accepts any 16-byte UL,
verifies the Tag-1 checksum, and collects per-field parse failures in
`fieldErrors()` rather than throwing. Pass `strict=true` / `compliance=true`
to the three-argument overload for stricter behaviour:

```java
// Strict: requires the ST 0601 family UL pattern.
UasDatalinkLs ls = Klv.decodeUasDatalink(klvBytes, /*strict=*/ true, /*compliance=*/ false);

// Compliance: also enforces Tag-2 first / Tag-1 last / Tag-65 present.
UasDatalinkLs ls = Klv.decodeUasDatalink(klvBytes, /*strict=*/ true, /*compliance=*/ true);
```

### Encode an ST 0601 UAS Datalink LS

Build an `UasDatalinkLs` with its `Builder`, push only the fields you want,
then call `Klv.encodeUasDatalink`. Encoding is lenient (no mandatory-tag
enforcement); use `encodeUasDatalinkStrictCompliance` to enforce the full
compliance rules.

```java
import org.tstrans.klv.*;

// Build a minimal record: timestamp + version (required by strict compliance).
UasDatalinkLs ls = new UasDatalinkLs.Builder()
    .timestampUs(1_700_000_000_000_000L)  // microseconds (Tag 2)
    .declaredVersion(17)                   // MISB ST 0601.17 (Tag 65)
    .build();

// Lenient encode — emits only populated fields.
byte[] wire = Klv.encodeUasDatalink(ls);  // throws KlvEncodeException

// Strict-compliance encode — enforces mandatory tags (Tags 1/2/65).
byte[] strictWire = Klv.encodeUasDatalinkStrictCompliance(ls);
```

The encode round-trip is byte-identical across the Rust, Python, and JVM
bindings for the same input record.

### Universal-label dispatcher (`parseUniversal`)

`Klv.parseUniversal(byte[])` inspects the first 16 bytes (the SMPTE UL) and
routes to the correct typed decoder. It returns `Optional<KlvSet>` — empty
for an unrecognised UL, or a concrete `KlvSet` implementer for a known one.
Use `instanceof` on JDK 17 to dispatch:

```java
import org.tstrans.klv.*;
import java.util.Optional;

Optional<KlvSet> result = Klv.parseUniversal(klvBytes);  // throws KlvDecodeException
if (result.isPresent()) {
    KlvSet set = result.get();
    if (set instanceof UasDatalinkLs ls) {
        System.out.println("ST 0601: sensorPos=" + ls.sensorPosition());
    } else if (set instanceof SecurityLs sec) {
        System.out.println("ST 0102: class=" + sec.securityClassification());
    } else if (set instanceof PrecisionTimeStampPack ptp) {
        System.out.println("ST 0605: ts=" + ptp.timestampUs() + " µs");
    } else if (set instanceof VmtiLs vmti) {
        System.out.println("ST 0903: " + vmti.targets().size() + " targets");
    }
} else {
    System.out.println("unrecognised UL");
}
```

For body-only sets (ST 0102 / ST 0903), `parseUniversal` peels the 16-byte UL
and the outer BER length before calling the per-set decoder. For the others
(ST 0601 / ST 0605), the full buffer is passed through.

### Other typed-set families

**ST 0102 — Security Metadata LS** (body-only — no UL / outer BER wrapper):

```java
// Decode body bytes (no UL / outer BER).
SecurityLs secLenient = Klv.decodeSecurity(bodyBytes);        // lenient
SecurityLs secStrict = Klv.decodeSecurity(bodyBytes, true);   // strict (rejects missing required tags)

// Encode back to body bytes.
byte[] body = Klv.encodeSecurity(secLenient);  // throws KlvEncodeException

// Enum accessors: typed + raw codepoint preserved for unknown values.
secLenient.securityClassification();         // Optional<SecurityClassification>
secLenient.securityClassificationCode();     // Integer (raw code, or null if tag absent)
```

**ST 0605 — Precision Time Stamp Pack** (full 26-byte framing):

```java
PrecisionTimeStampPack pack = Klv.decodePrecisionTimestamp(wireBytes);  // throws KlvDecodeException
System.out.println(pack.timestampUs() + " µs, locked=" + pack.timeStatus().isLocked());

byte[] wire = Klv.encodePrecisionTimestamp(pack);  // infallible; always 26 bytes
```

**ST 0903 — VMTI LS** (body-only for decode; two encode forms):

```java
VmtiLs vmtiLenient = Klv.decodeVmti(bodyBytes);        // lenient
VmtiLs vmtiStrict = Klv.decodeVmti(bodyBytes, true);   // strict

System.out.println(vmtiLenient.targets().size() + " targets");

byte[] body = Klv.encodeVmti(vmtiLenient);               // body only (no UL / BER / checksum)
byte[] framed = Klv.encodeVmtiStandalone(vmtiLenient);   // full [UL][BER][body][Tag1 checksum]
```

### Field-error model

Lenient decode is non-throwing for per-field problems. Errors that the Rust
core can recover from (malformed tag value, unsupported IMAPB length, invalid
codepoint, …) are collected in the set's `fieldErrors()` list as
`KlvFieldError(KlvFieldErrorKind, long tag, String message)`. Tags that fail
are skipped; all other tags decode normally.

```java
UasDatalinkLs ls = Klv.decodeUasDatalink(klvBytes);
for (KlvFieldError fe : ls.fieldErrors()) {
    // KlvFieldErrorKind: OUT_OF_RANGE, INVALID_UTF8, INVALID_LENGTH, ...
    System.out.printf("  tag %d: %s — %s%n", fe.tag(), fe.kind(), fe.message());
}
```

`fieldErrors()` returns an empty list when decoding succeeds without any
per-field problem. A non-empty list is advisory — the set is still usable;
the affected tags are missing from the typed fields.

### KLV byte fields are heap `ByteBuffer` copies

Fields typed `ByteBuffer` in the KLV records (for example `UasDatalinkLs`'s
`vmti()` or `securityLocalSet()`, `VTargetPack`'s `vmask()`, etc.) are
heap-`ByteBuffer.wrap(byte[])` copies — the same JDK-17 safety rule that
governs `DemuxEvent` payloads. Direct buffers over Rust memory are not
offered on this baseline; safe zero-copy is deferred to a JDK-22+ Foreign
Function & Memory API path. When hand-constructing a `ByteBuffer` to pass to
a builder setter, always use `ByteBuffer.wrap(byte[])`:

```java
// Correct: heap copy, safe on JDK 17.
vmtiLsBuilder.miisId(java.nio.ByteBuffer.wrap(miisIdBytes));

// Wrong: direct buffer would be rejected (and unsafe on JDK < 22).
// ByteBuffer.allocateDirect(16).put(miisIdBytes)  — do NOT do this
```

## Language-specific gotchas

- **`payload` is a heap-copied, JVM-owned `ByteBuffer`.** Each
  sample / metadata payload is a **copy** of the demuxed bytes, not a view
  over Rust memory. That makes it safe to retain indefinitely — the buffer
  stays readable after the next `nextEvent()` pull and after the `Demuxer`
  is `close()`d. True zero-copy (a direct `ByteBuffer` over native memory)
  is deferred to a future JDK-22+ path built on the Foreign Function &
  Memory API (`Arena` / `MemorySegment`), where the buffer's lifetime can
  be tied to a confined arena. On the JDK-17 baseline this binding copies —
  a direct buffer over Rust-owned memory would be a use-after-free
  foot-gun, so it is deliberately not offered here.
- **`dts` is a nullable `Long`** — boxed, not a primitive `long`. It is
  `null` when the PES carried no DTS. Null-check before unboxing.
- **`codec` lives on `StreamId.kind()`, not on the event record.** A
  `Video` event does not carry its codec directly; read it from the stream:
  `((StreamKind.Video) v.stream().kind()).codec()`. The event records
  intentionally don't duplicate the codec.
- **`Demuxer` is single-threaded** — the consumer owns concurrency. Don't
  share one `Demuxer` across threads without external synchronization.
  Iterating drains the currently-queued events; call `feed` / `flush` to
  enqueue more.
- **`NonConformant` collapses** the Rust core's 30+-variant issue set into
  a single `NonConformantKind` enum plus a human-readable `issue` String
  (and the optional CFI / multi-cell-reason fields). Match on `kind` for
  programmatic dispatch; read `issue` for the human-facing detail.
- **Payloads stay raw `ByteBuffer`** — typed elementary-stream payloads
  (NAL units, AV1 OBUs, ADTS frames) are **not** parsed in this wave. The
  payload is raw bytes; typed payloads land in the codec wave.

## Where this binding differs from the Rust core

- **Demux + offline mux + typed KLV shipped.** The JVM binding currently
  surfaces the `org.tstrans.mpegts.Demuxer` receive path (feed bytes →
  typed `DemuxEvent`s), the offline `org.tstrans.mpegts.Muxer` send path
  (config builder → push family → `pull`), the full `org.tstrans.klv`
  typed-KLV surface (ST 0601 / 0102 / 0605 / 0903 decode + encode +
  `parseUniversal` dispatcher), and the `org.tstrans.Version` bootstrap.
  Codec parsers (`org.tstrans.codec`) and SRT / RTP transport are on the
  roadmap. The Rust core has them; only the JNI wrap is the remaining work.
- **Payloads are raw `ByteBuffer`** (heap copies), not typed NAL / OBU /
  ADTS lists. Typed payloads land in the codec wave.
- **JDK 17 baseline.** The examples use `instanceof` pattern matching, not
  `switch`-on-sealed (which needs JDK 21+). `switch` patterns work on
  21+, but `instanceof` is the portable form on the 17 baseline.
- **`payload` is a heap-copied `ByteBuffer`**, not a direct buffer over
  native memory. Safe-zero-copy is deferred to a JDK-22+ Foreign Function &
  Memory API (`Arena`) path — see the gotcha above.
- **Single fat JAR** (planned) bundles the per-platform native library
  (`.so` / `.dylib` / `.dll`); the `NativeLoader` extracts the correct one
  at runtime. No per-platform classifier.

The Rust page's "Where this binding differs from the Rust core" section
treats Rust as the canonical surface; everything here is a subset of it.
See [`/docs/languages/rust.md`](/docs/languages/rust.md) for the full
surface and [`/docs/languages/python.md`](/docs/languages/python.md) for the
Python binding's gaps.

## Design

See [docs/specs/2026-05-27-tst-jni-design.md](../../docs/specs/2026-05-27-tst-jni-design.md)
(at parent-level project tree, outside the published repo).

## Roadmap

- **Bootstrap (`org.tstrans.Version`) — SHIPPED.** Proves the
  cargo → cdylib → Gradle → Java → JNI build pipeline and native loader.
- **mpegts demux (`org.tstrans.mpegts.Demuxer` + `DemuxEvent` + `DemuxerConfig`) — SHIPPED (this wave).**
- **mpegts mux (`org.tstrans.mpegts.Muxer` + `MuxerConfig` + push family + `pull`) — SHIPPED (this wave).**
- **klv** — typed KLV decode/encode (ST 0601 / 0102 / 0605 / 0903) under `org.tstrans.klv` — **SHIPPED (this wave).**
- **codec** — H.264 / H.265 / H.266 / AV1 + audio parsers under
  `org.tstrans.codec`; typed elementary-stream payloads (NAL / OBU / ADTS).
- **io** — file inspection helpers.
- **srt** — live SRT transport (Sender / Receiver / MuxSender / DemuxReceiver).
- **rtp** — MPEG-TS-over-RTP transport.
- **pipeline** — reconnect wrappers + pairing shells.
- **multi-platform fat JAR + Maven Central publish** — single JAR bundling
  linux-x86_64 / linux-aarch64 / macos-arm64 / macos-x86_64 / windows-x86_64
  native libraries, published as `org.tstrans:tstrans-jvm`.

## Where to go next

- [`/docs/start/concepts.md`](/docs/start/concepts.md) — the conceptual
  model (mux/demux, KLV, transport) before any code.
- [`/docs/guides/mpegts-demux.md`](/docs/guides/mpegts-demux.md) — the full
  demuxer contract: strict-mode ladder, AU-cell unwrap behavior,
  non-conformant handling. The JVM `Demuxer` is a thin wrap over this.
- [`/docs/guides/klv.md`](/docs/guides/klv.md) — the KLV substrate the
  `Metadata` event payloads carry; the `org.tstrans.klv` typed-decode
  surface mirrors this guide module-for-module.
- [`/docs/languages/rust.md`](/docs/languages/rust.md) — the canonical Rust
  surface this binding mirrors.
