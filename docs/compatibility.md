# Compatibility matrix

What `ts-transformer` actually implements today, mapped to the upstream specs and
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

The `ts-transformer` workspace deliberately scopes to **MPEG-TS + MISB ST 0601 KLV
over SRT**. Containers (MP4/CMAF/RIST/WebRTC), ST 0903 VMTI, raw elementary
streams, and so on are out of scope until a consumer asks. See
`crates/tst-core/tests/TEST_CORPUS.md` for the parsing-side compliance ledger
that this document summarises.

---

## Versions

| Component | Pinned at |
| --- | --- |
| Rust edition | 2024, MSRV **1.85** (`rust-toolchain.toml`) |
| `libsrt` (Haivision) | **v1.5.5** (`vendor/srt`, git submodule) |
| `mbedTLS` | **v3.6.6** LTS (`vendor/mbedtls`, git submodule) |
| `bindgen` | 0.72 (FFI; `srt-sys` build) |
| `cbindgen` | 0.29 (C header generation; `tst-c` build) |
| `cc` | 1.0 (compiles + links the C smoke test in `tst-c` integration tests) |
| `cmake` | upstream-compatible; `pkg-config` discovery first, vendored fallback |

The vendored builds disable libsrt's `ENABLE_HEAVY_LOGGING` (Debug+static
init crash), and force `USE_ENCLIB=mbedtls` when the `mbedtls` feature is
on (default).

---

## SRT transport (`tst-srt`)

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
| MPEG-TS muxer | ✅ Full | `mpegts::mux::Muxer` — multi-program (≤16 programs), multi-stream (≤16 video + ≤16 audio + ≤16 KLV + ≤16 subtitle PIDs per program), H.264/H.265/H.266/AV1 video + MP2/AAC/AC-3 audio + DVB/teletext/CEA-708/WebVTT subtitles + ST 0601 KLV (sync + async per ST 1402), VBR. |
| MPEG-TS demuxer | ✅ Full | `mpegts::demux::Demuxer` — multi-program, lenient by default, four-tier `StrictMode` ladder. See `mpegts::demux` block below. |
| Single PES/packet KLV embedding (ST 1402.2 Asynchronous) | ✅ Full | Default in `mpegts::mux::Config` (`klv_stream_type = PrivateData`, `klv_carries_pts = false`). |
| ST 1402.3 Synchronous metadata stream | ✅ Full | `KlvStreamType::SynchronousMetadata` + `klv_carries_pts = true` in `Config`. |
| H.222.0 § 2.12.4.2 sync metadata AU cell wrapping | ✅ Full | `mpegts::au_cell::write_metadata_au_cell` / `read_metadata_au_cell`. Auto-wrapped inside `Muxer::push_klv` for `KlvStreamType::SynchronousMetadata` streams; PTS in PES header per § 2.12.4.1. |
| Variable-length PES splitting | ⏳ Planned | Required for ≥ 65 000-byte KLV records (rare in practice). |
| KLVA registration descriptor (`stream_type 0x06` + `0x05 "KLVA"`) | ✅ Full | Detected/recognised on decode side; emitted by muxer on the KLV PID. |
| Audio carriage (mux side) | ✅ Full | MP2 / AAC ADTS / AAC LATM / AC-3; `Muxer::push_audio` / `push_audio_to`; PTS-only PES headers. AC-3 streams auto-emit `registration_descriptor` `format_identifier="AC-3"` per ATSC A/53 Part 3 §5.1 (caller-Registration suppression). Optional `iso_639_language_descriptor` auto-emit via `ConfigBuilder::add_audio_with_language(pid, codec, lang)` per ISO/IEC 13818-1 §2.6.18 (caller-tag-0x0A suppression). |
| Audio carriage (demux side) | ✅ Full | Typed `AudioCodec`; raw PES bytes in `SamplePayload::Audio { frames }`. |

---

## Subtitle / caption carriage

Four subtitle / caption codecs ride alongside video / KLV / audio.
All four share PMT `stream_type = 0x06` and disambiguate via
auto-emitted PMT descriptors (`subtitling_descriptor`,
`teletext_descriptor`, or `registration_descriptor` with
`format_identifier` `"VTTC"` / `"GA94"`).

