# Runnable examples

Hands-on, runnable companions to the cookbook recipes. Every example is
self-contained, builds via `cargo`, and prints a forward-pointer at the
end so you know what to read next.

```sh
cargo run -p tst-examples --example <name>
```

`<name>` is the basename without `.rs`. To list everything cargo knows
about: `cargo build -p tst-examples --examples` (it'll tell you each
example's name on the first build).

## Where to start

| If you want to… | Start here |
|---|---|
| See the smallest possible thing this library does | [`getting-started/hello_world.rs`](getting-started/hello_world.rs) |
| Send video + KLV over an SRT link | [`sending/pipeline_send_to_socket.rs`](sending/pipeline_send_to_socket.rs) |
| Write an MPEG-TS file from scratch | [`muxing/mux_to_file.rs`](muxing/mux_to_file.rs) |
| Decode KLV from a `.ts` capture | [`receiving/demux_to_events.rs`](receiving/demux_to_events.rs) → [`klv-metadata/extract_klv.rs`](klv-metadata/extract_klv.rs) → [`klv-metadata/klv_decode_file.rs`](klv-metadata/klv_decode_file.rs) |
| Pair video AUs with KLV records | [`pairing/pair_klv_pipeline.rs`](pairing/pair_klv_pipeline.rs) |
| Pull resolution / profile / sample-rate out of a stream | [`codec-parsing/parse_video_parameters.rs`](codec-parsing/parse_video_parameters.rs) |

## Categories

Each folder is a curriculum: examples are numbered in read-order in the
folder's own README, with explicit "diffs from previous" pointers where
the progression is cumulative.

| Folder | Covers | Cookbook |
|---|---|---|
| [`getting-started/`](getting-started/) | 1-page first-encounter example | [§0](../docs/cookbook.md#0-send-a-single-ts-packet-to-any-transport) |
| [`sending/`](sending/) | SRT + transport-trait senders | [Sending](../docs/cookbook.md#sending) |
| [`muxing/`](muxing/) | File-only mux (no SRT); single + multi-program; codecs | [Muxing](../docs/cookbook.md#muxing) |
| [`receiving/`](receiving/) | SRT receivers + file-replay demux | [Receiving](../docs/cookbook.md#receiving) |
| [`klv-metadata/`](klv-metadata/) | ST 0601 / ST 0102 / ST 0903 encode + decode | [KLV metadata](../docs/cookbook.md#klv-metadata) |
| [`pairing/`](pairing/) | Video AU ↔ KLV pairing (manual + `Pairer` helper) | [Pairing](../docs/cookbook.md#pairing-video--klv) |
| [`codec-parsing/`](codec-parsing/) | H.264 / H.265 / H.266 / AV1 parameter sets, audio, subtitles | [Codec parsing](../docs/cookbook.md#codec-parsing) |
| [`operations/`](operations/) | Reconnect, fan-out, ops-flavored patterns | [Operations](../docs/cookbook.md#operations) |

## Maintainer tooling

Fixture-generators are not examples — they're maintainer-only scripts
that regenerate the test corpus at `crates/tst-core/tests/fixtures/`.
They live alongside the tests at `crates/tst-core/tests/tools/` and are
declared as `[[bin]]` targets in `tst-core`:

```sh
cargo run -p tst-core --bin gen_synthetic_fixtures
cargo run -p tst-core --bin gen_subtitle_fixtures -- crates/tst-core/tests/fixtures/subtitles
cargo run -p tst-core --bin gen_h266_fixtures
cargo run -p tst-core --bin gen_av1_fixtures
```

Re-run them only when the encoder is intentionally changed; commit the
diff alongside the change.

## C examples

C ABI examples mirror this taxonomy under
[`../crates/tst-c/examples/c/`](../crates/tst-c/examples/c/). Linux
x86_64 only by build convention; build with `gcc` + the cbindgen-emitted
header. See that folder's README for build invocation.
