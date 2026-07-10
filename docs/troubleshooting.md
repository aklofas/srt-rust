# Troubleshooting

Common failure modes you'll hit when building or running this library, with diagnoses and fixes. If you're not finding your symptom here, check the per-module guide for the relevant area, or open an issue at https://github.com/aklofas/ts-transformer/issues.

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

The cdylib needs C++ runtime linkage because libsrt is C++. For `cargo build -p tst-c` this is handled automatically.

Fix: if you're consuming `tstrans.h` from another build system, add `-lstdc++` (Linux) or `-lc++` (macOS) to your link line. The shipped `tstrans.pc` declares the correct `Libs.private`; using `pkg-config --static --libs tstrans` is the safest way to get the right flags.

## Connection failures

**Caller hangs on `connect()`**

Three usual suspects: the listener side isn't actually bound yet, a firewall is dropping UDP (SRT runs over UDP, not TCP), or the peer rejected the handshake but is taking a while to surface that.

Fix: confirm the listener is up with `ss -ulpn | grep <port>`; verify both sides agree on passphrase and key length; if you need a hard upper bound on `connect()`, set `send_timeout(Duration::from_secs(N))` on the `SocketBuilder` (libsrt uses the send timeout during the synchronous handshake path).

**`Listener::accept` blocks forever**

`accept()` is blocking by design and has no built-in deadline. Contrary to what you might expect, `ListenerBuilder::recv_timeout` / `Listener::set_recv_timeout` does *not* gate the accept call — libsrt's `srt_accept` ignores `SRTO_RCVTIMEO`. The recv timeout only applies to accepted sockets (it is inherited as their per-socket read deadline).

Fix: use `Listener::accept_timeout(Duration)` instead of `accept()`. It returns `Err(AcceptError::TimedOut)` when the duration elapses with no incoming connection, and `Ok((socket, peer))` on success:

```rust
use std::time::Duration;
use tst_srt::AcceptError;

loop {
    match listener.accept_timeout(Duration::from_secs(1)) {
        Ok((socket, peer)) => { /* handle */ }
        Err(AcceptError::TimedOut) => { /* check shutdown flag, retry */ }
        Err(AcceptError::ListenerClosed) => break,
        Err(e) => return Err(e.into()),
    }
}
```

Alternatively, run `accept()` on a dedicated thread and call `Listener::close` from your shutdown path — that wakes the blocked call with `AcceptError::ListenerClosed`.

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

**Sender hangs for ~3 minutes when dropping a `Socket`**

`Socket::Drop` blocks the calling thread for up to 180 seconds. Cause: libsrt's default `SRTO_LINGER` is 180 seconds. With no peer ACK on pending sends, `srt_close` (called from `Drop`) blocks until the linger timer expires.

Fix: set `SocketConfig::linger = Some(Duration::ZERO)` for live streaming where late frames are useless, or use the `SocketBuilder::linger(Duration)` setter. The `tst-c` connect path (`bindings/c/core/src/sender/connect.rs::connect_srt`) defaults to 5 seconds — long enough to drain a small backlog, short enough to never block reconnect noticeably.

## TCP / TLS (`tcps://`)

**`tcps://` fails with a certificate error**

TLS verifies the server certificate against exactly what you dialed:

- If you dialed a **hostname** (`tcps://relay.example.com:7001`), the
  certificate must carry a `dnsName` SubjectAltName for that hostname.
- If you dialed an **IP literal** (`tcps://192.168.1.10:7001`), the
  certificate must carry an `iPAddress` SubjectAltName for that address.

A common mistake is generating a cert with only an `iPAddress` SAN and then
dialing a hostname (or vice versa). The fix is to dial what the cert says, or
regenerate the cert to match what you want to dial.

Generate a cert for hostname dialing:
```bash
openssl req -x509 -nodes -newkey rsa:2048 -subj "/CN=relay.example.com" \
  -addext "subjectAltName=DNS:relay.example.com" -out server.crt -keyout server.key
```

Generate a cert for IP-literal dialing:
```bash
openssl req -x509 -nodes -newkey rsa:2048 -subj "/CN=server" \
  -addext "subjectAltName=IP:192.168.1.10" -out server.crt -keyout server.key
```

Reference this certificate with `?cert=server.crt&key=server.key` on the
listener URL, and add the CA to the caller's trust store (`?ca=ca.crt`) or to
the OS native trust store. Listener bind addresses still require IP literals
(`0.0.0.0` / `::`); the listener-vs-caller asymmetry is intentional.