| Codec | Sender | Receiver | Verification |
|---|---|---|---|
| DVB subtitling | ✅ Full | ✅ Full | round-trip + ffprobe |
| DVB teletext | ✅ Full | ✅ Full | round-trip + ffprobe |
| CEA-708 standalone | ✅ Best-effort | ✅ Full | round-trip-only (ffmpeg's CEA-708 path is SEI-embedded) |
| WebVTT-in-TS | ✅ Full | ✅ Full | round-trip + ffprobe |
| CEA-608/708 in SEI | ❌ — | ❌ — | future SEI parsing plan |
| ARIB STD-B24 | ❌ — | ❌ — | deferred (Japan-only) |

Per-program cap: ≤16 subtitle streams (`MAX_SUBTITLE_STREAMS_PER_PROGRAM`).
Subtitle PIDs cannot serve as the PCR PID (too sparse for PCR pacing).
`push_subtitle` payload max: 65527 bytes (PES packet length budget).

### Wire-envelope and descriptor conformance

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| DVB-sub PES envelope (ETSI EN 300 743 §6.2) | ✅ Full | Auto-wrapped inside `Muxer::push_subtitle_to` for `DvbSubtitling` (`data_identifier=0x20` + `subtitle_stream_id=0x00` + segments + `end_of_PES_data_field_marker=0xFF`). Caller passes raw segment bytes. |
| DVB-teletext stuffed PES header (ETSI EN 300 472 §4.2) | ✅ Full | Auto-emitted inside `Muxer::push_subtitle_to` for `DvbTeletext` (45-byte header, `PES_header_data_length=0x24`); PES tail padded with `0xFF` stuffing to `(N × 184) − 6` `PES_packet_length`. |
| Multi-language single-PID `subtitling_descriptor` (ETSI EN 300 468 §6.2.41) | ✅ Full | `mpegts::descriptors::subtitling_descriptor_multi`. |
| Multi-language single-PID `teletext_descriptor` (ETSI EN 300 468 §6.2.43) | ✅ Full | `mpegts::descriptors::teletext_descriptor_multi`. |
| Subtitle auto-emit suppression on caller-supplied descriptor | ✅ Full | When `ConfigBuilder::stream_descriptors_for_subtitle` supplies a recognized codec marker (`subtitling_descriptor` 0x59 / `teletext_descriptor` 0x56 / VBI teletext 0x46 / `registration_descriptor` with `VTTC` or `GA94` format_identifier), the muxer suppresses its codec-driven auto-emit (mirrors KLVA / AV01 suppression). |
| ISO 639 language code casing (EN 300 468) | ✅ Full | `validate_language_code` accepts both lowercase and uppercase 3-letter ASCII alphabetic codes. |
| Subtitle-only program rejection | ✅ Full | `Config::validate` rejects programs with subtitle streams but no video/audio/KLV (`MuxError::SubtitleOnlyProgram`); subtitle PIDs are too sparse to anchor PCR. |
| Multi-descriptor `stream_type 0x06` ambiguity (demux) | ✅ Full | `NonConformantIssue::SubtitleDescriptorAmbiguous` surfaced when ≥2 distinguishing descriptors co-exist (subtitling / teletext / `VTTC` / `GA94`); cascade picks first match. |

---

## KLV substrate (`tst-core::klv`)

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
| ST 1402.2 Synchronous Method 5-byte AU cell header | ✅ Full | Spec-conformant 5-byte parser ships in `mpegts::au_cell::read_metadata_au_cell` per H.222.0 V9 § 2.12.4.2 Tables 2-155+2-156. |
| Broken-checksum captures | ✅ Full | `decode_unchecked` accepts; `decode` rejects with `ChecksumMismatch`. |

---

## KLV — typed MISB ST 0601 (`tst-core::klv::st0601`)

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

## KLV — typed MISB ST 0605 (`tst-core::klv::st0605`)

| Spec / Feature | Status | Notes |
| --- | --- | --- |
| **MISB ST 0605.10 §7** Precision Time Stamp Pack | ✅ Full | `klv::st0605::decode` / `encode`; 26-byte fixed layout. |
| **MISB ST 0807.27** registry row 1061 (UL CRC 23259) | ✅ Full | `UniversalLabel::PRECISION_TIMESTAMP_PACK_UL`. |
| **MISB ST 0603.5 §7.1** microsecond timestamp | ✅ Full | `timestamp_us: u64` (BE). |
| **MISB ST 0603.5 §7.4** Time Status byte | ✅ Full | `TimeStatus(u8)` newtype with `is_locked` / `has_discontinuity` / `is_reverse_jump` / `reserved_bits_valid` accessors. |
| Reserved-bits enforcement (4-0 = `0b11111`) | 🟡 Permissive | `decode` does not enforce; check via `time_status.reserved_bits_valid()`. The `KlvDecodeError::ReservedBitsInvalid` variant is present but not produced by `decode`. |

---

## MPEG-TS demuxer (`tst-core::mpegts::demux`)

| Feature / Type | Status | Notes |
| --- | --- | --- |
| `Demuxer` | ✅ Full | Stateful TS demuxer; `feed` bytes in, `next_event` typed events out, `flush` drains trailing PES on stream end. Bytes need not be 188-aligned. |
| `DemuxerBuilder` / `DemuxerOptions` | ✅ Full | Fluent builder + plain-struct config form. |
| `DemuxEvent::ProgramMap` | ✅ Full | Emitted on PAT/PMT discovery and version-bump; carries `program_number`, `pcr_pid`, `streams`, `klv_links`. |
| `DemuxEvent::Sample` | ✅ Full | Generic ES sample; payload typed for video / audio / subtitle, `Unknown` for unrecognized stream_types. |
| `DemuxEvent::Metadata` | ✅ Full | Standalone metadata events; `MetadataKind::KlvSyncAuCell { metadata_service_id, sequence_number, cell_fragment_indication, decoder_config_flag, random_access_indicator }` (5 fields per H.222.0 § 2.12.4.2 Table 2-156), `KlvAsync` (bare LS), `Unknown(u8)`. |
| `DemuxEvent::Discontinuity` | ✅ Full | `ContinuityJump`, `PesOversize`, `PesTotalOversize`, `AdaptationFieldFlag`. |
| `DemuxEvent::NonConformant` | ✅ Full | Lenient-mode signal for spec violations; converts to fatal in strict modes. |
| H.264 NAL split (stream_type 0x1B) | ✅ Full | Annex-B start codes stripped; `NalUnit::H264 { nal_type, ref_idc, payload }`. Emulation-prevention bytes preserved. |
| H.265 NAL split (stream_type 0x24) | ✅ Full | `NalUnit::H265 { nal_type, layer_id, temporal_id_plus1, payload }`. |
| H.266 NAL split (stream_type 0x33) | ✅ Full | `NalUnit::H266 { nal_type, layer_id, temporal_id_plus1, payload }`. |
| AV1 OBU split (stream_type 0x06 + AV01 registration) | ✅ Full | LEB128 `obu_size` consumed; `Obu { obu_type, extension, payload }`. Disambiguates from KLV-async via `format_identifier`. |
| Async KLV (stream_type 0x06 + KLVA) | ✅ Full | Detected via registration descriptor `KLVA`; emitted as `KlvAsync`. |
| Sync KLV (stream_type 0x15) | ✅ Full | H.222.0 § 2.12.4.2 5-byte `Metadata_AU_cell` header parsed; PES PTS surfaced on the parent event (per § 2.12.4.1). |
| Stream type / shape mismatch (sync↔async fallback) | ✅ Full | When PMT and wire shape disagree (e.g. 0x15 PID with bare KLV), demuxer classifies on actual shape and emits `StreamTypeMismatch{Sync,Async}On*Pid` non-conformance. PES PTS preserved. |
| `metadata_descriptor` parser | ✅ Full | KLV→video link emitted in `ProgramMap.klv_links` as `LinkSource::Declared`. |
| KLV link inference (single video + single KLV) | ✅ Full | `LinkSource::Inferred` when no descriptor but topology is unambiguous. |
| `DemuxerBuilder::link_klv` override | ✅ Full | Caller-supplied klv→video PID link (`LinkSource::Override`). |
| `DemuxerBuilder::treat_as` override | ✅ Full | Caller-supplied `StreamKind` override per PID — overrides the PMT-derived kind. |
| PES reassembly cap (`pes_cap_per_pid`) | ✅ Full | Default 4 MiB per-PID; overflow surfaces as `Discontinuity::PesOversize`. |
| PES reassembly cap (`pes_cap_total`) | ✅ Full | Default 64 MiB aggregate; overflow surfaces as `Discontinuity::PesTotalOversize`. |
| Sync recovery (HUNT/VERIFY/LOCKED) | ✅ Full | Internal syncer state machine; ~6 KiB search window, then `DemuxError::Unrecoverable`. |
| `StrictMode::Off` (lenient default) | ✅ Full | `NonConformant` events surface as data; receive loop continues. |
| `StrictMode::TimingOnly` | ✅ Full | Hard-fail on `PcrAnomaly`, `PusiMidPes`, `PsiChecksumMismatch`. |
| `StrictMode::DescriptorsOnly` | ✅ Full | Hard-fail on `MissingMetadataDescriptor`, `StreamTypeMismatch{Sync,Async}OnPid`. |
| `StrictMode::Full` | ✅ Full | Hard-fail on every `NonConformantIssue` variant including future-added ones. |
| KLV-mismatch event coalescing | ✅ Full | One `StreamTypeMismatch*` event per (PID, PMT version); avoids flooding. |
| PSI version-bump detection | ✅ Full | Re-emits `ProgramMap` only on PMT/PAT version change. |
| `pts_to_duration` helper | ✅ Full | 90 kHz ticks → `std::time::Duration`. |
| Multi-program TS | ✅ Full | Multi-PMT; one `ProgramMap` event per program + on PAT/PMT version bumps; `StreamInfo.program_number` on every `Sample`/`Metadata` event; PAT version diffing drops disappeared programs; `NonConformantIssue::PidReusedAcrossPrograms` on cross-program PID collision. |
| Subtitle classification on `stream_type 0x06` | ✅ Full | Cascade: subtitling/teletext/`VTTC`/`GA94` descriptors → `Subtitle` payload; KLV cases unchanged when no subtitle descriptor present. |
| AV1 / H.266 codec variants on `VideoCodec` | ✅ Full | `H266` (`stream_type=0x33`) emits `VideoPayload::Nals(_)`; `Av1` (`stream_type=0x06` + AV01 registration) emits `VideoPayload::Obus(_)`. |
| Typed SPS/VPS/PPS payload parser | ✅ Full | `codec::h264` / `codec::h265` / `codec::h266` for NAL-shaped codecs; `codec::av1` for OBU-shaped. See `codec` block below. |
| Sync-KLV ↔ video AU pairing helper | ❌ Out of scope | Pairing is a consumer-domain decision; cookbook recipes 12–14 are the canonical patterns. |

## Pipeline composition (`tst-pipeline`)

### Send side

| Feature / Type | Status | Notes |
| --- | --- | --- |
| `MuxSender<T>` | ✅ Full | Composes `Muxer` + `Transport` for the canonical NAL+KLV → TS → SRT path. Internally synchronized; lossless across transient transport failures via in-flight buffer. |
| `Sender<T>` | ✅ Full | Pre-muxed TS bytes → SRT with sync framing/recovery. 3-byte sync verify, 7-packet bundling, RECOVER + STRICT modes. |
| `RawSender<T>` | ✅ Full | Byte-blind one-shot sender. One `send` call = one SRT message; size-cap validation at construction. |
| `ManagedTransport<T>` | ✅ Full | Reconnect + gap-buffer decorator over any `Transport`. Synchronous reconnect on caller's thread; drop-oldest-message overflow policy; single-thread receiver. |
| Multi-stream / multi-program `Config` | ✅ Full | ≤16 programs, ≤16 video + ≤16 KLV streams per program; nested `add_program(N, pmt_pid).add_video(...).end_program()` builder; opaque handles from `video_handles_for_program(N)` / `klv_handles_for_program(N)`; `push_video_to(handle, …)` / `push_klv_to(handle, …)` on `Muxer` and `MuxSender`. Single-program single-stream callers keep the old flat API unchanged. |

### Receive side

| Feature / Type | Status | Notes |
| --- | --- | --- |
| `DemuxReceiver<R>` | ✅ Full | `RecvTransport → Receiver → Demuxer` shell. `recv_event` → `DemuxEvent`; auto-flushes demuxer on `Closed`. Implements `Iterator<Item = Result<DemuxEvent, DemuxReceiverError>>`. |
| `Receiver<R>` | ✅ Full | Pull bytes from a `RecvTransport`, run TS sync recovery, emit 188-byte aligned packets via `next_packet`. |
| `RawReceiver<R>` | ✅ Full | One `recv_one` call returns one owned byte vec — no TS framing or demux. |
| `ManagedReceiveTransport<R>` | ✅ Full | Reconnect decorator for the receive direction. No gap buffer (recv-side bytes that never arrived can't be replayed); restarts on `Closed` / `Broken` per `ReconnectPolicy`. |
| `RecvTransport` trait | ✅ Full | Receive-side counterpart to `Transport`: `recv_bytes`, `max_payload`, `is_alive`, `close`. Implemented by `SrtTransport` and any consumer-side mock. |
| `SrtTransport` impl `RecvTransport` | ✅ Full | Same `SrtTransport` wrapper handles both send and receive directions on a connected SRT `Socket`. |
| `DemuxReceiver::add_byte_sink` fan-out | ✅ Full | Register `Box<dyn FnMut(&[u8]) + Send>` callbacks; each sink sees every 188-byte TS packet in registration order before the demuxer parses it. |
| `DemuxReceiver::with_demux_options` | ✅ Full | Construct a receiver around a custom `DemuxerOptions` (e.g. strict mode, PES caps, link overrides). |
| Stream-end contract | ✅ Full | `TransportError::Closed` → iterator termination after `Demuxer::flush`. `Broken` → `DemuxReceiverError::Transport(Broken(_))`. `Demux` → strict-mode rejection or malformed PES. |
| Receive-side gap buffer | ❌ Out of scope | Receive-side bytes that never arrived can't be replayed; no symmetric counterpart to `ManagedTransport`'s gap buffer. |

---

## Codec parameter set parsing (`tst-core::codec`)

Stateless typed parsers for codec parameter sets. The demuxer event surface
is unchanged — NAL bytes surface as `NalUnit` with raw RBSP. Consumers call
these parsers explicitly when they need typed fields (resolution, profile,
level, color, frame rate). See [`guide-codec.md`](guide-codec.md).

| Codec | Rust core | C ABI |
| --- | --- | --- |
| H.264 SPS / PPS (`codec::h264`) | ✅ Full | ❌ Deferred (rides with receiver C ABI) |
| H.265 VPS / SPS / PPS (`codec::h265`) | ✅ Full | ❌ Deferred |
| H.266 VPS / SPS / PPS (`codec::h266`) | ⚙️ Partial (VPS+SPS+PPS) | ❌ Deferred |
| AV1 Sequence Header + Frame Header light (`codec::av1`) | ⚙️ Partial | ❌ Deferred |

**H.264 notes:** wraps `h264-reader` 0.8; `parse_sps` / `parse_pps` /
`parse_parameter_sets`; partial-success-tolerant on combined call; strict on
per-set functions. 13/13 corpus fixtures matched ffprobe.

**H.265 notes:** hand-rolled per spec; `parse_vps` / `parse_sps` / `parse_pps` /
`parse_parameter_sets`. Known limitation: bails with `UnsupportedProfile` on
`scaling_list_data` and `num_short_term_ref_pic_sets > 0` paths (not
exercised by x265 default config or current corpus; full RPS parser is a
future enhancement).

**H.266 notes:** hand-rolled per H.266 V4 §7.3 / §7.4; `parse_vps` /
`parse_sps` / `parse_pps` / `parse_parameter_sets`. APS NALs (types 17 / 18),
Picture Header NALs (type 19), and multi-layer streams (`nuh_layer_id != 0`)
pass through unparsed. `color_info` / `frame_rate` surface as `None` until
the deeper SPS field-walk lands. Bails `UnsupportedProfile` on
`sps_subpic_info_present_flag = 1` / `sps_scaling_list_data_present_flag = 1`
(rare; not in reference encoder defaults).

**AV1 notes:** OBU-shaped (not NAL-shaped); `parse_sequence_header` /
`parse_frame_header_light` / `parse_obu_stream`. `Av1FrameHeaderLight`
surfaces `frame_type` + `show_frame` + `show_existing_frame` only;
`frame_size` is always `None` (full per-frame decode requires reference-
frame management beyond this parser's scope). Operating points beyond 0
walked but not surfaced. Tile Group / Metadata / Padding OBUs pass through
as `Obu { obu_type, payload, .. }` without further parsing.

---

## C ABI (`tst-c`)

| Feature | Status | Notes |
| --- | --- | --- |
| `tst_muxer_t` standalone utility | ✅ Full | open/push_video/push_klv/pull/close. Internally synchronized; data-path callable from multiple threads. |
| `tst_mux_sender_t` (plain L1) | ✅ Full | NAL+KLV in, TS+SRT out via `SrtTransport`. Internally synchronized. |
| `tst_managed_mux_sender_t` (managed L2) | ✅ Full | reconnect + gap buffer over `ManagedTransport<SrtTransport>`; synchronous retries on caller's thread. |
| `tst_ts_sender_t` / `tst_managed_ts_sender_t` | ✅ Full | pre-muxed TS bytes in; sync framing/recovery (RECOVER auto-resync + STRICT fail-fast); `_get_stats` accessor. |
| `tst_raw_sender_t` / `tst_managed_raw_sender_t` | ✅ Full | one `_send` call = one outbound SRT message. `TST_E_TOO_LARGE` on `len > SRTO_PAYLOADSIZE`. |
| Opaque builder configs | ✅ Full | `tst_mux_config_t`, `tst_ts_sender_config_t`, `tst_raw_sender_config_t`, `tst_reconnect_policy_t`. Internally cloned by `_open`; caller frees independently. |
| Thread-local last-error idiom | ✅ Full | `tst_get_last_error()` + `tst_get_last_error_str()`; ten `TST_E_*` codes covering all `tst_core` failure shapes. |
| `TST_VERSION_MAJOR` / `MINOR` / `PATCH` macros | ✅ Full | Compile-time `#define`s in `tstrans.h`. |
| Lifecycle (`_open` / `_close`) | ✅ Full | `_open` returns NULL on failure with last-error set; `_close` is idempotent and NULL-safe; close-from-any-thread serializes through `Mutex<Option<...>>`. |
| URL parsing | ✅ Full | `srt://host:port?key=value&...` — IPv4 / DNS / bracketed IPv6 hosts plus the libsrt-URL Group 1 vocabulary (`streamid` / `passphrase` / `latency` / `payloadsize` / `congestion` / `conntimeo` / `linger` / `udprcvbuf` / `udpsndbuf` / etc.) plus a handful of ffmpeg-style aliases (`pkt_size`, `payload_size`, `srt_streamid`, `tsbpddelay`, `smoother`, `ffs`, `connect_timeout`, `recv_buffer_size`, `send_buffer_size`). See "FFmpeg URL interop quirks" below for unit divergence. |
| Stats accessors | ✅ Full | `tst_muxer_get_stats` / `_reset_stats`; `tst_mux_sender_*` and `tst_managed_mux_sender_*` (get + reset); `tst_ts_sender_*` and `tst_managed_ts_sender_*` (get + reset); `tst_raw_sender_*` and `tst_managed_raw_sender_*` (get + reset). `TstSenderStats` + `TstMuxerStats` + `TstRawSenderStats` `repr(C)` types; receiver-side C stats deferred. |
| Multi-stream `mpegts::mux` fan-out | ✅ Full | `tst_video_stream_handle_t` / `tst_klv_stream_handle_t` typedefs (transparent `uint32_t`); `tst_mux_config_add_video_stream` / `_add_klv_stream` return handles at config time; `_video_to(handle, ...)` / `_klv_to(handle, ...)` siblings on `tst_muxer_t`, `tst_mux_sender_t`, and `tst_managed_mux_sender_t` (≤16 video + ≤16 KLV streams per program — same caps as the Rust core). Single-target entry points (`tst_*_send_video` / `_send_klv`) keep their signatures and surface `MuxError::AmbiguousTarget` as `TST_E_INVALID_USAGE` on multi-stream muxers. The `Sender` / `RawSender` variants don't carry a `Muxer`, so multi-stream is N/A there. |
| Multi-program `mpegts::mux` config | ✅ Full | `tst_program_handle_t` (transparent `uint32_t`); `tst_mux_config_add_program` returns a handle; `tst_mux_config_add_video_stream_to_program` / `_add_klv_stream_to_program` scope streams to a program; ≤16 programs per muxer config. |
| Multi-program demux at the C ABI | ❌ Deferred | Rust demuxer fully supports multi-program; C ABI receiver surface doesn't yet exist. Rides with the future receiver-side `tst-c` plan. |
| cbindgen-generated `tstrans.h` | ✅ Full | Committed at `crates/tst-c/include/tstrans.h`; CI verifies no drift via `tests/header_drift.rs`. |
| Symbol-prefix audit | ✅ Full | `tests/symbol_audit.rs` runs `nm -D` and asserts every exported symbol matches `^(tst_|TST_|srt_)` (`srt_*` allowlisted because libsrt is statically linked). |
| `pkg-config` metadata | ✅ Full | `tstrans.pc` generated by `build.rs` from `tstrans.pc.in`; substitutes `@VERSION@` and `@PREFIX@`. |
| Static-link discipline | ✅ Full | libsrt + mbedTLS + libstdc++ statically embedded into `libtstrans.so` and `libtstrans.a`; `ldd` shows only libc / libpthread / libstdc++ / libdl / libm. |
| Distribution artifacts | ✅ Full | `libtstrans.so` + `libtstrans.a` + `tstrans.h` + `tstrans.pc`. Tarball staged manually; GitHub Releases publishing not automated today. |
| End-to-end C smoke test | ✅ Full | `tests/smoke.c` compiled by `cc` and linked against the cdylib at test time; exercises muxer push/pull + every NULL-close path + invalid-URL last-error. |
| Live-socket roundtrip test | ✅ Full | `tests/live_pair.rs` binds a real `Listener` on 127.0.0.1, connects `tst_mux_sender_t`, sends a NAL, asserts the listener receives a TS sync byte. |
| Linux x86_64 build | ✅ Full | cdylib + staticlib + cbindgen header + pkg-config. |
| macOS / Windows / Linux aarch64 | ⏳ Planned | Cross-compilation follows demonstrated demand. |
| Pre-emptive close cancellation while parked in libsrt | ✅ Full | `Sender::close()` (and the underlying `Socket::cancel_handle()`) atomically closes the SRT handle from any thread, unblocking a peer thread parked in `srt_sendmsg`/`srt_recvmsg`. See `guide-pipeline.md`. |

---

## FFmpeg URL interop quirks

`ts-transformer` follows the libsrt-URL canonical conventions
(`srt-live-transmit`, OBS, mediamtx, gstreamer's `srtsink`/`srtsrc`,
Haivision Connect). FFmpeg's `srt://` protocol diverges in a few unit
conventions; users copying URLs between tools should be aware.

| URL key | FFmpeg unit | ts-transformer unit | Notes |
| --- | --- | --- | --- |
| `latency` | µs | ms | ts-transformer warns when value ≥ 10 s (likely paste from ffmpeg URL). |
| `rcvlatency` | µs | ms | Same warning. |
| `peerlatency` | µs | ms | Same warning. |
| `snddropdelay` | µs | (deferred) | Currently rejected as unsupported. |

When migrating an ffmpeg pipeline URL to ts-transformer, divide the latency
values by 1000.

ffmpeg-style key aliases honored by ts-transformer (zero new functionality —
just alternate spellings of existing keys): `pkt_size` / `payload_size`
(→ `payloadsize`), `srt_streamid` (→ `streamid`), `tsbpddelay` (→
`latency`), `smoother` (→ `congestion`), `ffs` (→ `fc`),
`recv_buffer_size` / `send_buffer_size` (→ `udprcvbuf` / `udpsndbuf`),
`connect_timeout` (→ `conntimeo`).

---

## Standards reference — what we cite vs. what we implement

The MISB / SMPTE / IETF documents that bear on `ts-transformer`. The
"Implemented?" column reflects what `ts-transformer` does, not what the spec
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
| **MISB ST 1402.2** | KLV in MPEG-2 Transport Streams | ✅ Async (0x06) + sync (0x15) modes in both `mpegts::mux` (encode) and `mpegts::demux` (decode) |
| **MISB ST 1607.2** | Constructs to Amend / Segment KLV | ❌ Out of scope (no multi-PES KLV in corpus) |
| **MISB ST 1910.1** | KLV in CMAF emsg boxes (HLS/DASH delivery) | ❌ Deferred — unrelated to MPEG-TS carriage; trigger on first CMAF/HLS consumer ask |
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
| `tst-core` | ✅ Full (sync) | Safe Rust API — `Socket`, `Listener`, builders, KLV. |
| `tst-c` | ✅ Full | cdylib + staticlib + cbindgen header + pkg-config; Linux x86_64. |
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
