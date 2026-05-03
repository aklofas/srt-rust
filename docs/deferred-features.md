# Deferred features

Things deliberately out of scope today with a clear path back if they
become load-bearing. Each entry records the reason it was deferred and
the trigger that would unblock it.

## Audio carriage in `mpegts::mux` and `mpegts::demux`

- **Status:** Sender-side `mpegts::mux` carries video + KLV only.
  Receiver-side `mpegts::demux` reserves audio surface in
  `SamplePayload::Audio { codec: AudioCodec, .. }`; `AudioCodec`
  exists today as an enum with a single hidden `__Reserved` variant
  so a future typed value (e.g. `Aac`) lands additively rather than
  as a breaking change.
- **Why deferred:** Gimbaled-platform streams typically deliver video
  plus KLV with no audio track. Adding audio speculatively means
  guessing codec, framing, and PTS-sync questions that no shipping
  consumer is asking. Adding the typed `AudioCodec` variants and the
  decode path is mechanical when one is asked for.
- **Trigger to revisit:** A consumer ships requiring synchronized
  audio in the same TS as the video and KLV.

## Subtitle, caption, and auxiliary-data channels in `mpegts::mux` and `mpegts::demux`

- **Status:** Same shape as audio — sender side does not emit;
  receiver side reserves `SamplePayload::Subtitle { codec: SubtitleCodec, .. }`
  with `SubtitleCodec` carrying a single hidden `__Reserved` variant.
- **Why deferred:** Same situation as audio — no shipping consumer
  asks for them. The shapes also diverge enough across codecs (DVB
  subtitling, CEA-608/708, teletext, ARIB) that "generic auxiliary
  track" is the wrong abstraction.
- **Trigger to revisit:** A specific channel type asked for by a
  consumer, designed against that channel's actual semantics.

## Other PMT entries / auxiliary services

- **Status:** The muxer emits PAT + PMT with video PID + KLV PID(s)
  only. SCTE-35 splice info, EMM/ECM (conditional access), data
  carousels (DSM-CC), and private-data PIDs beyond KLV are not
  emitted.
- **Why deferred:** No shipping consumer asks for any of them. Adding
  them speculatively risks the same wrong-abstraction trap as
  subtitles — each is its own descriptor + stream_type + PES shape.
- **Trigger to revisit:** A consumer asks for one specifically.
- **Scope when added:** Case-by-case; each carries its own
  descriptor + stream_type + PES framing.

## Async / reactor exposure

- **Status:** Not implemented; the public API is sync blocking.
- **Why deferred:** Consumers fit ten or fewer SRT connections per
  process; thread-per-connection is straightforward and matches
  `std::net::TcpStream` semantics. A reactor-backed surface is
  significantly more design work than the current consumer base
  justifies.
- **Trigger to revisit:** A consumer needing fifty or more concurrent
  connections, or a binding (UniFFI, JNI) that explicitly wants async
  at the FFI surface.

## Bonding / connection groups (`SRTO_GROUP*`)

- **Status:** Not implemented.
- **Why deferred:** SRT 1.5 supports caller-side bonding for redundant
  uplinks; no shipping consumer uses it today. The bonding API surface
  is large, and a half-finished wrap is worse than no wrap at all.
- **Trigger to revisit:** A dual-radio uplink that wants redundancy at
  the SRT layer rather than at VPN / MPUDP.

## Key rotation (`SRTO_KMREFRESHRATE`, `SRTO_KMPREANNOUNCE`)

- **Status:** Not implemented; AES-CTR uses the static key derived
  from the passphrase for the lifetime of the connection.
- **Why deferred:** Shipping streams are typically minutes to hours;
  static-key AES-CTR is fine across that duration.
- **Trigger to revisit:** A 24/7 unattended stream, or a compliance
  regime that requires periodic rekey.

## Protocol-version pinning (`SRTO_PEERVERSION`, `SRTO_MINVERSION`)

- **Status:** Not exposed.
- **Why deferred:** libsrt 1.5.5 negotiates with anything 1.3 or
  newer. No current consumer needs to refuse older peers.
