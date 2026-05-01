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
| ❌ Out of scope | Deferred indefinitely (see `docs/deferred-features.md` in the parent workspace). |

The `srt-rust` workspace deliberately scopes to **MPEG-TS + MISB ST 0601 KLV
over SRT** for v0. Containers (MP4/CMAF/RIST/WebRTC), ST 0903 VMTI, raw
elementary streams, and so on are out of scope until a consumer asks. See
`crates/srt-core/tests/TEST_CORPUS.md` for the parsing-side compliance ledger
that this document summarises.

---

## Versions

| Component | Pinned at |
| --- | --- |
| Rust edition | 2024, MSRV **1.85** (`rust-toolchain.toml`) |
| `libsrt` (Haivision) | **v1.5.5** (`vendor/srt`, git submodule) |
| `mbedTLS` | **v3.6.6** LTS (`vendor/mbedtls`, git submodule) |
| `bindgen` | 0.72 (FFI) |
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
| Async / poll API (`srt_epoll_*`) | ⏳ Planned | v0 is sync blocking. Reactor is on the roadmap. |

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
| MPEG-TS muxer | ⏳ Planned | `mpegts::mux` is the next major workstream after KLV. |
| MPEG-TS demuxer | ❌ Out of scope (v0) | Receivers use FFmpeg / JavaCV / Bento4 / platform demuxers. |
| Single PES/packet KLV embedding (ST 1402.2 Asynchronous) | ⏳ Planned | Wire-format target; muxer not started. |
| Synchronous Metadata Multiplex Method (ST 1402.2 §9.4) | ⏳ Planned | Decode path: 5-byte AU cell header is currently recovered via UL-prefix scan. |
| Variable-length PES splitting | ⏳ Planned | Required for ≥ 65,000-byte KLV records (rare in practice). |
| KLVA registration descriptor (`stream_type 0x06` + `0x05 "KLVA"`) | ✅ Full | Detected/recognised on decode side; emitted by muxer when it lands. |

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
| `Iter::local_set` / `Iter::universal_set` | ✅ Full | Universal-set iterator currently exposes `tag: 0` synthetic ID; UL bytes accessor is a future tightening (see `docs/deferred-features.md`). |
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

## Standards reference — what we cite vs. what we implement

The MISB / SMPTE / IETF documents we read while building `srt-rust`. The
"Implemented?" column reflects what `srt-rust` does, not what the spec
covers.