## KLV decode rejection

**`decode_strict_compliance` rejects**

The record violates one of ST 0601.8-09 / -11 / -12's mandatory rules: Tag 2 (timestamp) must be the first element, Tag 1 (checksum) must be the last element, Tag 65 (UAS LS Version) must be present. The corresponding `KlvDecodeError` variants are `Tag2NotFirst`, `Tag1NotLast`, and `MissingTag65`.

Fix: walk the strictness ladder — fall back to `decode_strict` (validates UL family + checksum but not ordering) or plain `decode` (validates checksum only) to inspect the record despite non-compliance. If the producer is yours, fix the producer to emit the mandatory tags in the correct order. Worked example: [../examples/klv-metadata/klv_decode_file.rs](../examples/klv-metadata/klv_decode_file.rs).

**`decode` rejects with `KlvDecodeError::ChecksumMismatch`**

Either Tag 1 (checksum) wasn't emitted by the producer, or the bytes were corrupted in transit.

Fix: try `decode_unchecked` to get a parsed record without checksum validation. If `decode_unchecked` returns sensible values, transit corruption is the likely cause. If it returns nonsense values (out-of-range coordinates, truncated strings), the producer is broken — investigate that side instead.

**`decode_strict` rejects with `KlvDecodeError::UnexpectedUniversalLabel`**

The record's 16-byte universal label isn't in the ST 0601 family that `decode_strict` accepts.

Fix: use plain `decode` (no UL family check) if you're handling non-ST-0601 records, or validate the UL upstream before dispatching.

**`KlvDecodeError::DuplicateTag`**

The record contains the same tag twice in its top-level list. ST 0601 disallows duplicates within a single LS.

Fix: this is almost always a producer bug; fix the producer.

**`NonConformantIssue::MultiCellAu` events on a sync KLV PID**

A multi-cell AU reassembly attempt failed on the named PID. `reason` discriminates the failure mode:

- `Orphan` — a `Middle` or `Last` cell arrived without a prior `First`. Either the stream started mid-AU (e.g. seek into a recording) or a `First` cell was lost upstream. (Note: the producer-side CFI malformation pattern — encoders shipping `0b00` (Middle) on single-cell AUs — is rescued by the default-on `cfi_tolerance` knob and does NOT produce an Orphan event under default config. You will only see Orphan here for legitimate fragmentation losses or if you explicitly set `cfi_tolerance: false`.)
- `SequenceGap` — a buffered AU's continuation cell had the wrong `sequence_number`. A cell was lost between the buffered `First`/`Middle` and the arriving cell.
- `ConcurrentFirst` — a new `First` arrived while the previous AU was still buffering (its `Last` never appeared). The partial buffer is dropped before the new `First` is processed.
- `Overflow` — the accumulated inner-byte total would exceed `DemuxerConfig::au_cell_cap_per_pid` (default 1 MiB). Tune the cap via `DemuxerConfigBuilder::au_cell_cap_per_pid(bytes)`.

Fix: for `SequenceGap` and `Overflow`, investigate the upstream sender. If `ts-transformer`'s muxer is the sender, this is automatic — `Muxer::push_klv*` always emits `Complete` cells. Legitimate multi-cell streams reassemble transparently into a single `MetadataKind::KlvSyncAuCell` event with `was_reassembled = true` and `cell_count = N`.

**I see `MultiCellAu{Orphan}` events but zero typed KLV from a malformed encoder**

This shouldn't happen under default configuration — the producer-side CFI malformation (encoders shipping `0b00` (Middle) on what are actually single complete KLV records) is rescued by the default-on `cfi_tolerance` knob. If you are seeing `Orphan` events with zero KLV, check whether you have explicitly opted into strict mode:

```rust,ignore
Demuxer::with_config(DemuxerConfig::builder().cfi_tolerance(false).build())  // strict — disables the rescue
```

