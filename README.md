# srt-rust

Cross-platform SRT-based libraries for live video streaming from **gimbaled platforms** — drones (rotary and fixed-wing UAVs), manned fixed-wing aircraft with sensor pods, helicopters with EO/IR turrets, and other manned/unmanned platforms carrying stabilized imaging payloads.

**Status:** scaffolded; implementation pending.

## Scope (v0)

- Container: **MPEG-TS**
- Metadata: **MISB ST 0601 KLV** (multiplexed per MISB ST 1402 / ST 1910)
- Transport: **SRT** (Haivision libsrt 1.5.5)

## Architecture

A Rust core wrapping libsrt via FFI, with bindings for JVM (JNI, JDK 17+), iOS/Android (UniFFI), and embedded Linux (cdylib + cbindgen). MPEG-TS demux is deferred — receivers use FFmpeg/JavaCV/Bento4/platform demuxers and feed extracted KLV bytes through this crate's `klv::Decoder`.

The full v0 design — module layout, binding strategy, distribution model, and decision log — is in `../docs/specs/2026-04-29-srt-libraries-design.md` (parent workspace, not in this repo). Deferred features and triggers to revisit them are in `../docs/deferred-features.md`.

## Workspace layout

```
crates/
  srt-sys/      raw libsrt FFI (bindgen-generated against libsrt 1.5.5)
  srt-core/     safe Rust API — srt::, klv::, mpegts::, pipeline::
  srt-c/        cdylib + cbindgen header — embedded, future Panama/FFM
  srt-jni/      JNI bindings — JAR for JDK 17+ JVM consumers
  srt-uniffi/   Swift/Kotlin via UniFFI — iOS/Android frameworks
```

(Crates are added incrementally as the implementation lands.)

## Building

To be filled in once the workspace has its first crate. The vendored libsrt source is not yet a submodule of this repo; the design assumes pinning at `v1.5.5`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
