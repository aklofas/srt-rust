# Compatibility matrix

What `srt-rust` actually implements today, mapped to the upstream specs and
deployed protocol surfaces. Statuses are deliberate and conservative — items
not listed below are intentionally **not yet implemented**.

**Legend**

| Mark | Meaning |
| --- | --- |
| ✅ Full | Implemented and exercised by tests (synthetic + real-world fixtures). |
| ⚙️ Partial | Implemented for the curated subset listed; other parts not modelled. |
| 🔁 Pass-through | Not typed; preserved verbatim through the parser and re-emitted. |
| 🟡 Permissive | Wire format accepted; spec-mandated strictness is opt-in. |
| ⏳ Planned | On the roadmap, not yet implemented. |
| ❌ Out of scope | Deferred indefinitely. |

The `srt-rust` workspace deliberately scopes to **MPEG-TS + MISB ST 0601 KLV
over SRT**. Containers (MP4/CMAF/RIST/WebRTC), ST 0903 VMTI, raw elementary
streams, and so on are out of scope until a consumer asks. See
`crates/srt-core/tests/TEST_CORPUS.md` for the parsing-side compliance ledger
that this document summarises.

---

## Versions

| Component | Pinned at |
| --- | --- |
| Rust edition | 2024, MSRV **1.85** (`rust-toolchain.toml`) |
| `libsrt` (Haivision) | **v1.5.5** (`vendor/srt`, git submodule) |
| `mbedTLS` | **v3.6.6** LTS (`vendor/mbedtls`, git submodule) |
| `bindgen` | 0.72 (FFI; `srt-sys` build) |
| `cbindgen` | 0.29 (C header generation; `srt-c` build) |
| `cc` | 1.0 (compiles + links the C smoke test in `srt-c` integration tests) |
| `cmake` | upstream-compatible; `pkg-config` discovery first, vendored fallback |

The vendored builds disable libsrt's `ENABLE_HEAVY_LOGGING` (Debug+static
init crash), and force `USE_ENCLIB=mbedtls` when the `mbedtls` feature is
on (default).

---

## SRT transport (`srt-core::srt`)

### Wire protocol

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| `draft-sharabayko-srt` (IETF SRT v1.5) | ✅ Full | via libsrt 1.5.5; we don't reimplement the wire protocol. |
| `draft-sharabayko-srt-over-quic` | ❌ Out of scope | Upstream is exploratory. |
| Caller / Listener / Rendezvous handshake | ⚙️ Partial | Caller + Listener exposed; Rendezvous reachable via raw `srt-sys` only. |
| Live congestion controller (`SRTO_CONGESTION=live`) | ✅ Full | `Congestion::Live` (default). |
| File congestion controller (`SRTO_CONGESTION=file`) | ✅ Full | `Congestion::File`. |
| Message API (datagram-style send/recv) | ✅ Full | `Socket::send` / `Socket::recv`. |
| Stream API (`SRTO_TSBPDMODE=false`, `SRTO_MESSAGEAPI=false`) | ⏳ Planned | Reachable only via `srt-sys` today. |
| Bonded sockets / groups (`SRT_GROUP_*`) | ❌ Out of scope | No consumer demand. |
| Async / poll API (`srt_epoll_*`) | ⏳ Planned | Today's API is sync blocking. Reactor is on the roadmap. |

### Encryption (AES-CTR over SRT)