To restore tolerance, either remove the `.cfi_tolerance(false)` call or set it back to `true`. The demuxer then payload-validates the orphan cell as one complete KLV unit (SMPTE 336M UL prefix + BER length match) and, if it passes, emits the cell as `KlvSyncAuCell{Complete}` plus a `NonConformantIssue::CfiTolerated { pid, observed_cfi, treated_as }` diagnostic so the malformation remains visible to telemetry. See [guides/mpegts-demux.md](/docs/guides/mpegts-demux.md#malformed-cell_fragment_indication-tolerance-default-on) for the full contract.

**I'm running a conformance suite and want spec-strict CFI handling**

Set `cfi_tolerance: false` on the `DemuxerConfig`. Orphan Middle/Last cells then surface as `NonConformantIssue::MultiCellAu { reason: MultiCellAuReason::Orphan }` per H.222.0 V9 §2.12.4.2 Table 2-157 with no metadata event.

## TS framing issues

**`Sender` in `TsFramingMode::Strict` errors on the first push**

Strict mode requires the input bytes to start with a TS sync byte (`0x47`) at offset 0 with the standard 188-byte cadence. If your upstream producer emits a partial packet at the boundary or has any byte-level offset, strict mode rejects rather than realigning.

Fix: switch to `TsFramingMode::Recover` to auto-resync, or fix the producer to emit aligned bytes from the start. See [guides/pipeline.md](/docs/guides/pipeline.md) for the framing state machine details.

**Receiver gets garbled TS**

After the run, check `Sender::stats()` and inspect `bytes_skipped_for_sync` and `resync_events`. If either is nonzero in production, the producer is emitting non-aligned bytes intermittently. In `Recover` mode the sender still emits a clean stream (it realigns silently), so the receiver should be fine; in `Strict` mode you'd have already errored.

Fix: if the receiver is still seeing garble despite zero stats, the corruption is happening downstream of the sender — check the network path and any intermediate transcoders.

**Receiver sees double-wrapped KLV (legacy callers from older library versions)**

If you previously passed pre-wrapped bytes to `Muxer::push_klv` for a `KlvStreamType::SynchronousMetadata` stream (older library versions where the caller had to wrap), the muxer now double-wraps. Strip the outer wrapper and let the muxer wrap once.

Fix: pass raw KLV LS bytes (16-byte SMPTE UL + BER length + body) to `Muxer::push_klv` / `MuxSender::send_klv`; the muxer auto-prepends a 5-byte `Metadata_AU_cell` header per ITU-T H.222.0 V9 § 2.12.4.2 (Tables 2-155+2-156). PTS lives in the PES header (§ 2.12.4.1). Asynchronous KLV streams (`KlvStreamType::PrivateData`) pass the raw 0601 LS bytes through unchanged. See [guides/mpegts-mux.md](/docs/guides/mpegts-mux.md) for the synchronous vs. asynchronous distinction.

**`MuxError::KlvTooLarge`**

Your KLV blob exceeds the PES_packet_length ceiling (65532 bytes without PTS, 65527 with PTS). ST 0601 packs are typically <2 KB so this is a sanity check, not a normal failure mode.

Fix: investigate why your producer emitted a multi-KB metadata blob; this is almost always a bug.

## Reconnect loops

**`ManagedTransport` keeps reconnecting fast in a tight loop**

Backoff is set to `BackoffStrategy::Constant(Duration::ZERO)`, or `Exponential` with a too-low base.

Fix: use the default `BackoffStrategy::Exponential { base: Duration::from_millis(100), max: Duration::from_secs(10) }` (this is what `ReconnectPolicy::default()` returns), or tune the base up if your transport factory is itself expensive.

**Reconnect appears to succeed but no data flows after**

The gap buffer overflowed during the disconnect window. With the default `OverflowPolicy::DropOldest` newer messages displace older ones; with `OverflowPolicy::Reject` new sends fail outright. Either way, some messages were lost between the break and the reconnect.

Fix: size `gap_buffer_capacity` to your worst-case disconnect window times your send rate. The default of 256 messages is fine for a 1 Hz KLV stream over a 4-minute outage; for higher-rate video you'll want to budget more aggressively. See [guides/pipeline.md](/docs/guides/pipeline.md) for the sizing math.

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

## Performance and reliability

**High `pktRcvLossTotal` on a stable network**

SRT reports loss / retransmits on a network you know is healthy. Cause: kernel UDP socket buffer overflow, common above ~25 Mbps. The kernel drops UDP packets before SRT can drain them; SRT sees the gaps as transmission losses and triggers ARQ retransmits.

Diagnosis: `cat /proc/net/udp` (or `ss -unp`) — non-zero `drops` column on the SRT port confirms.

Fix: set `SocketConfig::udp_recv_buffer_bytes = Some(12_500_000)` (or higher) for the receiver. For 100 ms RTT @ 25 Mbps, ~12.5 MB is the recommended floor. Linux clamps to `net.core.rmem_max` — raise with `sysctl -w net.core.rmem_max=33554432` if needed.

## All `UnpairedVideo`, zero `Paired`

**Symptom:** Using `tst_pipeline::Pairer::with_config` with `PairerMode::Realtime`,
the stats report `paired = 0` and `unpaired_video` matches your video event count.
KLV events are present (PMT shows the stream, demux events arrive).

**Most likely cause:** the encoder interleaves the KLV PES *after* its
matching video PES on the wire. Realtime mode's past-only history search
sees no KLV when the video event arrives, so every video pairs as
`UnpairedVideo`. The KLV then arrives, ingests into history, and never
finds a video that needs it (Realtime doesn't look back at past videos).

**Fix:** switch to `PairerMode::Buffered { max_lag: Duration::from_secs(2) }`
and bump `max_buffered_video` to ≈60 (≈2 s @ 30 fps). Buffered mode holds
video briefly to look ahead for KLV; the trade-off is up to
`max_buffered_video` × frame-period of pairing-induced latency.

```rust,ignore
use std::time::Duration;
let mut opts = PairerConfig::default();
opts.mode = PairerMode::Buffered { max_lag: Duration::from_secs(2) };
opts.tolerance = Duration::from_millis(300);
opts.max_buffered_klv = 32;
opts.max_buffered_video = 60; // ≈2 s @ 30 fps
let pairer = Pairer::with_config(video_pid, klv_pid, opts);
```

If `paired` is still zero after switching, the cause is not interleave
order — check the PIDs, tolerance, and `MetadataKind` distribution
(`KlvSyncAuCell` vs `KlvAsync` are both treated as KLV candidates, so
filtering by kind is not the issue).

## UDP / RIST receive cancellation

**A UDP or RIST `recv` blocks forever and cannot be stopped from another thread**

**Symptom:** a thread parked in `UdpRecvTransport::recv_bytes` (or the
Python equivalent) does not return when another thread tries to shut it
down. SRT, RTP, and TCP expose a cloneable cancel handle
(`SrtCancelHandle` / `RtpCancelHandle` / `TcpCancelHandle`) that can be
fired from any thread; UDP and RIST have no equivalent.

**Diagnosis:** there is no race-free way to interrupt a live UDP or RIST
receive from another thread. Both `recv_bytes` and `close()` take `&mut
self`, so they cannot be called concurrently in safe Rust — calling
`close()` from another thread while `recv` is in flight is not possible
without unsafe code, and wrapping the transport in a `Mutex` just
reproduces the GIL-freeze shape (the mutex blocks the closer until the
recv finishes, so nothing is gained). The supported shutdown pattern is
cooperative: pass a finite per-call timeout and check a stop flag between
calls. See the [deferred-features entry](/docs/project/deferred-features.md)
for the deferral rationale.

**Fix:** use `recv_timeout` (UDP Rust) or `timeout_ms` (Python UDP/RIST)
for a bounded per-call deadline, and check a stop flag in the caller loop.
The owning thread calls `close()` once it decides to stop, between `recv`
calls:

```rust,ignore
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

let stop = Arc::new(AtomicBool::new(false));
let stop_for_signal = Arc::clone(&stop);

// Signal thread: set the flag. The recv loop notices on the next
// timeout tick (within `timeout_ms`). No reference to the transport
// is needed here — cooperative stop via an AtomicBool.
std::thread::spawn(move || {
    std::thread::sleep(std::time::Duration::from_secs(30));
    stop_for_signal.store(true, Ordering::Release);
});

// Recv loop on the owning thread — the ONLY thread that calls recv or close.
let mut buf = vec![0u8; 65535];
loop {
    if stop.load(Ordering::Acquire) {
        recv_transport.close(); // safe: called by the owning thread
        break;
    }
    match recv_transport.recv_timeout(&mut buf, std::time::Duration::from_millis(200)) {
        Ok(Some(n)) => { /* process buf[..n] */ }
        Ok(None) => continue,  // timeout tick — loop back and check stop
        Err(e) => return Err(e.into()),
    }
}
```

If you need to interrupt the recv from a thread that does not own the
transport, consider switching to SRT, RTP, or TCP — all three expose a
cloneable cancel handle that is safe to store and fire from any context.
See [srt-cancel-handle.md](/docs/reference/srt-cancel-handle.md) for the
cancel-handle pattern.