- **Trigger to revisit:** Integration with a peer that has a known
  protocol bug below some version.

## Typed packet-filter / FEC builder

- **Status:** Not implemented; spec strings pass through verbatim via
  `PacketFilter::new("fec,cols:N,rows:M,arq:onreq")`.
- **Why deferred:** The raw-string surface is small, well-documented
  by libsrt, and unlikely to grow. A typed builder roughly doubles the
  surface area for marginal benefit over the string form.
- **Trigger to revisit:** libsrt adds a filter type that's hard to
  compose by string.

## Stream-ID filtering on `Listener`

- **Status:** Not implemented; `Listener::accept` returns every
  successful handshake.
- **Why deferred:** Filter shape is application policy (regex? exact
  list? signed token?), not transport policy. The library exposes
  `socket.stream_id()` post-`accept` so the caller's accept loop
  decides whether to keep the connection.
- **Trigger to revisit:** A common abstraction emerges across multiple
  consumers.

## Custom congestion controller selection

- **Status:** `Live` (default) and `File` only.
- **Why deferred:** libsrt's `Live` is the right answer for live
  video. Plugging custom controllers via libsrt's C-callback
  registration is awkward research-tier work.
- **Trigger to revisit:** A research collaboration produces a
  controller that empirically beats `Live` for our workload.

## `klv::st0102` typed Security Local Set decoder

- **Status:** Pass-through only — ST 0601 Tag 48 surfaces as
  `Option<Vec<u8>>`.
- **Why deferred:** Adding the typed layer means duplicating the
  per-tag table for ST 0102 specifically. No current consumer reads
  ST 0102 in typed form; the substrate already supports the work when
  it lands.
- **Trigger to revisit:** A consumer needing typed access to specific
  ST 0102 fields (classification, releasable-to indicator, and so
  on).

## Other typed MISB sets (`klv::st0903` VMTI, `klv::st0806` RVT, ...)

- **Status:** Pass-through only. The substrate supports them; the
  typed layer is missing.
- **Why deferred:** No current consumer uses these sets. Adding any
  one set means writing and maintaining its per-tag table without a
  driving use case.
- **Trigger to revisit:** A consumer needing the typed layer for one
  of these sets specifically.

## KLV conformance cross-check vs. Python `klvdata`

- **Status:** Default test suite uses synthetic + MISB public fixtures
  as ground truth. No automated cross-decoder agreement check.
- **Why deferred:** Adding Python to CI is marginal value when MISB
  public test vectors are already authoritative ground truth.
- **Trigger to revisit:** A parsing bug ships that golden-file tests
  would have missed — i.e., the library and the spec disagreed without
  the test suite catching it.
- **Scope when added:** A `--features conformance` cargo feature plus
  a separate CI job that runs Python with `pip install klvdata`,
  parses each fixture with both decoders, and asserts typed-field
  agreement within tolerance.

## Streaming / chunked KLV decode

- **Status:** Buffer-in / buffer-out. The decoder consumes a complete
  KLV LS in one call.
- **Why deferred:** ST 0601 records are sub-1 KB typical and sub-10 KB
  worst case. A streaming decoder is implementation cost without a
  beneficiary.
- **Trigger to revisit:** Implausible — would require a consumer with
  records over 100 KB.

## `serde` integration for typed KLV records

- **Status:** Not implemented.
- **Why deferred:** Wire format and JSON aren't isomorphic — JSON
  would carry the typed shape but lose unknown-tag pass-through, which
  is the whole point of the ST 0107.5 future-proof skip rule.
- **Trigger to revisit:** A consumer wants typed records as JSON for
  external tooling, with an explicit decision on how unknown tags are
  represented.

## `no_std` support for `klv`

- **Status:** Requires `std`.
- **Why deferred:** Every shipping target has `std`. `no_std` means
  replacing `Vec` / `String` / `format!` with allocator equivalents —
  bounded but not free.
- **Trigger to revisit:** An embedded target with a hard `no_std`
  requirement.

