# JVM bindings (`org.tstrans`)

> **Who this is for:** You write Java (or any JVM language — Kotlin, Scala,
> Clojure) and want to demux MPEG-TS + KLV streams into typed events on
> JDK 17+.

> **You will learn:**
> - How to build the JVM binding from source today (Maven Central is the planned distribution)
> - How to read a `.ts` file and dispatch typed `DemuxEvent` items
> - How to configure the demuxer with a fluent `DemuxerConfig` builder
> - The JVM-specific gotchas: heap-copied `ByteBuffer` payloads, nullable `Long` DTS, codec on `StreamId`
> - How this binding differs from the Rust core (demux only in this wave; mux / KLV-decode / transport are roadmap)

> **Status (mpegts demux surface shipped):** the JVM binding currently
> ships exactly two things: the bootstrap `org.tstrans.Version` hello-world
> and the complete `org.tstrans.mpegts` **demux** surface (`Demuxer`,
> `DemuxerConfig`, the sealed `DemuxEvent` hierarchy, `StreamId`, and the
> codec / kind enums). Offline `Muxer`, typed KLV decode
> (`org.tstrans.klv`), codec parsers (`org.tstrans.codec`), and SRT / RTP
> transport are on the roadmap — the Rust core has them; only the JNI wrap
> is the remaining work. This page documents only what exists today.

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

- **Demux only in this wave.** The JVM binding currently surfaces the
  `org.tstrans.mpegts.Demuxer` receive path (feed bytes → typed
  `DemuxEvent`s) plus the `org.tstrans.Version` bootstrap. There is no
  `Muxer`, so this page has **no "First send"** — offline `Muxer`, typed
  KLV decode (`org.tstrans.klv`), codec parsers (`org.tstrans.codec`), and
  SRT / RTP transport are all on the roadmap. The Rust core has them; only
  the JNI wrap is the remaining work.
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
- **mpegts mux** — offline `Muxer` + config builder + push family + `pull`.
- **klv** — typed KLV decode (ST 0601 / 0102 / 0605 / 0903) under `org.tstrans.klv`.
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
  `Metadata` event payloads carry (typed JVM decode is roadmap).
- [`/docs/languages/rust.md`](/docs/languages/rust.md) — the canonical Rust
  surface this binding mirrors.