| Spec | Title | Local copy | Implemented? |
| --- | --- | --- | --- |
| **SMPTE ST 336** | Data Encoding Protocol Using Key-Length-Value | (referenced via MISB) | ✅ KLV substrate in `klv::pack` / `klv::length` / `klv::universal_label` |
| **MISB ST 0102.12** | Security Metadata Universal & Local Sets | `reference/ST0102.12.pdf` | 🔁 Pass-through (Tag 48 raw bytes); not typed |
| **MISB ST 0107.5** | KLV Metadata in Motion Imagery | `reference/ST0107.5.pdf` | ✅ Future-proof skip rule, UL family helpers |
| **MISB ST 0601.19** | UAS Datalink Local Set | `reference/ST0601.19.pdf` | ⚙️ 49 of 143 items typed (see table above) |
| **MISB ST 0603.5** | Time Stamping Motion Imagery | `reference/ST0603.5.pdf` | ✅ For ST 0605 Time Status byte |
| **MISB ST 0604.6** | Time Stamping & Transport in MISB Motion Imagery | `reference/ST0604.6.pdf` | ⏳ Planned (PCR / PTS in muxer) |
| **MISB ST 0605.10** | Encoding & Inserting Time Codes / Stamps | `reference/ST0605.10.pdf` | ✅ Precision Time Stamp Pack |
| **MISB ST 0607.5** | UAS Datalink LS Time-Stamped Records | `reference/ST0607.5.pdf` | 🔁 Pass-through; not exercised by corpus |
| **MISB ST 0805.1** | KLV Metadata over RTP | `reference/ST0805.1.docx` | ❌ Out of scope (we transport over SRT/MPEG-TS) |
| **MISB ST 0807.27** | KLV Metadata Registry | `reference/ST0807.27.xls` | ⚙️ Used as canonical source for UL constants |
| **MISB ST 0902.8** | Motion Imagery Sensor Minimum Metadata Set | `reference/ST0902.8.pdf` | ❌ Out of scope (subset of ST 0601 we already cover) |
| **MISB ST 0903.6** | Video Moving Target Indicator (VMTI) | `reference/ST0903.6.pdf` | ❌ Out of scope (v0); add when a consumer needs VMTI |
| **MISB ST 1201.5** | IMAPB / IMAPA Floating-Point Mapping | `reference/ST1201.5.pdf` | ⚙️ §7.1.2 / §7.2 implemented; §7.1.3 special values not |
| **MISB ST 1303.2** | Multi-Dimensional Array Pack (MDAP) | `reference/ST1303.2.pdf` | ❌ Out of scope (no ST 0903 consumer) |
| **MISB ST 1402.2** | KLV in MPEG-2 Transport Streams | `reference/ST1402.2.pdf` | ⏳ Decode-side recovery in place (UL-prefix scan); muxer planned |
| **MISB ST 1607.2** | Constructs to Amend / Segment KLV | `reference/ST1607.2.pdf` | ❌ Out of scope (no multi-PES KLV in corpus) |
| **MISB ST 1910.1** | Inserting KLV in MPEG-TS for ISR | `reference/ST1910.1.pdf` | ⏳ Planned (muxer target topology) |
| **MISB TRM 0909.4** | Motion Imagery Quality Metadata | `reference/TRM0909.4.pdf` | ⚙️ §7 multi-record PES pattern handled |
| **MISB RP 0802.2** | UAS Streaming Pipeline Recommendation | `reference/RP0802.2.pdf` | 📖 Reference reading |
| **MISB RP 1011.1** | Local Set Inheritance Recommendation | `reference/RP1011.1.pdf` | 📖 Reference reading |
| **MISP-2025.1** | Motion Imagery Standards Profile | `reference/MISP-2025.1.pdf` | 📖 Roadmap reference |
| **MISP-2025.1 Handbook** | MIS Handbook (companion to MISP) | `reference/MISP-2025.1_Motion_Imagery_Handbook.pdf` | 📖 Reference reading |
| **MISP-2023.2** | Motion Imagery Standards Profile (prior) | `reference/MISP-2023.2.pdf` | 📖 Reference reading |
| `draft-sharabayko-srt` (IETF) | Secure Reliable Transport Protocol | `haivision/srt-rfc/draft-sharabayko-srt.md` | ✅ via `vendor/srt` |
| `draft-sharabayko-srt-over-quic` | SRT over QUIC | `haivision/srt-rfc/` | ❌ Out of scope |

📖 = read for context; nothing to implement directly.

---

## Bindings & FFI presentation

| Crate | Status | Target |
| --- | --- | --- |
| `srt-sys` | ✅ Full | Bindgen-generated FFI to libsrt 1.5.5; encryption via mbedTLS. |
| `srt-core` | ✅ Full (sync) | Safe Rust API — `Socket`, `Listener`, builders, KLV. |
| `srt-c` | ⏳ Planned | cdylib + cbindgen header for embedded Linux / Panama / FFM. |
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

## Out of scope (v0)

These items appear in nearby specs but are explicitly **not** on the v0
roadmap. They are revisitable on consumer ask — not philosophical refusals.

- Containers other than MPEG-TS (MP4 / fMP4 / CMAF, Matroska / WebM, RTP).
- Metadata sets other than ST 0601 + ST 0605 (ST 0102 typed view, ST 0903 VMTI, ST 1303 MDAP, ST 0902 minimum-set).
- RTP / WebRTC / RTMP / RIST transports.
- ST 1607 segmented multi-PES KLV reassembly.
- ST 1201.5 §7.1.3 special-value bit (±∞ / ±NaN passthrough).
- Async / reactor SRT API.
- Bonded / grouped sockets.

See `docs/deferred-features.md` in the parent workspace for the full
rationale ledger.