## Multi-stream `mpegts::mux` — `srt-jni` / `srt-uniffi` binding surface

- **Status:** The `srt-c` C ABI fan-out shipped — `srtc_video_stream_handle_t` /
  `srtc_klv_stream_handle_t` typedefs, `srtc_mux_config_add_video_stream` /
  `_add_klv_stream` returning handles, and `_video_to(handle, ...)` /
  `_klv_to(handle, ...)` siblings on `srtc_muxer_t`, `srtc_mux_sender_t`,
  and `srtc_managed_mux_sender_t`. The single-target entry points keep
  their v0 signatures and surface `MuxError::AmbiguousTarget` as
  `SRTC_E_INVALID_USAGE` on multi-stream muxers. The same handle-aware
  shape has NOT yet landed in `srt-jni` or `srt-uniffi`.
- **Note on TsSender / RawSender:** the original deferred-features entry
  said `srtc_ts_sender_*` / `srtc_managed_ts_sender_*` would also gain
  `_video_to` / `_klv_to` siblings. That was wrong: `pipeline::TsSender`
  exposes only `send_ts(bytes)` (pre-muxed TS bytes) and `pipeline::RawSender`
  exposes only `send(bytes)`. Neither carries a `Muxer`, so handle-aware
  fan-out is meaningless on those variants. Only the three muxer-owning
  C variants (`srtc_muxer_t`, `srtc_mux_sender_t`, `srtc_managed_mux_sender_t`)
  have the new `_to` surface.
- **Trigger to revisit:** First JNI or UniFFI consumer that actually wants
  multi-stream output. The pattern is mechanical — mirror the same
  handle-typedef + `_to(handle, ...)` fan-out across the JNI/UniFFI
  binding once each ships.

## Typed SPS / VPS / PPS payload parser

- **Status:** Not implemented; SPS/VPS/PPS surface as ordinary
  `NalUnit::H264 { nal_type: 7, .. }` / `NalUnit::H265 { nal_type: 32 or 33, .. }`
  with raw RBSP payload. Consumers needing width / height / profile /
  level parse the RBSP themselves (e.g. with `h264-reader`,
  `h265-parser`).
- **Why deferred:** The `mpegts::demux` ship surfaces NAL boundaries +
  typed headers; consumers wanting frame dimensions reach for an
  in-tree codec library. Adding a dependency-free typed SPS parser
  duplicates existing well-trodden code.
- **Trigger to revisit:** A consumer asks for resolution / profile /
  level on `Sample::Video` without wanting to embed a codec library.

## `pipeline::pairing` — opt-in convenience pairing utility

- **Status:** Not implemented; consumers pair sync-KLV ↔ video AUs and
  sample-and-hold async-KLV themselves. Three cookbook recipes
  (`docs/cookbook.md` 12–14) cover the canonical patterns in ~20 lines
  each.
- **Why deferred:** Per the demux spec §4 (decoupled-pairing decision),
  pairing is a consumer-domain decision. A library-side helper would
  abstract over choices the library can't make correctly (tolerance
  windows, sample-and-hold semantics, multi-stream routing). The
  cookbook recipes are the recommended path until consumers ask for
  more.
- **Trigger to revisit:** Multiple consumers reimplement the same
  nearest-PTS pairing; converging strategies become a candidate for
  shared substrate.

## Multi-program TS in `mpegts::demux`

- **Status:** Single-program TS only (one PMT). `ProgramMap` carries
  `program_number` so multi-program lifts additively.
- **Why deferred:** No current consumer ships a multi-program TS.
- **Trigger to revisit:** A consumer needs to separate-and-route
  multiple programs from a single TS.

## AV1 / H.266 codec variants on `SamplePayload::Video`

- **Status:** `VideoCodec` covers H.264 + H.265. Other codecs surface
  as `SamplePayload::Unknown { stream_type, raw }`.
- **Why deferred:** No current consumer asks for AV1 / H.266 carriage.
  AV1 specifically is OBU-shaped (not NAL-shaped), so adding it
  requires either a separate `SamplePayload::Video` shape with
  `Vec<Obu>` instead of `Vec<NalUnit>`, or a cross-codec rework of the
  video payload type. Either is bigger than the current demuxer scope.
