# Troubleshooting

Common failure modes you'll hit when building or running this library, with diagnoses and fixes. If you're not finding your symptom here, check the per-module guide for the relevant area, or open an issue at https://github.com/aklofas/srt-rust/issues.

## Build failures

**"could not find python3"**

mbedTLS's build system uses Python for code generation during the encrypted build path. The `mbedtls` cargo feature is on by default, so this trips first-time builders without Python on PATH.

Fix: `sudo apt-get install -y python3` on Debian/Ubuntu; on macOS Python 3 is preinstalled. If you don't need encryption, build with `--no-default-features` to skip the mbedTLS step entirely.

**"submodule X is empty" / "fatal: no submodule mapping"**

You cloned without submodules. `vendor/srt` and `vendor/mbedtls` are git submodules pinned to specific upstream tags, and the build script needs their contents.

Fix: from the repo root, run `git submodule update --init --recursive`.

**"could not find cmake"**

libsrt's build is CMake-based; the build script invokes it when the vendored fallback path is taken.

Fix: `sudo apt-get install cmake` on Debian/Ubuntu, `brew install cmake` on macOS.

**"could not find pkg-config"**

By default the build script tries `pkg-config srt` first to detect a system libsrt before falling back to the vendored build. The pkg-config probe itself needs the `pkg-config` binary on PATH.

Fix: install pkg-config, or set `SRT_FORCE_VENDORED=1` to skip the probe and go straight to the vendored compile.

**First build hangs at "Compiling srt-sys"**

Not actually hung. libsrt and mbedTLS compile from source on a cold build, which takes 3-5 minutes on a typical workstation. Subsequent builds reuse the artifacts and finish in seconds.

Fix: wait it out. Run `cargo build -v` if you want to see what's actually executing.

**Linker error: undefined reference to libstdc++ symbols**

The cdylib needs C++ runtime linkage because libsrt is C++. For `cargo build -p srt-c` this is handled automatically.

Fix: if you're consuming `srtc.h` from another build system, add `-lstdc++` (Linux) or `-lc++` (macOS) to your link line. The shipped `srtc.pc` declares the correct `Libs.private`; using `pkg-config --static --libs srtc` is the safest way to get the right flags.

## Connection failures

**Caller hangs on `connect()`**

Three usual suspects: the listener side isn't actually bound yet, a firewall is dropping UDP (SRT runs over UDP, not TCP), or the peer rejected the handshake but is taking a while to surface that.

Fix: confirm the listener is up with `ss -ulpn | grep <port>`; verify both sides agree on passphrase and key length; if you need a hard upper bound on `connect()`, set `send_timeout(Duration::from_secs(N))` on the `SocketBuilder` (libsrt uses the send timeout during the synchronous handshake path).

**`Listener::accept` blocks forever**

`accept()` is blocking with no built-in timeout. There is no `accept_timeout` knob on `ListenerBuilder` today.

Fix: if you need to bound the wait, run `accept` on a dedicated thread and signal it from your shutdown path, or close the listener from another thread to unblock it (the close call wakes the accept with `AcceptError::ListenerClosed`).

**Connection establishes but no data arrives**

Call `socket.stats()` on both sides. If `bytes_sent` is increasing on the sender but `bytes_received` isn't moving on the listener, the link is up but packets are being dropped on the path. Most often this is an MTU / path-MTU issue.

Fix: confirm both sides are using SRT defaults (1316-byte payload), or set explicit `payload_size(...)` on both builders matching the actual path MTU minus the SRT/UDP/IP overhead.

**`ConnectError::BadEncryption`**

Encryption configuration was rejected before the handshake even ran. Usually one side has a malformed passphrase (the `Passphrase::new` constructor enforces 10-79 ASCII-printable bytes, but raw FFI users can sometimes bypass that).

Fix: build the `Passphrase` through `Passphrase::new` and let the constructor validate.

**`ConnectError::Rejected { reason: RejectReason::BadSecret, .. }`**

Passphrase strings don't match between caller and listener. libsrt rejects with `SRT_REJ_BADSECRET` after the handshake confirms the keying material doesn't agree.

Fix: verify both sides pass byte-identical passphrase strings — mind shell quoting, trailing newlines from heredocs, and environment variables that include leading whitespace.

**`ConnectError::Rejected { reason: RejectReason::Unsecure, .. }` or `AcceptError::PeerRejected { reason: RejectReason::Unsecure, .. }`**

Caller and listener disagree on whether encryption is in use at all. Most common cause: one side built with `--no-default-features` (no `mbedtls` feature, encryption disabled) and the other built with the default feature set.

Fix: build both sides with the same feature configuration. If you need encryption on the link, neither side may be built `--no-default-features`.

## KLV decode rejection

**`decode_strict_compliance` rejects**

The record violates one of ST 0601.8-09 / -11 / -12's mandatory rules: Tag 2 (timestamp) must be the first element, Tag 1 (checksum) must be the last element, Tag 65 (UAS LS Version) must be present. The corresponding `KlvDecodeError` variants are `Tag2NotFirst`, `Tag1NotLast`, and `MissingTag65`.

Fix: walk the strictness ladder — fall back to `decode_strict` (validates UL family + checksum but not ordering) or plain `decode` (validates checksum only) to inspect the record despite non-compliance. If the producer is yours, fix the producer to emit the mandatory tags in the correct order. Worked example: [../crates/srt-core/examples/klv_decode_file.rs](../crates/srt-core/examples/klv_decode_file.rs).