| Item | Status | Notes |
| --- | --- | --- |
| Pre-shared passphrase (10–79 chars) | ✅ Full | `Passphrase::new` enforces the spec range. |
| AES-128 key length | ✅ Full | `KeyLength::Aes128` (default). |
| AES-192 key length | ✅ Full | `KeyLength::Aes192`. |
| AES-256 key length | ✅ Full | `KeyLength::Aes256`. |
| mbedTLS backend | ✅ Full | Default; statically linked from `vendor/mbedtls`. |
| OpenSSL backend | ❌ Out of scope | Add only on consumer ask. |
| Built-in (libsrt's "internal" cryptolib) | ❌ Out of scope | Not used. |
| Encryption disabled (`--no-default-features`) | ✅ Full | Builds without mbedTLS; `ENABLE_ENCRYPTION=OFF`. |

### Tunables (`SocketConfig` / `ListenerConfig`)

Each row maps a libsrt option to its safe-Rust accessor. Accessors that
aren't yet wrapped are reachable via `srt-sys`.

| libsrt option | Safe field / setter | Status |
| --- | --- | --- |
| `SRTO_PASSPHRASE` | `passphrase: Option<Passphrase>` | ✅ Full |
| `SRTO_PBKEYLEN` | `key_length: KeyLength` | ✅ Full |
| `SRTO_LATENCY` | `latency: Option<Duration>` | ✅ Full |
| `SRTO_PEERLATENCY` | `peer_latency` (caller) | ✅ Full |
| `SRTO_RCVLATENCY` | `recv_latency` | ✅ Full |
| `SRTO_RCVBUF` (packets) | `recv_buf_packets` | ✅ Full |
| `SRTO_SNDBUF` (packets) | `send_buf_packets` (caller) | ✅ Full |
| `SRTO_MAXBW` | `max_bandwidth: MaxBandwidth` | ✅ Full |
| `SRTO_INPUTBW` | `input_bandwidth` (caller) | ✅ Full |
| `SRTO_OHEADBW` | `overhead_bandwidth_pct` | ✅ Full |
| `SRTO_MSS` | `mss: Option<u16>` | ✅ Full |
| `SRTO_PAYLOADSIZE` | `payload_size: Option<u16>` | ✅ Full |
| `SRTO_STREAMID` | `stream_id: Option<StreamId>` | ✅ Full |
| `SRTO_LOSSMAXTTL` | `loss_max_ttl` | ✅ Full |
| `SRTO_TLPKTDROP` | `too_late_packet_drop` | ✅ Full |
| `SRTO_FC` | `flow_window_packets` | ✅ Full |
| `SRTO_PACKETFILTER` | `packet_filter: Option<PacketFilter>` | ✅ Full |
| `SRTO_CONGESTION` | `congestion: Option<Congestion>` | ✅ Full |
| `SRTO_SNDTIMEO` / `SRTO_RCVTIMEO` | `send_timeout` / `recv_timeout` | ✅ Full |
| `SRTO_REUSEADDR` (listener) | `reuse_addr: bool` | ✅ Full |
| Backlog (`srt_listen` arg) | `backlog: u32` | ✅ Full |
| Per-socket statistics (`srt_bistats`) | `Socket::stats() -> Stats` | ⚙️ Partial — flat counter snapshot; finer windowing TBD. |
| All other `SRTO_*` options | — | 🔁 Pass-through via `srt-sys` (raw FFI). |

### Packet filters (`SRTO_PACKETFILTER`)

| Filter | Status | Notes |
| --- | --- | --- |
| `fec` (Reed–Solomon ARQ) | ✅ Full | Filter spec passed verbatim; libsrt evaluates it. |
| Custom-name pass-through | ✅ Full | Any value accepted by libsrt 1.5.5. |
| Filter parameter validation | 🟡 Permissive | Length ≤ 512 bytes + ASCII charset enforced; semantics handed to libsrt. |

---

## Container — MPEG-TS

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| MPEG-TS muxer | ✅ Full | `mpegts::mux::Muxer` — single-program, H.264/H.265 video + ST 0601 KLV (sync + async per ST 1402), VBR. |
| MPEG-TS demuxer | ❌ Out of scope | Receivers use FFmpeg / JavaCV / Bento4 / platform demuxers. |
| Single PES/packet KLV embedding (ST 1402.2 Asynchronous) | ✅ Full | Default in `mpegts::mux::Config` (`klv_stream_type = PrivateData`, `klv_carries_pts = false`). |
| ST 1402.3 Synchronous metadata stream | ✅ Full | `KlvStreamType::SynchronousMetadata` + `klv_carries_pts = true` in `Config`. |
| ST 1910 AU cell wrapping (sync KLV with timestamp) | ✅ Full | `klv::st1910::wrap_au_cell` / `unwrap_au_cell`. Compose with `Muxer::push_klv` when `klv_carries_pts = true`. |
| Variable-length PES splitting | ⏳ Planned | Required for ≥ 65 000-byte KLV records (rare in practice). |
| KLVA registration descriptor (`stream_type 0x06` + `0x05 "KLVA"`) | ✅ Full | Detected/recognised on decode side; emitted by muxer on the KLV PID. |

---

## KLV substrate (`srt-core::klv`)

The generic SMPTE/MISB substrate underneath the typed layers.

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| **SMPTE ST 336** — KLV encoding rules | ✅ Full | Universal Label, BER short/long, BER-OID, local-set / universal-set iterators. |
| **MISB ST 0107.5** — KLV Metadata in Motion Imagery | ✅ Full | Future-proof skip rule (`unknown: Vec<OwnedRawField>`); UL family-versioning helpers. |
| BER short-form length (≤ 127) | ✅ Full | `length::write_ber` / `read_ber`. |
| BER long-form length (1–8 octets) | ✅ Full | Indefinite-length form rejected (not allowed in KLV). |
| BER-OID length (base-128 self-terminating) | ✅ Full | `length::write_ber_oid` / `read_ber_oid`. |
| 16-bit running-sum checksum (ST 0601 §6.8) | ✅ Full | `checksum::checksum_running_sum_16`. |
| **MISB ST 1201.5** — IMAPB integer↔float mapping | ⚙️ Partial | `imapb` matches §7.1.2 / §7.2 forward and inverse; ±∞/±NaN special-value bit (§7.1.3) not modelled — pure additive future work. |
| Universal-label introspection | ✅ Full | `UniversalLabel::oid` / `category` / `registry` / `structure` / `version_byte` / `is_st0601_family`. |
| `Iter::local_set` / `Iter::universal_set` | ✅ Full | Universal-set iterator currently exposes `tag: 0` synthetic ID; UL bytes accessor is a future tightening. |
| `OwnedRawField` pass-through | ✅ Full | Unknown / non-typed tags preserved verbatim, round-trip safe. |

### Real-world recovery shapes

Recognised "shape" wrappers the parser handles when KLV PES payloads aren't
plain `[UL][len][body]`:

| Shape | Status | Notes |
| --- | --- | --- |
| Plain ST 0601 record | ✅ Full | `decode` / `decode_unchecked` / `decode_strict` / `decode_strict_compliance`. |
| Wrapped Precision Time Stamp Pack + ST 0601 LS (TRM 0909.4 §7) | ✅ Full | Phase A of the spec-compliance plan; first record decoded via `klv::st0605::decode`, rest via record-iter. |
| ST 1402.2 Synchronous Method 5-byte AU cell header | 🟡 Permissive | Recovery via SMPTE UL prefix (`06 0E 2B 34`) scan; principled AU cell parser is future work. |
| Broken-checksum captures | ✅ Full | `decode_unchecked` accepts; `decode` rejects with `ChecksumMismatch`. |

---

## KLV — typed MISB ST 0601 (`srt-core::klv::st0601`)

| Spec | Status | Notes |
| --- | --- | --- |
| **MISB ST 0601.19** — UAS Datalink Local Set | ⚙️ Partial | 49 of 143 items typed (see table below); remainder ride through `unknown` (ST 0107.5 future-proof skip). |
| ST 0601.19 §6.4 UAS LS UL | ✅ Full | `UniversalLabel::ST_0601_LS`; family check honours version-byte evolution. |
| ST 0601.8-09 (Tag 2 first) | ⚙️ Partial | Wire format accepts any order; `decode_strict_compliance` enforces. |
| ST 0601.8-11 (Tag 1 last) | ⚙️ Partial | Wire format accepts any order; `decode_strict_compliance` enforces. |
| ST 0601.8-12 (Tag 65 required) | ⚙️ Partial | Wire format treats as `Option<u8>`; `decode_strict_compliance` enforces. |
| ST 0601 ISO 646 / 7-bit ASCII string fields (Tags 3, 4, 10–12) | 🟡 Permissive | UTF-8 accepted; corpus is plain ASCII. |
| Linear-range int↔float mapping (§7.5) | ✅ Full | `INT_MIN` sentinel correctly rejected as `InvalidSentinel`. |
| Big-endian byte / bit ordering (§6.5.1) | ✅ Full | Throughout. |

### Typed ST 0601 items (49 of 143)

| ID | Name | Status |
| --- | --- | --- |
| 1 | Checksum | ✅ Full (verified, auto-emitted) |
| 2 | Precision Time Stamp | ✅ Full |
| 3 | Mission ID | ✅ Full |
| 4 | Platform Tail Number | ✅ Full |
| 5 | Platform Heading Angle | ✅ Full |
| 6 | Platform Pitch Angle | ✅ Full |
| 7 | Platform Roll Angle | ✅ Full |
| 8 | Platform True Airspeed | ✅ Full |
| 9 | Platform Indicated Airspeed | ✅ Full |
| 10 | Platform Designation | ✅ Full |
| 11 | Image Source Sensor | ✅ Full |
| 12 | Image Coordinate System | ✅ Full |
| 13 | Sensor Latitude | ✅ Full |
| 14 | Sensor Longitude | ✅ Full |
| 15 | Sensor True Altitude | ✅ Full |
| 16 | Sensor Horizontal FOV | ✅ Full |
| 17 | Sensor Vertical FOV | ✅ Full |
| 18 | Sensor Relative Azimuth | ✅ Full |
| 19 | Sensor Relative Elevation | ✅ Full |
| 20 | Sensor Relative Roll | ✅ Full |
| 21 | Slant Range | ✅ Full |
| 22 | Target Width | ✅ Full |
| 23 | Frame Center Latitude | ✅ Full |
| 24 | Frame Center Longitude | ✅ Full |
| 25 | Frame Center Elevation | ✅ Full |
| 26–33 | Offset Corner Lat/Lon Points 1–4 | ✅ Full |
| 47 | Generic Flag Data | ✅ Full |
| 48 | Security Local Set | 🔁 Pass-through (raw bytes; not parsed — see ST 0102 row) |
| 50 | Platform Call Sign | ✅ Full |
| 65 | UAS LS Version Number | ✅ Full (auto-emitted on encode if unset) |
| 75 | Sensor Ellipsoid Height | ✅ Full |
| 78 | Frame Center Height Above Ellipsoid | ✅ Full |
| 82–89 | Corner Lat/Lon Points 1–4 (Full) | ✅ Full |
| 90 | Platform Pitch Angle (Full) | ✅ Full |
| 91 | Platform Roll Angle (Full) | ✅ Full |
| 0–143 not listed above | other Items | 🔁 Pass-through via `unknown` (forwards-/backwards-compatible per ST 0107.5) |

Composite views layered on top: `GeoPoint`, `Attitude`, `FieldOfView`,
`Corners`. They surface `None` if any constituent typed field is missing.

### Decode entry points

| Entry point | Checksum | UL gating | Structural rules |
| --- | --- | --- | --- |
| `klv::st0601::decode` | ✅ verified | accepts any UL | ❌ |
| `klv::st0601::decode_unchecked` | ❌ skipped | accepts any UL | ❌ |
| `klv::st0601::decode_strict` | ✅ verified | requires ST 0601 family | ❌ |
| `klv::st0601::decode_strict_compliance` | ✅ verified | requires ST 0601 family | ✅ -09/-11/-12 |

---

## KLV — typed MISB ST 0605 (`srt-core::klv::st0605`)

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| **MISB ST 0605.10 §7** Precision Time Stamp Pack | ✅ Full | `klv::st0605::decode` / `encode`; 26-byte fixed layout. |
| **MISB ST 0807.27** registry row 1061 (UL CRC 23259) | ✅ Full | `UniversalLabel::PRECISION_TIMESTAMP_PACK_UL`. |
| **MISB ST 0603.5 §7.1** microsecond timestamp | ✅ Full | `timestamp_us: u64` (BE). |
| **MISB ST 0603.5 §7.4** Time Status byte | ✅ Full | `TimeStatus(u8)` newtype with `is_locked` / `has_discontinuity` / `is_reverse_jump` / `reserved_bits_valid` accessors. |
| Reserved-bits enforcement (4-0 = `0b11111`) | 🟡 Permissive | `decode` does not enforce; check via `time_status.reserved_bits_valid()`. The `KlvDecodeError::ReservedBitsInvalid` variant is present but not produced by `decode`. |

---

## Pipeline composition (`srt-core::pipeline`)

| Feature / Type | Status | Notes |
| --- | --- | --- |
| `Sender<T>` | ✅ Full | Composes `Muxer` + `Transport` for the canonical NAL+KLV → TS → SRT path. Internally synchronized; lossless across transient transport failures via in-flight buffer. |
| `TsSender<T>` | ✅ Full | Pre-muxed TS bytes → SRT with sync framing/recovery. 3-byte sync verify, 7-packet bundling, RECOVER + STRICT modes. |
| `RawSender<T>` | ✅ Full | Byte-blind one-shot sender. One `send` call = one SRT message; size-cap validation at construction. |
| `ManagedTransport<T>` | ✅ Full | Reconnect + gap-buffer decorator over any `Transport`. Synchronous reconnect on caller's thread; drop-oldest-message overflow policy; single-thread receiver. |
| Single-stream-shaped `Config` (Path 2 forward compat.) | ✅ Full | ≤1 video + ≤1 KLV stream today; multi-stream additive in future Path 3 without breaking changes. |

---

## C ABI (`srt-c`)

| Feature | Status | Notes |
| --- | --- | --- |
| `srtc_muxer_t` standalone utility | ✅ Full | open/push_video/push_klv/pull/close. Internally synchronized; data-path callable from multiple threads. |
| `srtc_mux_sender_t` (plain L1) | ✅ Full | NAL+KLV in, TS+SRT out via `SrtTransport`. Internally synchronized. |
| `srtc_managed_mux_sender_t` (managed L2) | ✅ Full | reconnect + gap buffer over `ManagedTransport<SrtTransport>`; synchronous retries on caller's thread. |
| `srtc_ts_sender_t` / `srtc_managed_ts_sender_t` | ✅ Full | pre-muxed TS bytes in; sync framing/recovery (RECOVER auto-resync + STRICT fail-fast); `_get_stats` accessor. |
| `srtc_raw_sender_t` / `srtc_managed_raw_sender_t` | ✅ Full | one `_send` call = one outbound SRT message. `SRTC_E_TOO_LARGE` on `len > SRTO_PAYLOADSIZE`. |
| Opaque builder configs | ✅ Full | `srtc_mux_config_t`, `srtc_ts_sender_config_t`, `srtc_raw_sender_config_t`, `srtc_reconnect_policy_t`. Internally cloned by `_open`; caller frees independently. |
| Thread-local last-error idiom | ✅ Full | `srtc_get_last_error()` + `srtc_get_last_error_str()`; ten `SRTC_E_*` codes covering all `srt_core` failure shapes. |
| `SRTC_VERSION_MAJOR` / `MINOR` / `PATCH` macros | ✅ Full | Compile-time `#define`s in `srtc.h`. |
| Lifecycle (`_open` / `_close`) | ✅ Full | `_open` returns NULL on failure with last-error set; `_close` is idempotent and NULL-safe; close-from-any-thread serializes through `Mutex<Option<...>>`. |
| URL parsing | ✅ Full | `srt://host:port?key=value&...` — IPv4 / DNS / bracketed IPv6 hosts plus the libsrt-URL Group 1 vocabulary (`streamid` / `passphrase` / `latency` / `payloadsize` / `congestion` / `conntimeo` / `linger` / `udprcvbuf` / `udpsndbuf` / etc.) plus a handful of ffmpeg-style aliases (`pkt_size`, `payload_size`, `srt_streamid`, `tsbpddelay`, `smoother`, `ffs`, `connect_timeout`, `recv_buffer_size`, `send_buffer_size`). See "FFmpeg URL interop quirks" below for unit divergence. |
| Stats accessors | ⚙️ Partial | `srtc_ts_sender_get_stats` / `srtc_managed_ts_sender_get_stats` shipped (mirror `pipeline::TsSenderStats`). Mux/raw stats await `Sender::stats()` / `RawSender::stats()` upstream. |
| `srtc_*_add_stream` (multi-stream Path 3) | ⏳ Planned | Today: `add_video` / `add_klv` only. Path 3 lifts the current single-stream cap additively without ABI break. |
| cbindgen-generated `srtc.h` | ✅ Full | Committed at `crates/srt-c/include/srtc.h`; CI verifies no drift via `tests/header_drift.rs`. |
| Symbol-prefix audit | ✅ Full | `tests/symbol_audit.rs` runs `nm -D` and asserts every exported symbol matches `^(srtc_|SRTC_|srt_)` (`srt_*` allowlisted because libsrt is statically linked). |
| `pkg-config` metadata | ✅ Full | `srtc.pc` generated by `build.rs` from `srtc.pc.in`; substitutes `@VERSION@` and `@PREFIX@`. |
| Static-link discipline | ✅ Full | libsrt + mbedTLS + libstdc++ statically embedded into `libsrtc.so` and `libsrtc.a`; `ldd` shows only libc / libpthread / libstdc++ / libdl / libm. |
| Distribution artifacts | ✅ Full | `libsrtc.so` + `libsrtc.a` + `srtc.h` + `srtc.pc`. Tarball staged manually; GitHub Releases publishing not automated today. |
| End-to-end C smoke test | ✅ Full | `tests/smoke.c` compiled by `cc` and linked against the cdylib at test time; exercises muxer push/pull + every NULL-close path + invalid-URL last-error. |
| Live-socket roundtrip test | ✅ Full | `tests/live_pair.rs` binds a real `Listener` on 127.0.0.1, connects `srtc_mux_sender_t`, sends a NAL, asserts the listener receives a TS sync byte. |
| Linux x86_64 build | ✅ Full | cdylib + staticlib + cbindgen header + pkg-config. |
| macOS / Windows / Linux aarch64 | ⏳ Planned | Cross-compilation follows demonstrated demand. |
| Pre-emptive close cancellation while parked in libsrt | ⏳ Planned | Close blocks until any in-flight send returns; tightening to libsrt's "close-anywhere unblocks the parked send" idiom is a follow-up. |

---

## FFmpeg URL interop quirks

`srt-rust` follows the libsrt-URL canonical conventions
(`srt-live-transmit`, OBS, mediamtx, gstreamer's `srtsink`/`srtsrc`,
Haivision Connect). FFmpeg's `srt://` protocol diverges in a few unit
conventions; users copying URLs between tools should be aware.

| URL key | FFmpeg unit | srt-rust unit | Notes |
| --- | --- | --- | --- |
| `latency` | µs | ms | srt-rust warns when value ≥ 10 s (likely paste from ffmpeg URL). |
| `rcvlatency` | µs | ms | Same warning. |
| `peerlatency` | µs | ms | Same warning. |
| `snddropdelay` | µs | (deferred) | Currently rejected as unsupported. |

When migrating an ffmpeg pipeline URL to srt-rust, divide the latency
values by 1000.

ffmpeg-style key aliases honored by srt-rust (zero new functionality —
just alternate spellings of existing keys): `pkt_size` / `payload_size`
(→ `payloadsize`), `srt_streamid` (→ `streamid`), `tsbpddelay` (→
`latency`), `smoother` (→ `congestion`), `ffs` (→ `fc`),
`recv_buffer_size` / `send_buffer_size` (→ `udprcvbuf` / `udpsndbuf`),
`connect_timeout` (→ `conntimeo`).

---

## Standards reference — what we cite vs. what we implement

The MISB / SMPTE / IETF documents that bear on `srt-rust`. The
"Implemented?" column reflects what `srt-rust` does, not what the spec
covers.

| Spec | Title | Implemented? |
| --- | --- | --- |
| **SMPTE ST 336** | Data Encoding Protocol Using Key-Length-Value | ✅ KLV substrate in `klv::pack` / `klv::length` / `klv::universal_label` |
| **MISB ST 0102.12** | Security Metadata Universal & Local Sets | 🔁 Pass-through (Tag 48 raw bytes); not typed |
| **MISB ST 0107.5** | KLV Metadata in Motion Imagery | ✅ Future-proof skip rule, UL family helpers |
| **MISB ST 0601.19** | UAS Datalink Local Set | ⚙️ 49 of 143 items typed (see table above) |
| **MISB ST 0603.5** | Time Stamping Motion Imagery | ✅ For ST 0605 Time Status byte |
| **MISB ST 0604.6** | Time Stamping & Transport in MISB Motion Imagery | ⏳ Planned (PCR / PTS in muxer) |
| **MISB ST 0605.10** | Encoding & Inserting Time Codes / Stamps | ✅ Precision Time Stamp Pack |
| **MISB ST 0607.5** | UAS Datalink LS Time-Stamped Records | 🔁 Pass-through; not exercised by corpus |
| **MISB ST 0805.1** | KLV Metadata over RTP | ❌ Out of scope (we transport over SRT/MPEG-TS) |
| **MISB ST 0807.27** | KLV Metadata Registry | ⚙️ Used as canonical source for UL constants |
| **MISB ST 0902.8** | Motion Imagery Sensor Minimum Metadata Set | ❌ Out of scope (subset of ST 0601 we already cover) |
| **MISB ST 0903.6** | Video Moving Target Indicator (VMTI) | ❌ Out of scope; add when a consumer needs VMTI |
| **MISB ST 1201.5** | IMAPB / IMAPA Floating-Point Mapping | ⚙️ §7.1.2 / §7.2 implemented; §7.1.3 special values not |
| **MISB ST 1303.2** | Multi-Dimensional Array Pack (MDAP) | ❌ Out of scope (no ST 0903 consumer) |
| **MISB ST 1402.2** | KLV in MPEG-2 Transport Streams | ✅ Async (0x06) + sync (0x15) modes in `mpegts::mux`; decode-side recovery via UL-prefix scan |
| **MISB ST 1607.2** | Constructs to Amend / Segment KLV | ❌ Out of scope (no multi-PES KLV in corpus) |
| **MISB ST 1910.1** | Inserting KLV in MPEG-TS for ISR | ✅ AU cell wrap/unwrap in `klv::st1910`; compose with `mpegts::mux` for full pipeline |
| **MISB TRM 0909.4** | Motion Imagery Quality Metadata | ⚙️ §7 multi-record PES pattern handled |
| **MISB RP 0802.2** | UAS Streaming Pipeline Recommendation | 📖 Reference reading |
| **MISB RP 1011.1** | Local Set Inheritance Recommendation | 📖 Reference reading |
| **MISP-2025.1** | Motion Imagery Standards Profile | 📖 Roadmap reference |
| **MISP-2025.1 Handbook** | MIS Handbook (companion to MISP) | 📖 Reference reading |
| **MISP-2023.2** | Motion Imagery Standards Profile (prior) | 📖 Reference reading |
| `draft-sharabayko-srt` (IETF) | Secure Reliable Transport Protocol | ✅ via `vendor/srt` |
| `draft-sharabayko-srt-over-quic` | SRT over QUIC | ❌ Out of scope |

📖 = read for context; nothing to implement directly.

---

## Bindings & FFI presentation

| Crate | Status | Target |
| --- | --- | --- |
| `srt-sys` | ✅ Full | Bindgen-generated FFI to libsrt 1.5.5; encryption via mbedTLS. |
| `srt-core` | ✅ Full (sync) | Safe Rust API — `Socket`, `Listener`, builders, KLV. |
| `srt-c` | ✅ Full | cdylib + staticlib + cbindgen header + pkg-config; Linux x86_64. |
| `srt-jni` | ⏳ Planned | JVM JAR for JDK 17+ consumers. |
| `srt-uniffi` | ⏳ Planned | iOS / Android via UniFFI (Swift / Kotlin). |

---

## Platforms

| Platform | Status | CI? |
| --- | --- | --- |
| linux-x86_64 (Ubuntu 22.04+, glibc) | ✅ Full | ✅ `.github/workflows/ci.yml` (vendored libsrt) |
| linux-aarch64 | ⏳ Planned | not yet in CI |
| macOS (x86_64 / aarch64) | ⏳ Planned | not yet in CI |
| Windows (MSVC) | ⏳ Planned | not yet in CI |
| iOS / Android | ⏳ Planned | gated on `srt-uniffi` |

---

## Out of scope

These items appear in nearby specs but are explicitly **not** on the
roadmap. They are revisitable on consumer ask — not philosophical refusals.

- Containers other than MPEG-TS (MP4 / fMP4 / CMAF, Matroska / WebM, RTP).
- Metadata sets other than ST 0601 + ST 0605 (ST 0102 typed view, ST 0903 VMTI, ST 1303 MDAP, ST 0902 minimum-set).
- RTP / WebRTC / RTMP / RIST transports.
- ST 1607 segmented multi-PES KLV reassembly.
- ST 1201.5 §7.1.3 special-value bit (±∞ / ±NaN passthrough).
- Async / reactor SRT API.
- Bonded / grouped sockets.