- **Trigger to revisit:** A consumer ships AV1 or H.266 in MPEG-TS.

## Rustdoc lift to docs.rs via `#![doc = include_str!(...)]`

- **Status:** Not implemented; the markdown guides under `docs/` are
  read directly from the repo.
- **Why deferred:** This crate is not yet on crates.io; until it
  publishes, docs.rs has nothing to auto-build. The markdown guides
  are written rustdoc-compatibly so the future lift is mechanical.
- **Trigger to revisit:** Immediately before publishing to crates.io.

## URL parameter coverage (Group 3 — recognized but unsupported)

- **Status:** Parser recognizes the libsrt URL key by name and rejects
  with `UrlError::UnsupportedKey` carrying its `SRTO_*` name. No
  silent failure; the operator gets a clear "this option exists but
  isn't yet exposed" message.
- **The list:** `bindtodevice` (`SRTO_BINDTODEVICE`),
  `cryptomode` (`SRTO_CRYPTOMODE`), `drifttracer`
  (`SRTO_DRIFTTRACER`), `enforcedencryption` (`SRTO_ENFORCEDENCRYPTION`),
  `groupconnect` (`SRTO_GROUPCONNECT`), `groupminstabletimeo`
  (`SRTO_GROUPMINSTABLETIMEO`), `iptos` (`SRTO_IPTOS`), `ipttl`
  (`SRTO_IPTTL`), `ipv6only` (`SRTO_IPV6ONLY`), `kmpreannounce`
  (`SRTO_KMPREANNOUNCE`), `kmrefreshrate` (`SRTO_KMREFRESHRATE`),
  `maxrexmitbw` (`SRTO_MAXREXMITBW`), `messageapi` (`SRTO_MESSAGEAPI`),
  `mininputbw` (`SRTO_MININPUTBW`), `minversion` (`SRTO_MINVERSION`),
  `nakreport` (`SRTO_NAKREPORT`), `peeridletimeo` (`SRTO_PEERIDLETIMEO`),
  `retransmitalgo` (`SRTO_RETRANSMITALGO`), `snddropdelay`
  (`SRTO_SNDDROPDELAY`), `transtype` (`SRTO_TRANSTYPE`), `tsbpdmode`
  (`SRTO_TSBPDMODE`).
- **Why deferred:** Each requires a new `SocketBuilder` setter on
  `srt-core` plus its typed wrapper / validation. Single-developer
  scope discipline — none of these has a current consumer ask.
- **Trigger to revisit:** A consumer asks for any specific key. Adding
  one is mechanical: add the builder setter + URL parser arm + remove
  it from this list and the parser's `GROUP3_REJECTED` table.

## URL parameter coverage — `rcvbuf` / `sndbuf` (units mismatch)

- **Status:** Listed in the URL parser as Group 3 (rejected). Separate
  entry from the rest because the blocker is units, not "no setter
  yet."
- **Why deferred:** libsrt's `SRTO_RCVBUF` / `SRTO_SNDBUF` are byte
  counts; this library's `recv_buf_packets` / `send_buf_packets`
  builder setters are packet counts. Exposing `?rcvbuf=1048576` would
  silently mean different things in libsrt-tools vs. our parser.
- **Note:** distinct from `udprcvbuf` / `udpsndbuf` (kernel UDP socket
  buffer sizes via `SRTO_UDP_RCVBUF` / `SRTO_UDP_SNDBUF`), which **are**
  exposed as URL keys (and as `recv_buffer_size` / `send_buffer_size`
  ffmpeg aliases). The unresolved unit-mismatch is for the
  SRT-internal packet queue setters.
- **Trigger to revisit:** Resolve the byte-vs-packets question on the
  builder side first (either rename to `_bytes`, add a `_bytes`
  variant, or document the unit conversion). Then the URL key can
  expose the chosen semantic without the foot-gun.

## URL-vs-builder conflict warning channel