**`decode` rejects with `KlvDecodeError::ChecksumMismatch`**

Either Tag 1 (checksum) wasn't emitted by the producer, or the bytes were corrupted in transit.

Fix: try `decode_unchecked` to get a parsed record without checksum validation. If `decode_unchecked` returns sensible values, transit corruption is the likely cause. If it returns nonsense values (out-of-range coordinates, truncated strings), the producer is broken — investigate that side instead.

**`decode_strict` rejects with `KlvDecodeError::UnexpectedUniversalLabel`**

The record's 16-byte universal label isn't in the ST 0601 family that `decode_strict` accepts.

Fix: use plain `decode` (no UL family check) if you're handling non-ST-0601 records, or validate the UL upstream before dispatching.

**`KlvDecodeError::DuplicateTag`**

The record contains the same tag twice in its top-level list. ST 0601 disallows duplicates within a single LS.

Fix: this is almost always a producer bug; fix the producer.

## TS framing issues

**`TsSender` in `TsFramingMode::Strict` errors on the first push**

Strict mode requires the input bytes to start with a TS sync byte (`0x47`) at offset 0 with the standard 188-byte cadence. If your upstream producer emits a partial packet at the boundary or has any byte-level offset, strict mode rejects rather than realigning.

Fix: switch to `TsFramingMode::Recover` to auto-resync, or fix the producer to emit aligned bytes from the start. See [guide-pipeline.md](guide-pipeline.md) for the framing state machine details.

**Receiver gets garbled TS**

After the run, check `TsSender::stats()` and inspect `bytes_skipped_for_sync` and `resync_events`. If either is nonzero in production, the producer is emitting non-aligned bytes intermittently. In `Recover` mode the sender still emits a clean stream (it realigns silently), so the receiver should be fine; in `Strict` mode you'd have already errored.

Fix: if the receiver is still seeing garble despite zero stats, the corruption is happening downstream of the sender — check the network path and any intermediate transcoders.

**Receiver expects sync KLV (ST 1402) but sees raw bytes**

The muxer does not auto-wrap KLV in ST 1910 AU cells. When you've configured the muxer for `KlvStreamType::SynchronousMetadata`, you must wrap the payload yourself before passing it to the muxer.

Fix: call `klv::st1910::wrap_au_cell(payload, timestamp)` and pass the result to `Muxer::push_klv` or `Sender::send_klv`. Asynchronous KLV streams pass the raw 0601 LS bytes through unchanged. See [guide-mpegts-mux.md](guide-mpegts-mux.md) for the synchronous vs. asynchronous distinction.

**`MuxError::KlvTooLarge`**

Your KLV blob exceeds the PES_packet_length ceiling (65532 bytes without PTS, 65527 with PTS). ST 0601 packs are typically <2 KB so this is a sanity check, not a normal failure mode.

Fix: investigate why your producer emitted a multi-KB metadata blob; this is almost always a bug.

## Reconnect loops

**`ManagedTransport` keeps reconnecting fast in a tight loop**

Backoff is set to `BackoffStrategy::Constant(Duration::ZERO)`, or `Exponential` with a too-low base.

Fix: use the default `BackoffStrategy::Exponential { base: Duration::from_millis(100), max: Duration::from_secs(10) }` (this is what `ReconnectPolicy::default()` returns), or tune the base up if your transport factory is itself expensive.

**Reconnect appears to succeed but no data flows after**

The gap buffer overflowed during the disconnect window. With the default `OverflowPolicy::DropOldest` newer messages displace older ones; with `OverflowPolicy::Reject` new sends fail outright. Either way, some messages were lost between the break and the reconnect.

Fix: size `gap_buffer_capacity` to your worst-case disconnect window times your send rate. The default of 256 messages is fine for a 1 Hz KLV stream over a 4-minute outage; for higher-rate video you'll want to budget more aggressively. See [guide-pipeline.md](guide-pipeline.md) for the sizing math.

**`max_attempts` exhausted; subsequent sends return `TransportError::Closed`**

The policy's retry budget is spent and the managed transport is now a dead end.

Fix: increase `max_attempts`, or set it to `None` to retry forever — only safe if your transport factory is itself rate-limited, otherwise a permanent peer outage produces a hot reconnect loop. The default is `Some(10)` which gives roughly 10 attempts with exponential backoff, on the order of a few minutes of real time before giving up.

## Build-script behaviors

**Want to use a system libsrt instead of the vendored copy**

Leave `SRT_FORCE_VENDORED` unset (the default) and ensure `pkg-config srt --modversion` returns 1.5.0 or newer. The build script probes pkg-config first and uses the system copy when available. If the probe fails or the version is too old, it transparently falls back to the vendored build.

**Want to force the vendored build for reproducibility**

Set `SRT_FORCE_VENDORED=1` (equivalent: `SRT_NO_PKG_CONFIG=1`). This skips pkg-config entirely and always compiles `vendor/srt` from source. Use this in CI and release builds where you want bit-for-bit reproducibility independent of whatever libsrt is installed on the build host.

**Want to skip the encryption build to iterate faster**

Run `cargo build --no-default-features`. This disables the `mbedtls` feature, drops the mbedTLS submodule from the build, and compiles libsrt with `ENABLE_ENCRYPTION=OFF`. Cold builds become roughly 1-2 minutes faster. Both peers must be built the same way — see [Connection failures](#connection-failures) above for the symptom when they disagree.