- **Status:** Today the URL parser silently overrides builder values
  on conflict (per the documented "URL wins" rule). There's no
  channel to surface "FYI, your builder said X but the URL changed
  it to Y."
- **Why deferred:** No warning channel exists in the C ABI today —
  `srtc_get_last_error_str()` is for failures, not warnings. Adding a
  warning surface is its own design (separate buffer? log callback?
  per-thread storage like the error?). Out of scope for the URL
  parser ship.
- **Trigger to revisit:** A consumer reports a debugging session
  where they spent more than a few minutes wondering why their
  builder values didn't take effect; OR an unrelated request for a
  warning surface lands first.

## URL parser: additional test coverage

- **Status:** Three test categories not in the initial ship:
  1. Property-based roundtrip via `proptest` (random valid URLs
     roundtrip cleanly through parse and apply).
  2. Concurrent-open smoke (50–100 threads, no shared parser state).
  3. Atomicity-under-load 1000-iteration smoke (Q9-A invariant
     defended against future regression).
- **Why deferred:** The initial ship includes a fuzz target for
  panic-freedom and a one-shot atomicity test. Property testing needs
  a `proptest` dev-dependency; the parser's structural invariants (no
  shared mutable state, clone-then-mutate) make 2–3 redundant for
  initial coverage. They're additive regression-guards, not must-have
  for first ship.
- **Trigger to revisit:** First consumer-reported URL parser bug
  becomes a property test; concurrent-open returns when adding builder
  setters from Group 3 (more parser surface = more potential for
  shared state); atomicity-under-load gets re-considered if the Q9-A
  invariant gets touched (e.g. someone optimizes the clone away for
  performance).

## URL parser: strict percent-encoding validation

- **Status:** The parser inherits `url::Url::query_pairs()`'s lenient
  handling of malformed percent-encoding in queries. Sequences like
  `%2` or `%XY` pass through as literal substrings rather than
  rejecting with `UrlError::Syntax`. The fuzz target enforces
  panic-freedom; functional rejection of malformed sequences is not
  enforced.
- **Why deferred:** Strict rejection would mean either pre-validating
  the query string before handing to `url::Url::parse` or adding a
  manual percent-decode pass. Non-trivial work for a low-risk failure
  mode — the worst that happens is the typed validator on the per-key
  value rejects the literal `%2` substring (e.g. `StreamId::new("%2")`
  accepts ASCII, so even that escape hatch is partial).
- **Trigger to revisit:** A consumer reports a malformed URL silently
  parsing where they expected an error.

## `Listener::accept_timeout` — bounded blocking accept

- **Status:** Not implemented. `Listener::set_recv_timeout` writes
  `SRTO_RCVTIMEO` on the listener handle, but verification against
  libsrt 1.5.5 (`srtcore/api.cpp::CUDTUnited::accept` ~line 1299) shows
  the blocking accept calls `accept_sync.wait()` unconditionally —
  `SRTO_RCVTIMEO` does NOT drive `srt_accept` blocking in this libsrt
  version.
- **Why deferred:** Implementing accept-with-timeout requires either
  switching the listener to non-blocking (`SRTO_RCVSYN=0`) and using
  `srt_epoll_wait`, or otherwise integrating the existing async-deferred
  surface. Both are larger than this audit's scope.
- **Trigger to revisit:** A consumer needs a bounded accept (e.g. for
  graceful shutdown without a sentinel-thread workaround) AND has the
  context to motivate exposing `srt_epoll_*` from `srt-core`. Until
  then, the documented workaround is "run accept on a dedicated thread,
  call `Listener::close` from your shutdown path — that wakes the call
  with `AcceptError::ListenerClosed`."

## Errno-based error classification (`SrtErrno` minor codes)

- **Status:** Several `From<RawError> for *Error` impls match libsrt
  error message strings (`raw.message.contains("refused")`,
  `contains("in use")`, `contains("permission")`, `contains("closed")`,
  etc.) instead of the libsrt errno (`SRT_ENOSERVER`, `SRT_ECONNREJ`,
  `SRT_ELARGEMSG`, `SRT_EMSGSIZE`, etc.). The current `SrtErrno` enum
  collapses to major categories only.
- **Why deferred:** String-matching works against libsrt 1.5.5 today;
  the audit recommended deferring this refactor until either a libsrt
  upgrade breaks a string match or a user reports a misclassified
  error. Either trigger is well-defined and should reach the
  maintainer.
- **Trigger to revisit:** libsrt minor-version upgrade (1.5.x → 1.6.x)
  with classification regressions, OR a user-reported wrong-variant.

## `KeyLength` → `Option<KeyLength>` ergonomics

- **Status:** `SocketConfig::key_length` is `KeyLength` (default
  `Aes128`) and is unconditionally written to `SRTO_PBKEYLEN` whenever
  a passphrase is set. ffmpeg only sets `SRTO_PBKEYLEN` when the user
  explicitly passes `?pbkeylen=`, letting libsrt auto-negotiate.
- **Why deferred:** Negligible interop impact. AES-128 is the de-facto
  default everywhere; the only failure mode is a peer hardcoded to
  AES-256 rejecting our handshake.
- **Trigger to revisit:** A user reports an interop failure with an
  AES-256-only peer.

## `srt_cleanup()` shutdown hatch

- **Status:** Never called. `ensure_initialized()` runs once and libsrt
  stays initialized for the process lifetime. `init.rs` documents the
  rationale (drop-order ambiguity vs. negligible OS-reclaimed leaks).
- **Why deferred:** For long-running services this is correct. For
  short-lived CLIs and tests, valgrind / LeakSanitizer / Miri may
  report leaks; for dynamically-loadable modules unloaded by host
  processes, there's no escape hatch.
- **Trigger to revisit:** A consumer reports problems with libsrt
  init persisting beyond their module's lifetime (e.g. host plugin
  framework with hot-reload), OR LeakSanitizer integration becomes
  load-bearing in CI.

## Rust-API-only sender pipeline defaults

- **Status:** The 15 s `connect_timeout`, 5 s `linger`, and
  `Role::Sender` defaults applied for the audit's "live-streaming
  sensible defaults" set live in
  `crates/srt-c/src/connect.rs::connect_srt` (the canonical "default
  sender connect path" used by all six `srtc_*_open` calls).
  Pure-Rust users who construct a `SrtTransport` via `SocketBuilder`
  directly do NOT get these defaults — they get libsrt's defaults
  (3 s, 180 s, `Unspecified`).
- **Why deferred:** `pipeline::Sender` takes a pre-built `Transport`,
  so there's no SocketConfig construction point in the Rust pipeline
  layer to inject the defaults. Adding a
  `SocketConfig::sender_defaults()` helper or a
  `SocketBuilder::sender_preset()` shortcut is mechanical but adds API
  surface; the audit deferred this design choice to on-demand.
- **Trigger to revisit:** A pure-Rust consumer builds a sender pipeline
  and reports surprise at one of these libsrt defaults (e.g. their drop
  hangs 180 s, or their connect fails after 3 s on a radio link).

## URL parameter coverage — bigger Group 3 keys (audit Issue 6 Cat B/C)

- **Status:** The audit identified roughly 14 Group 3 keys that ffmpeg
  honors. This plan accepted only Category A (5 cheap aliases mapping
  to existing setters). Categories B (`rcvbuf` / `sndbuf` /
  `messageapi` / `nakreport` / `minversion`) and C
  (`enforcedencryption` / `kmrefreshrate` / `kmpreannounce` / `iptos` /
  `ipttl` / `snddropdelay` / `transtype` / `tsbpdmode`) remain
  deferred. Each Category C key needs a new `SocketConfig` field plus
  typed wrapper plus URL parser arm.
- **Why deferred:** Per the audit's recommendation: "duplicates work
  the project will eventually do anyway as deferred features get
  unblocked one by one — don't try to land them all in this audit
  fix."
- **Trigger to revisit:** Each individual key's existing trigger in
  the general "Group 3 unsupported keys" entry above; nothing
  additional.

## Reconnect counters on `ManagedTransport` stats

- **Status:** `SenderStats` / `ReceiverStats` aggregate transport-level
  byte / packet counters but not reconnect-cycle counters
  (`reconnect_attempts`, `reconnect_successes`, `last_reconnect_at`).
  Plain `Sender<SrtTransport>` has no such concept; only the managed
  variants do.
- **Why deferred:** Surfacing these on `SenderStats` / `ReceiverStats`
  forces optional fields that are always zero for plain (non-managed)
  handles, which muddies the C ABI shape. The cleaner home is a
  separate `ManagedTransportStats` accessor exposed on the
  `Sender<ManagedTransport<...>>` / `Receiver<ManagedReceiveTransport<...>>`
  variants only — but that means a second per-handle accessor, a
  second C struct, and decisions about how plain handles behave when
  callers ask (return zeros / return error). The first stats pass
  ships pipeline-level counters; reconnect telemetry slots in next.
- **Trigger to revisit:** A consumer running a managed-reconnect
  pipeline asks for visibility into how often the link is flapping
  (e.g. for alarm thresholds / backoff tuning).

## Last-activity-wall-clock gauges per stream

- **Status:** `StreamStats` carries item / byte counters but no
  `last_seen_at: Option<SystemTime>` gauge. Consumers that want
  "is this stream stalled?" detection have to compare deltas across
  successive `stats()` calls themselves.
- **Why deferred:** Adding a `SystemTime` field across what's
  otherwise plain integer counters cascades into the C ABI: the
  layout has to encode either epoch nanos in a `uint64_t` (overflows
  in 2554 — fine, but explicit) or a `(seconds, nanos)` split
  (extra two fields per `srtc_stream_stats_t`). Either way it's a
  bigger surface than the rest of the stats struct, and consumers
  who care can derive staleness from `items` deltas + their own
  wall-clock sampling cadence.
- **Trigger to revisit:** A consumer ships a watchdog that needs
  per-stream staleness without holding two snapshots, or asks for
  millisecond-resolution event timing in the stats surface.

## Per-stream PMT descriptor surface at the C ABI

- **Status:** The Rust core ships per-stream PMT descriptors via the
  `mpegts::descriptors` module and `ConfigBuilder::stream_descriptors_for_video` /
  `stream_descriptors_for_klv` / `stream_descriptors_for_stream` methods.
  The C ABI exposure is deferred.
- **Why deferred:** The descriptor-construction surface and the future
  receiver C ABI's per-stream descriptor surface should land together —
  exposing a send-only C ABI shape now would need reshaping when the
  receiver C ABI lands (which will also need `SrtcRawDescriptor` and
  read access to `StreamInfo::raw_descriptors`).
- **Trigger to revisit:** The receiver-surface design lands and pulls
  the descriptor surface into scope. At that point the C ABI gets
  descriptor builders mirroring `mpegts::descriptors` plus
  `srtc_mux_config_set_video_stream_descriptors` /
  `_set_klv_stream_descriptors` with bounded array params + a
  `SrtcRawDescriptor` `repr(C)` shape for the receive side's
  `StreamInfo::raw_descriptors`.

## `Socket::close` Result-type cleanup

- **Status:** `srt::Socket::close(self) -> Result<(), IoError>` always
  returns `Ok` after the 2026-05-03 cancellation refactor. The
  underlying `srt_close` return code is consumed inside the
  `CancelHandle` closer (which has a `Fn` signature, no return path).
  Same applies to `srt::Listener::close`.
- **Why deferred:** The signature is preserved for API stability —
  changing it now would be a breaking change for consumers who pattern
  on `if let Err(e) = sock.close()`. A future breaking-change cycle
  could either drop the `Result` entirely (close becomes infallible)
  or plumb `srt_close`'s rc back via a richer `CloseError` channel.
- **Trigger to revisit:** Next breaking-change cycle, OR a consumer
  reports needing the `srt_close` rc (e.g., to distinguish
  graceful-close from race-close errors).
