# Deferred features

Things deliberately out of scope today with a clear path back if they
become load-bearing. Each entry records the reason it was deferred and
the trigger that would unblock it.

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

## ST 0102 universal-set form

- **Status:** Deferred. The Local Set form ships in `klv::st0102`
  (decode + decode_strict + encode); the parallel Universal Set form
  (16-byte UL per item, separate from the LS encoding) is not
  implemented.
- **Why deferred:** LS form is the only form on MPEG-TS+KLV streams.
  The Universal Set is for archival / file-based use cases the library
  does not target.
- **Trigger to revisit:** A consumer ingesting archival / file-based
  ST 0102-bearing streams that use the Universal Set encoding.

## ST 0102 country-code validation

- **Status:** Deferred. `klv::st0102` decodes the country coding
  method (Tags 2 / 12) as a typed enum but the country codes
  themselves (Tags 3 / 6 / 13) pass through as `String` verbatim. No
  validation against ISO 3166 / GENC / FIPS 10-4 / STANAG 1059 /
  CAPCO tables.
- **Why deferred:** Tables are large (GENC alone has 250+ codes plus
  admin subdivisions plus version dates plus deprecations),
  version-dependent, and a moving target across spec revisions.
  Pass-through strings sidestep the maintenance burden.
- **Trigger to revisit:** A compliance pipeline that requires
  validating codes against authoritative tables, AND a clear answer
  for which spec revision's table to bake in.

## Typed nested VMTI Local Sets (`VMask`, `VObject`, `VFeature`, `VTracker`, `VChip`)

- **Status:** Pass-through. The five LSes inside each `VTargetPack` —
  `vmask`, `vtracker`, `vchip`, `vchip_series`, `vobject_series` — are
  `Option<Vec<u8>>` raw bytes today.
- **Why deferred:** The structural per-target slice (target ID,
  centroid, bbox, lat/lon, dimensions, color, intensity, detection
  status, algorithm ID, etc.) covers the load-bearing analyst use
  case. Each nested LS is its own per-tag table to write and maintain
  — without a consumer asking for typed access, the table is carrying
  weight without paying for itself.
- **Trigger to revisit:** A consumer asks for typed access to per-
  target classification (VObjectSeries), feature vectors (deferred —
  ST 0903.6 deprecated the VFeature LS at Tag 103), track state
  (VTracker), pixel masks (VMask), or image cutouts (VChip /
  VChipSeries).

## Typed VMTI Algorithm + Ontology Series

- **Status:** Pass-through. `VmtiLs.algorithm_series` and
  `VmtiLs.ontology_series` are `Option<Vec<u8>>` raw bytes today.
- **Why deferred:** Same reasoning as the nested-LS entry above —
  per-tag tables without a driving consumer ask. Algorithm describes
  detector/tracker provenance; Ontology describes class label
  hierarchy.
- **Trigger to revisit:** A consumer asks for typed algorithm
  provenance or class-label hierarchy.

## VMTI standalone-PID demuxer dispatch (`MetadataKind::VmtiLs`)

- **Status:** Consumer-side dispatch. Consumers carrying VMTI on its
  own KLV PID match `data.starts_with(&klv::st0903::VMTI_LS_UL)`
  themselves and call `klv::st0903::decode` on the inner bytes (after
  stripping the 16-byte UL prefix and reading the BER outer length).
  The demuxer's `MetadataKind` enum has no VMTI-aware variant.
- **Why deferred:** Adding `MetadataKind::VmtiLs` to the demuxer event
  surface makes the demuxer typed-set-aware — and that's a slippery
  slope (do we then add `MetadataKind::SecurityLs`,
  `MetadataKind::Ais`, ...?). Today's pattern keeps the demuxer UL-
  agnostic and pushes dispatch to consumer code, which is where the
  typed-set decision naturally lives.
- **Trigger to revisit:** A consumer with VMTI on its own KLV PID
  asks for ergonomic dispatch, AND we're prepared to commit to a
  `MetadataKind::*` policy across all typed sets.

## VMTI Universal Set form

- **Status:** Local Set form ships in `klv::st0903` (decode +
  decode_strict + encode); the parallel Universal Set form (16-byte UL
  per item, separate from the LS encoding) is not implemented.
- **Why deferred:** LS form is the only form on MPEG-TS+KLV streams.
  The Universal Set is for archival / file-based use cases the library
  does not target.
- **Trigger to revisit:** A consumer ingesting archival / file-based
  VMTI-bearing streams that use the Universal Set encoding.

## `klv::st0806` RVT typed layer

- **Status:** Pass-through. Carried as ST 0601 Tag 73; consumers see
  raw bytes in the `unknown` field of `UasDatalinkLs` (no typed
  pass-through field today — could be added if a consumer asks,
  mirroring how Tag 48 → `security_local_set` and Tag 74 → `vmti`
  fields work).
- **Why deferred:** No consumer ask. ST 0806 PDF is not on hand —
  acquiring it (it's a public NGA spec) is a prerequisite. ST 0806 is
  metadata-emitting from receiver terminals (POI / AOI annotations,
  user-typed text) — narrower scope than VMTI but still its own per-
  tag table.
- **Trigger to revisit:** A consumer asks AND the ST 0806 PDF is
  obtained.

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

## Multi-stream `mpegts::mux` — `tst-jni` / `tst-uniffi` binding surface

- **Status:** The `tst-c` C ABI fan-out shipped — `tst_video_stream_handle_t` /
  `tst_klv_stream_handle_t` typedefs, `tst_mux_config_add_video_stream` /
  `_add_klv_stream` returning handles, and `_video_to(handle, ...)` /
  `_klv_to(handle, ...)` siblings on `tst_muxer_t`, `tst_mux_sender_t`,
  and `tst_managed_mux_sender_t`. The single-target entry points keep
  their original signatures and surface `MuxError::AmbiguousTarget` as
  `TST_E_INVALID_USAGE` on multi-stream muxers. The same handle-aware
  shape has NOT yet landed in `tst-jni` or `tst-uniffi`.
- **Note on Sender / RawSender:** the original deferred-features entry
  said `tst_ts_sender_*` / `tst_managed_ts_sender_*` would also gain
  `_video_to` / `_klv_to` siblings. That was wrong: `tst_pipeline::Sender`
  exposes only `send_ts(bytes)` (pre-muxed TS bytes) and `tst_pipeline::RawSender`
  exposes only `send(bytes)`. Neither carries a `Muxer`, so handle-aware
  fan-out is meaningless on those variants. Only the three muxer-owning
  C variants (`tst_muxer_t`, `tst_mux_sender_t`, `tst_managed_mux_sender_t`)
  have the new `_to` surface.
- **Trigger to revisit:** First JNI or UniFFI consumer that actually wants
  multi-stream output. The pattern is mechanical — mirror the same
  handle-typedef + `_to(handle, ...)` fan-out across the JNI/UniFFI
  binding once each ships.

## Codec parameter set parsing at the C ABI

- **Status:** Deferred. The Rust core ships `tst_core::codec::h264` and
  `tst_core::codec::h265` with typed parsers for SPS / PPS (H.264) and
  VPS / SPS / PPS (H.265). The C ABI exposure is deferred.
- **Why deferred:** The receiver-surface C ABI plan is the natural carrier
  for all FFI parser exposure — consistent ownership and error semantics
  across receiver fields, parameter-set fields, and future audio / subtitle
  parser exposure are best designed in one pass rather than piecemeal. An
  interim send-only or codec-only C ABI shape would need reshaping when the
  receiver C ABI lands anyway.
- **Trigger to revisit:** The receiver-surface C ABI plan starts execution.
  At that point the codec parsers get C entry points alongside the receiver
  event surface, sharing the same error-reporting and lifetime conventions.

## AV1 full Frame Header parsing

- **Status:** Deferred. `codec::av1::parse_frame_header_light` ships
  surfacing `frame_type` (KEY / INTER / INTRA_ONLY / SWITCH),
  `show_frame`, and `show_existing_frame`. Full Frame Header parsing
  (reference frame management, segmentation, loop filter, film grain,
  per-frame display size) is not in scope.
- **Why deferred:** Crosses into "you want a decoder." The light
  scope covers keyframe detection — the load-bearing use case for
  metadata extraction — and the cookbook recipes route off
  `frame_type == KEY` + `show_frame` for keyframe gating today.
- **Trigger to revisit:** A consumer shipping per-frame display-time,
  per-frame aspect-ratio, or `film_grain_params` consumption.

## AV1 still-picture / AVIF detection helper

- **Status:** Deferred. `Av1SequenceHeader::still_picture` and
  `reduced_still_picture_header` are parsed and surfaced, but no
  consumer-facing helper for "is this an AVIF?" exists yet.
- **Why deferred:** AVIF-in-MPEG-TS isn't a common consumer pattern;
  no consumer ask. Callers that need it can read the two raw flags off
  the parsed Sequence Header today.
- **Trigger to revisit:** A consumer shipping AVIF over SRT.

## AV1 multi-operating-point streams

- **Status:** Deferred. Operating points beyond OP 0 are walked past
  in the Sequence Header parser but not surfaced — `Av1SequenceHeader::level`
  and `tier` reflect OP 0 only.
- **Why deferred:** Single-OP is the common live-streaming pattern.
  Multi-OP (used for scalable encodes) is rare in real-world captures
  and absent from the local corpus.
- **Trigger to revisit:** A real-world capture with multi-OP AV1
  streams, or a consumer shipping scalable AV1.

## AV1-in-MPEG-2-TS binding §3.2 / §3.4 carriage conformance

- **Status:** Shipped (validate-1 C8). Default carriage is now
  `Av1CarriageMode::Mpeg2TsBinding`: PES `stream_id = 0xBD`
  (private_stream_1, §3.4) and `ts_open_bitstream_unit()` framing
  on each OBU (3-byte `obu_start_code` = `uimsbf(24)` = `0x000001`,
  i.e. byte sequence `0x00 0x00 0x01`, + emulation prevention
  bytes, §3.2). Set `MuxerConfig::av1_carriage =
  Av1CarriageMode::InteropRawObu` (escape hatch) for ffmpeg /
  libaom / hls.js / mediamtx interop carriage.
  Demuxer-side: matching `DemuxerConfig::av1_carriage`; binding
  mode surfaces `NonConformantIssue::Av1WrongStreamId` /
  `NonConformantIssue::Av1MissingTsObuFraming` on non-conforming
  input and falls back to raw-OBU parsing in lenient mode.

## `AV1_video_descriptor` (typed PMT descriptor)

- **Status:** Deferred. The muxer auto-emits the AV01 `registration_descriptor`
  (AV1-in-MPEG-2-TS binding §2.1) but not the optional typed
  `AV1_video_descriptor` from binding §2.2.
- **Why deferred:** The registration descriptor alone is sufficient for
  receiver classification — the demuxer routes off `format_identifier =
  "AV01"` today. The typed descriptor adds metadata (profile, level,
  tier, bit depth) that consumers can recover by parsing the Sequence
  Header OBU directly via `codec::av1::parse_sequence_header`.
- **Trigger to revisit:** A consumer that strictly requires the typed
  descriptor for transport-level metadata extraction without parsing
  the elementary stream.

## H.266 APS (Adaptation Parameter Set) parsing

- **Status:** Deferred. APS NALs (type 17 PREFIX_APS_NUT, type 18
  SUFFIX_APS_NUT) pass through as untyped `NalUnit::H266 { nal_type, .. }`
  with raw RBSP payload. VPS / SPS / PPS are typed via `codec::h266`.
- **Why deferred:** APS carries ALF (Adaptive Loop Filter), LMCS
  (Luma Mapping with Chroma Scaling), and scaling-list data — all of
  it useful only for full decode, not stream-level metadata extraction.
- **Trigger to revisit:** A consumer needing typed APS access for
  decoder pipeline introspection.

## H.266 Picture Header (PH_NUT, type 19) parsing

- **Status:** Deferred. Picture Header NALs pass through as untyped
  `NalUnit::H266 { nal_type: 19, .. }` with raw RBSP payload.
- **Why deferred:** Picture Header carries per-picture flags relevant
  to the decoder pipeline (picture_output_flag, GDR fields, partitioning
  overrides) — not load-bearing for stream-level metadata.
- **Trigger to revisit:** A consumer needing per-picture state
  extraction.

## H.266 multi-layer streams (`nuh_layer_id != 0`)

- **Status:** Deferred. The `nuh_layer_id` field is parsed off every
  NAL header but parameter sets aren't tracked per-layer — `parse_parameter_sets`
  fills a single (vps_id → vps, sps_id → sps, pps_id → pps) map across
  all layers.
- **Why deferred:** Multi-layer H.266 (the VVC scalability extension)
  isn't shipped by common encoders today and isn't in the local corpus.
- **Trigger to revisit:** A consumer using H.266 with scalability
  layers (spatial, quality, or multi-view).

## H.266 `stream_type 0x32` (VVC temporal video subsets)

- **Status:** Deferred. Only `stream_type 0x33` (VVC main video stream)
  is recognized as `VideoCodec::H266` by the demuxer.
- **Why deferred:** `stream_type 0x32` is for temporal subsetting in
  scalable VVC video — a rare use case absent from the corpus.
- **Trigger to revisit:** A consumer using temporal subsetting, or a
  capture surfacing 0x32 in the corpus. Workaround today:
  `DemuxerConfig::treat_as` lets callers manually classify the PID.

## AV1 on `0x80` user-private `stream_type`

- **Status:** Deferred. Only the binding-conformant AV1 carriage —
  `stream_type = 0x06` plus AV01 `registration_descriptor` — is
  auto-classified as `VideoCodec::Av1`. Non-conformant captures using
  `stream_type = 0x80` (user-private) require manual classification
  via `DemuxerConfig::treat_as`.
- **Why deferred:** 0x80 is reserved by H.222.0 for user-private use;
  some early AV1 captures used it before the binding settled.
  `DemuxerConfig::treat_as` covers the corner case without baking a
  non-conformant default into the auto-classifier.
- **Trigger to revisit:** A real-world capture stream with `stream_type
  0x80` plus AV01 registration that needs auto-classification (rather
  than a `treat_as` hint).

## SEI parsing for video codecs

- **Status:** Deferred. SEI NALs surface as `NalUnit::H264 { nal_type: 6, .. }`
  / `NalUnit::H265 { nal_type: 39 or 40, .. }` with raw RBSP payload — the same
  pass-through treatment as non-parameter-set NALs today.
- **Why deferred:** SEI parsing would expose HDR mastering display info (SEI 137),
  content light level (SEI 144), picture timing, and recovery-point info. Each
  SEI message type is its own sub-parser. No consumer has asked for any
  specific SEI type yet.
- **Trigger to revisit:** A consumer asks for a specific SEI type — most likely
  HDR mastering display (SEI 137) or content light level (SEI 144) for an HDR
  delivery pipeline.
- **Scope when added:** Case-by-case; each SEI type is a separate parser
  function in the same `codec::h264` / `codec::h265` module namespace.

## Audio frame parsers — AAC LATM and AC-3

- **Status:** Deferred. The MP2 (Layer I/II/III) and AAC ADTS frame
  iterators ship in 2026-05-07 (`codec::mpegaudio` + `codec::aac`).
  AAC LATM (ISO/IEC 14496-3 §1.7 LOAS/LATM framing) and AC-3 (ATSC
  A/52) frame parsers are not in scope.
- **Why deferred:** Neither codec appears in the local capture corpus
  (zero LATM events, zero AC-3 events across 250 files / 33 GB at plan
  #21 ship). Synthetic-only fixtures would be the validation path; we
  defer until a consumer or capture surfaces them so the work is
  driven by real-world bytes.
- **Trigger to revisit:** A consumer ships a stream needing LATM or
  AC-3 typed frame access, or a corpus capture surfaces either codec.
- **Scope when added:** AAC LATM lands as `codec::aac::latm` sibling
  module under the existing `aac/` directory (the directory layout was
  set up at the 2026-05-07 ship for exactly this future). AC-3 lands
  as a new top-level `codec::ac3` module. Both follow the same
  iterator-of-`Result<Frame, CodecParseError>` shape as the existing slice.

## Audio carriage at the `tst-c` C ABI

- **Status:** Deferred (no consumer ask). Audio carriage in `mpegts::mux`
  and `mpegts::demux` ships in Rust (codec scope: MP2 + AAC ADTS + AAC
  LATM + AC-3, plus `DemuxerConfig::treat_as` for non-conformant
  stream_type cases). The `tst-c` C ABI sender surface currently exposes
  `tst_mux_sender_send_video` and `tst_mux_sender_send_klv` but no
  `tst_*_send_audio` / `tst_*_send_audio_to` siblings, and the config
  builders do not expose `tst_mux_config_add_audio_stream` /
  `tst_audio_stream_handle_t`.
- **Why deferred:** Adding the entries is mechanical (parallel to the
  existing video / KLV send entries) but requires deciding the audio
  frame envelope shape at the C boundary — whether to take raw access
  units, ADTS frames, LATM blocks, etc., and how to surface the codec
  selection per stream. No consumer has asked.
- **Trigger to revisit:** A binding-author asks for audio send through
  the C ABI; a downstream consumer needs in-band audio for a use case
  not served by the Rust API.

## Non-ATSC AC-3 variants (E-AC-3, DVB-shaped AC-3)

- **Status:** Deferred. `mpegts::mux` emits and `mpegts::demux` recognizes
  ATSC-shaped AC-3 only — `stream_type 0x81`, with `format_identifier =
  "AC-3"` in a registration descriptor (the shape ffmpeg's mpegts muxer
  emits by default). E-AC-3 (`stream_type 0x87`) and DVB-shaped AC-3
  (`stream_type 0x06` + AC-3 registration descriptor) are not classified
  as `AudioCodec::Ac3` automatically.
- **Why deferred:** Neither variant appears in the local corpus. Adding
  them means either (a) parsing registration descriptors on every
  `stream_type 0x06` PID to disambiguate "AC-3" from "KLVA" / "HDMV" /
  etc. (a structural complication that isn't justified without a corpus
  signal), or (b) adding a new `AudioCodec::EAc3` variant plus the
  corresponding stream_type byte / muxer / demuxer plumbing.
- **Workaround:** `DemuxerConfig::treat_as` lets callers map an
  `Unknown(0x87)` PID or a `0x06` PID with the AC-3 registration
  descriptor to `AudioCodec::Ac3`. The library hands back raw PES
  bytes; the caller's decoder handles whatever framing is actually
  present.
- **Trigger to revisit:** A capture surfaces in the corpus or a consumer
  ships either variant.

## Typed audio descriptor helpers in `mpegts::descriptors`

- **Status:** Deferred. Per-stream PMT descriptors are caller-supplied via
  `MuxerConfigBuilder::stream_descriptors_for_audio` (parallel to `_for_video` /
  `_for_klv` from plan #17). Two auto-emit helpers ship: `add_audio_with_language(pid, codec, lang)` emits an `iso_639_language_descriptor`
  (tag 0x0A); `AudioCodec::Ac3` streams auto-emit a `registration_descriptor`
  with `format_identifier="AC-3"`. Codec-specific helpers (`ac3_audio()` —
  descriptor tag 0x6A in DVB / 0x81 in ATSC; `aac_audio()` — tag 0x7C;
  `mpeg2_audio()`) are not added.
- **Why deferred:** No consumer has asked for typed audio descriptors,
  and the corpus shape is bare PMT entries (no audio descriptors at all
  on AAC / MP2 streams across the local capture set). Callers who need a
  codec-specific audio descriptor today assemble one via
  `user_private_with_tag(tag, payload)` from the existing helper menu and
  attach via `stream_descriptors_for_audio`.
- **Trigger to revisit:** A consumer needs a specific typed audio
  descriptor, OR the audio frame parser plan lands and pulls the descriptor
  surface into scope alongside the parsed frame metadata.

## Heuristic payload-kind detection (`codec::detect`)

- **Status:** Deferred. The demuxer maps `Unknown { stream_type, raw }` for
  PIDs it can't classify from the PMT (unregistered stream_types, missing
  descriptors). No heuristic inspection is applied.
- **Why deferred:** Heuristics (looks-like-ADTS, looks-like-UL+BER,
  looks-like-Annex-B H.264, etc.) are useful for the local-capture
  exploration use case — feeding in an unfamiliar capture and learning what's
  in it — but they add complexity and false-positive risk. A dedicated
  inspection plan is the right home.
- **Trigger to revisit:** A consumer asks for content-type detection on
  `Unknown` PIDs, or a corpus analysis workflow needs stream-kind heuristics
  without PMT descriptors.

## `pipeline::ext::pairing` — opt-in convenience pairing utility

- **Status:** Shipped (Rust API). `tst_pipeline::ext::pairing::Pairer` with
  `with_config` (Realtime + Buffered) and `last_before_pts` strategies.
  Cookbook recipes 24–27 cover the canonical patterns; recipes 12–14
  remain as the inline-pattern reference. C ABI / JNI / UniFFI
  exposure deferred — see the next entry.

## `pipeline::ext::pairing` C ABI / JNI / UniFFI exposure

- **Status:** Rust API only. `tst-c`, `tst-jni`, `tst-uniffi` do not
  yet expose `Pairer`.
- **Why deferred:** Receiver-side cross-language surfaces are deferred
  to the future receiver-surface plan, so all receiver-side exposure
  (multi-program demux at C ABI, receiver-side stats at C ABI, typed
  codec parsers at C ABI, audio / subtitle / AV1 / H.266 carriage at
  C ABI, and now `Pairer`) lands coherently in one pass instead of
  piecemeal. The Rust API was designed with FFI in mind: flat
  projection structs (`VideoSample`, `KlvSample`), a tagged-enum
  output (`PairerOutput`) that maps to C discriminator + union, and
  no lifetimes.
- **Trigger to revisit:** When the receiver-surface C ABI plan is
  written, `Pairer` joins as one more handle type
  (`tst_pairer_t`, `tst_pairer_open_with_config`,
  `tst_pairer_last_before_pts_open`, `tst_pairer_feed`,
  `tst_pairer_flush`, `tst_pairer_stats`, `tst_pairer_close`).
- **Scope when added:** ~7 C entry points + 1 handle type + tagged
  output discriminator. Sketch parallel to the existing
  `tst_demux_receiver_t` shape.

## Multi-program demux at the C ABI

- **Status:** Shipped in Phase 3 (plan #62, 2026-05-16). The
  `tst_demux_receiver_t` typed-event surface includes `tst_event_t`
  with a `PROGRAM_MAP` arm carrying `program_number`; `tst_stream_info_t`
  carries `program_number`; multi-program streams are handled naturally
  by the `DemuxReceiver` backend.

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
  `tst-srt` plus its typed wrapper / validation. Single-developer
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
  `tst_get_last_error_str()` is for failures, not warnings. Adding a
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

- **Status:** Shipped in plan #30 (commit cf3233b).
  `Listener::accept_timeout(Duration)` uses a one-shot `srt_epoll_wait`
  to gate readiness, then calls `srt_accept` once a connection arrives or
  returns `AcceptError::TimedOut` on expiry. `Listener::set_recv_timeout`
  continues to apply only to *accepted* sockets, not to the accept call
  itself — see `guide-srt.md` §Blocking semantics for the distinction.

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

- **Status:** Shipped 2026-05-07. `SocketConfig::sender_defaults()` /
  `::receiver_defaults()` constructors, `merge_sender_defaults()` /
  `merge_receiver_defaults()` in-place merge methods, and matching
  `SocketBuilder::sender_defaults()` / `::receiver_defaults()` chain
  methods all live in `tst-srt`. The `tst-c::connect_srt` helper now
  calls `SocketConfig::merge_sender_defaults` instead of inlining the
  merge logic. See the "Sender / receiver presets" section in
  `docs/guides/srt.md`.

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
  `Sender<ManagedTransport<...>>` / `Receiver<ManagedRecvTransport<...>>`
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
  (extra two fields per `tst_stream_stats_t`). Either way it's a
  bigger surface than the rest of the stats struct, and consumers
  who care can derive staleness from `items` deltas + their own
  wall-clock sampling cadence.
- **Trigger to revisit:** A consumer ships a watchdog that needs
  per-stream staleness without holding two snapshots, or asks for
  millisecond-resolution event timing in the stats surface.

## Per-stream PMT descriptor surface at the C ABI

- **Status:** The Rust core ships per-stream PMT descriptors via the
  `mpegts::descriptors` module and `MuxerConfigBuilder::stream_descriptors_for_video` /
  `stream_descriptors_for_klv` / `stream_descriptors_for_stream` methods.
  The C ABI exposure is deferred.
- **Why deferred:** The descriptor-construction surface and the future
  receiver C ABI's per-stream descriptor surface should land together —
  exposing a send-only C ABI shape now would need reshaping when the
  receiver C ABI lands (which will also need `TstRawDescriptor` and
  read access to `StreamInfo::raw_descriptors`).
- **Trigger to revisit:** The receiver-surface design lands and pulls
  the descriptor surface into scope. At that point the C ABI gets
  descriptor builders mirroring `mpegts::descriptors` plus
  `tst_mux_config_set_video_stream_descriptors` /
  `_set_klv_stream_descriptors` with bounded array params + a
  `TstRawDescriptor` `repr(C)` shape for the receive side's
  `StreamInfo::raw_descriptors`.

## `Socket::close` Result-type cleanup

- **Status:** `tst_srt::Socket::close(self) -> Result<(), IoError>` always
  returns `Ok` after the 2026-05-03 cancellation refactor. The
  underlying `srt_close` return code is consumed inside the
  `SrtCancelHandle` closer (which has a `Fn` signature, no return path).
  Same applies to `tst_srt::Listener::close`.
- **Why deferred:** The signature is preserved for API stability —
  changing it now would be a breaking change for consumers who pattern
  on `if let Err(e) = sock.close()`. A future breaking-change cycle
  could either drop the `Result` entirely (close becomes infallible)
  or plumb `srt_close`'s rc back via a richer `CloseError` channel.
- **Trigger to revisit:** Next breaking-change cycle, OR a consumer
  reports needing the `srt_close` rc (e.g., to distinguish
  graceful-close from race-close errors).

## Pre-emptive close cancellation at the C ABI

- **Status:** Partially shipped. All six sender `_cancel` entry points
  ship in Phase 1 (plan #59). `tst_raw_receiver_cancel` ships in
  Phase 1; `tst_receiver_cancel` and its managed sibling ship in
  Phase 2 (plan #60). The remaining `tst_demux_receiver_cancel` rides
  with Phase 3.
- **Why deferred (originally):** The C ABI's `Handle<T>`
  (= `Mutex<Option<T>>`) has the same blocking issue at the C layer
  that the Rust shells had — `tst_*_close` waits on the handle's
  mutex, so it competes with a parked C-side data-path call. Fixing
  it cleanly requires a side-channel `Arc<dyn TransportCancel>` +
  `Arc<AtomicBool>` captured at `_open` time, outside the mutex.
  That design was implemented in Phase 1 and carried forward.
- **Status (updated 2026-05-16):** `tst_demux_receiver_cancel` shipped
  in Phase 3 (plan #62). Pre-emptive close cancellation is now complete
  across all six sender families and all three receiver handle types.

## Typed WebVTT cue substrate (`mpegts::webvtt::format_pes_payload` + `WebVttCue`)

- **Status:** Deferred. WebVTT-in-TS carriage ships in plan #22
  (2026-05-04); `Muxer::push_subtitle` accepts pure pass-through
  bytes (caller hand-builds the cue PES payload). A typed substrate
  with `WebVttCue { identifier, start, end, settings, payload }` and
  `format_pes_payload(&WebVttCue) -> Vec<u8>` (write side) +
  `parse_pes_payload(bytes) -> Vec<WebVttCue>` (read side) is not
  shipped.
- **Why deferred:** Mirrors how `klv::st0601` typed builder layered
  on top of the `klv` byte substrate — typed layer is a separate
  session's worth of work. Downstream consumers (e.g. HLS POI
  injection) can build cue bytes ad-hoc until the typed layer ships.
- **Trigger to revisit:** A consumer asks for typed cue
  parameters / serialization or the second WebVTT consumer
  reimplements the same byte-builder logic.

## Typed DVB-sub data segment / DVB-teletext data unit / CEA-708 cc_data parsers

- **Status:** Deferred. Plan #22 ships carriage layer only — payload
  bytes pass through verbatim. Typed parsers (`subtitle_data_segment`
  per ETSI EN 300 743; `teletext_data_unit` per ETSI EN 300 706;
  `cc_data_pkt` per CEA-708-D) do not exist.
- **Why deferred:** No driving consumer for typed access today; the
  typed layer is a separate session's worth of work per codec.
- **Trigger to revisit:** A consumer asks for typed access to
  specific fields (page composition pixel-data, teletext line
  Hamming-decoded text, CEA-708 caption text channel). Resolving
  this entry will also wire `NonConformantIssue::SubtitleDescriptorMalformed`
  (currently a reserved variant — the classification cascade is
  tag-presence-based via `find_descriptor_tag`, so malformed
  descriptor bodies pass through today).

## WebVTT-in-TS interop

- **Status:** Deferred. WebVTT-in-MPEG-TS carriage ships in plan #22
  (registration_descriptor `"VTTC"` + single-cue PES + subtitle PID
  excluded from PCR fallback) and round-trips through the library's
  own mux + demux. Interop with external tools (ffmpeg, hls.js,
  mediamtx, etc.) has not been empirically verified.
- **Why deferred:** The `"VTTC"` format_identifier is not defined by
  any published normative spec (RFC 8216, draft-pantos-hls-rfc8216bis,
  Apple HLS authoring docs — none mention it). It appears in ffmpeg's
  `mpegtsenc.c` emitter and is widely observed in WebVTT-in-TS
  captures, but the cross-tool interop is empirical, not normative.
  Empirical interop testing requires fixture corpus from each tool +
  a test matrix — a separate session's worth of work.
- **Trigger to revisit:** Validate-1 Wave I (empirical interop matrix)
  schedules an interop test against ffmpeg / hls.js / mediamtx /
  GStreamer; results from that pass either confirm interop or
  surface concrete divergences requiring spec follow-up.

## CEA-708 interop

- **Status:** Deferred. CEA-708 caption data as a standalone
  elementary stream ships in plan #22 (registration_descriptor
  `"GA94"` + private-data PES). Library-internal round-trip works;
  interop with ATSC ecosystem tooling (decoders, MPEG-2 video user_data
  bridges) has not been empirically verified.
- **Why deferred:** ATSC A/53 Part 4 §6.2.3 defines `"GA94"` as the
  `user_data_identifier` for caption data **embedded in MPEG-2 video
  user_data**, not as a stream-level marker. Using it for standalone
  PES carriage is best-effort interop with ATSC ecosystem tooling,
  not normatively defined. Empirical interop testing requires the
  same fixture / matrix infrastructure as the WebVTT-in-TS entry
  above.
- **Trigger to revisit:** Validate-1 Wave I (empirical interop matrix)
  schedules CEA-708 interop testing against ATSC ecosystem tooling;
  results from that pass either confirm interop or surface the need
  for a different marker convention.

## Subtitle carriage at the `tst-c` C ABI

- **Status:** Deferred (no consumer ask). Plan #22 ships sender-side and
  receiver-side Rust APIs covering DVB-sub, teletext, CEA-708, and
  WebVTT-in-TS. The `tst-c` C ABI sender surface currently exposes
  `tst_mux_sender_send_video` and `tst_mux_sender_send_klv` but no
  `tst_*_send_subtitle` / `tst_*_send_subtitle_to` siblings, and the
  config builders do not expose `tst_mux_config_add_subtitle_stream` /
  `tst_subtitle_stream_handle_t`.
- **Why deferred:** Adding the entries is mechanical (parallel to the
  existing video / KLV send entries) but requires deciding the subtitle
  envelope shape at the C boundary across the four supported codec
  families. No consumer has asked.
- **Trigger to revisit:** A binding-author asks for subtitle send
  through the C ABI; a downstream consumer needs in-band subtitles
  for a use case not served by the Rust API.

## ARIB STD-B24 / ARIB STD-B37 subtitling

- **Status:** Deferred. Japanese broadcast subtitling carriage is
  not classified by the cascade. ARIB-shaped PIDs surface as
  `Unknown` today.
- **Why deferred:** No consumer ships ARIB content. The PMT shape
  diverges enough from ETSI / Apple forms that adding it is its
  own design.
- **Trigger to revisit:** A Japanese broadcast consumer.

## WebVTT out-of-band for HLS (separate `*.vtt` segment files)

- **Status:** Not in scope. The HLS spec lets subtitles ride
  out-of-band as separate `*.vtt` segment files referenced from a
  `#EXT-X-MEDIA:TYPE=SUBTITLES` entry in the playlist — no MPEG-TS
  involvement.
- **Why deferred:** Not a `ts-transformer` concern. Out-of-band WebVTT
  delivery is an HLS-packager / orchestrator concern outside this
  library.
- **Trigger to revisit:** Never (different layer).

## Real-world public-broadcast subtitle fixture acquisition

- **Status:** Deferred. Plan #22 ships synthetic-only fixtures
  (~200 KB) generated by `gen_subtitle_fixtures` — no real
  broadcast captures.
- **Why deferred:** Synthetic + ffprobe cross-check is enough for
  ship; real broadcast captures add coverage if/when a real-world
  bug surfaces that the synthetic suite missed. Acquisition has
  legal / licensing overhead too.
- **Trigger to revisit:** A consumer reports a real-world bug the
  synthetic suite missed.


## Multi-cell fragmented metadata AU cells

- **Status:** SHIPPED 2026-05-24.
- **Plan:** `docs/plans/2026-05-24-multi-cell-au-reassembly.md` (outside the published repo).
- **Behavior:** the demuxer now reassembles fragmented AU cells per
  H.222.0 V9 §2.12.4.2 Table 2-157. Both flavors covered:
  - Multiple AU cells back-to-back within one PES — every cell emits
    its own event; previously only the first cell did.
  - Cells of one AU spread across multiple PES packets — `First` +
    `Middle`* + `Last` accumulate in a per-PID buffer until `Last`
    completes the AU; the demuxer then emits one event with
    `MetadataKind::KlvSyncAuCell::was_reassembled = true` and
    `cell_count = N`.
- **Failure modes:** `NonConformantIssue::MultiCellAu` now carries a
  typed `reason: MultiCellAuReason` (`Orphan` / `SequenceGap` /
  `ConcurrentFirst` / `Overflow`). Per-PID buffer cap configurable via
  `DemuxerConfig::au_cell_cap_per_pid` (default 1 MiB).
- **Out of scope:** caller override of `random_access_indicator` /
  `decoder_config_flag` on the mux side; mux-side emit of fragmented
  output. Both remain as separate deferred entries.

## Caller override of `random_access_indicator` / `decoder_config_flag` on mux

- **Status:** Deferred.
- **Why deferred:** `Muxer::push_klv_to` hard-codes
  `random_access_indicator=true` (every push is an entry point —
  correct for self-contained ST 0601 LS records) and
  `decoder_config_flag=false` (we do not carry decoder
  configuration). For stateful KLV sets — ST 1206 SAR with delta
  encoding, ST 0902 motion imagery with reference-frame-relative
  VMTI — "entry point" semantics differ; only some pushes would be
  RAI=1. The current ST 0601 typed surface is correctly served by
  the hard-coded defaults.
- **Trigger to revisit:** A typed surface for a stateful set
  lands, OR a consumer emits non-ST-0601 sync KLV that needs
  different semantics. Likely landing shape:
  `Muxer::push_klv_to_with_config(handle, klv, pts,
  SyncKlvConfig { random_access_indicator,
  decoder_config_flag })` — following the workspace
  `_with_config` constructor convention (see `docs/reference/conventions.md`).

## ST 1910.1 KLV-in-CMAF-emsg-box delivery

- **Status:** Deferred.
- **Why deferred:** ST 1910.1 (Adaptive Bitrate Content Encoding,
  2020) defines KLV-in-CMAF emsg-box delivery for HLS/DASH
  consumption — separate from MPEG-TS carriage. No CMAF/HLS
  consumer asks for this in the current pipeline. Note: this is
  unrelated to the MPEG-TS sync-metadata AU cell at
  `mpegts::au_cell` (per H.222.0 § 2.12.4.2) — different specs,
  different layers.
- **Trigger to revisit:** An HLS/DASH-delivery consumer needs to
  ingest sync KLV from a CMAF stream (e.g., a future HLS pipeline
  that elects the emsg-box path instead of the MPEG-TS path).

## DVB-shaped AC-3 (`stream_type 0x06` + AC3_descriptor `0x6A`)

- **Status:** Deferred. AC-3 carriage is ATSC-shaped only —
  `stream_type 0x81` with `format_identifier="AC-3"` registration
  descriptor (the shape ffmpeg's mpegts muxer emits by default).
  The DVB shape uses `stream_type 0x06` with `AC3_descriptor`
  (tag `0x6A`) per ETSI TS 101 154 §5.6.
- **Why deferred:** No consumer in the current target deployment
  uses DVB-shaped AC-3. Adding the path means either a new
  `AudioCodec::Ac3Dvb` enum variant (parallel to existing `Ac3`)
  or a `MuxerConfig::ac3_mode: Ac3Mode { Atsc, Dvb }` switch — both
  expand the public API without a use case. ATSC-only mode covers
  every known consumer.
- **Workaround:** A receiver consuming DVB-shaped AC-3 today
  classifies as `Unknown(0x06)` unless the caller passes
  `DemuxerConfig::treat_as` mapping the PID to `AudioCodec::Ac3`;
  the library hands back raw PES bytes regardless of the
  registration descriptor shape.
- **Trigger to revisit:** A DVB-only receiver appears in the
  target deployment, or a corpus capture shows DVB-shaped AC-3.

## Auto-prepend of access-unit delimiter (AUD) on H.264 / H.265 / H.266

- **Status:** Deferred. Caller is responsible for prepending the
  codec-specific AUD NAL when required. `Muxer::push_video_to`
  passes the caller's NAL stream through verbatim (post-Annex-B
  framing validation).
- **Why deferred:** ffmpeg's `mpegtsenc.c:1907-2069` auto-inserts
  AUD on H.264/H.265 if missing, but the AUD NAL type and content
  differ across codecs (H.264 type 9, H.265 type 35, H.266 type
  20) and the encoder side already emits AUD on most modern
  toolchains (x264 with `--aud`, x265 with `--aud`, libavcodec
  with `flags +aud`). Adding auto-insert means a bit-stream-aware
  filter on every video push and a codec-dispatch table — non-
  trivial without a consumer-driven need.
- **Trigger to revisit:** A consumer reports decoder breakage on
  AUD-required hardware decoder (some HW decoders, libde265 in
  certain configurations, broadcast-grade STBs) when streams
  arrive without AUD. Likely landing shape:
  `MuxerConfig::auto_aud: bool` gate on the muxer, defaulting off,
  with per-codec NAL emission.

## SRT URL `mode=listener` / `mode=rendezvous` dispatch

- **Status:** Deferred. The URL parser at `tst-srt/src/url.rs`
  rejects `mode=listener` and `mode=rendezvous` with a
  ts-transformer-specific error; only `mode=caller` is accepted.
  The URL surface is sender-only today.
- **Why deferred:** The receiver-side C ABI surface is the
  current P0 work item (see project ROADMAP). Listener-side URL
  dispatch is naturally part of that plan: the receiver C entry
  points are what need `mode=listener` URLs to bind a passive
  socket and accept incoming connections. Landing parser-only
  support today (accepting the keyword without dispatching it)
  creates a half-implemented feature that crashes or silently
  misroutes when a caller actually uses it.
- **Trigger to revisit:** Receiver C ABI surface plan begins.
  That plan's URL dispatch will reuse the existing `parse`
  function and route on `mode` to either the existing caller
  path or new listener / rendezvous paths.

## Media over QUIC (MoQ) transport target

- **Status:** Deferred. The only transport implementation is
  `tst-srt::SrtTransport` over libsrt. The IETF MoQ Transport
  draft (`draft-ietf-moq-transport`) and its MSFTS payload-
  format extension (`draft-gregoire-moq-msfts`, which carries
  MPEG-TS packets over MoQ) are not implemented and have no
  scaffolding in the workspace.
- **Why deferred:** Project scope is SRT-only by design. MoQ
  Transport itself is still a working-group draft; MSFTS is
  `draft-00`, Informational, individual submission, May 2026.
  Both specs are too early to commit binding code to. No
  consumer has asked for browser-reachable delivery, which is
  the natural pull-through for a MoQ binding. The `Transport`
  trait in `tst-pipeline` already cleanly decouples the SRT
  crate from `tst-core`, so this remains an additive future
  move rather than a refactor.
- **Trigger to revisit:** Any of: (1) a consumer asks for
  browser-side delivery that MoQ would enable; (2) MoQ
  Transport reaches WGLC; (3) MSFTS publishes a `-01` revision
  with metadata-stream / sidecar-data signaling (e.g. a
  KLV-aware mapping) or picks up an ISR-aware co-author;
  (4) ffmpeg or gstreamer ship a stable MoQ output that
  becomes a de facto receiver target.
- **Scope when added:** A new `tst-moq` crate parallel to
  `tst-srt`, exposing `MoqTransport` / `MoqRecvTransport`
  implementing the existing traits. Because MSFTS preserves
  TS packets verbatim, the existing `tst-core` mux/demux
  passes through unchanged — KLV-in-TS rides over MoQ
  without codec or framing changes. A QUIC stack dependency
  (likely `quinn`) is the main new build axis. URL surface
  gets a `moq://` family alongside `srt://`.

## iOS (arm64 device + arm64 simulator + x86_64 simulator)

- **Status:** Deferred. `tst-c` builds Linux x86_64, Linux
  aarch64, macOS arm64, and Windows MSVC today (see
  `compatibility.md` build-targets table).
- **Why deferred:** iOS requires the Xcode SDK + a macOS-based
  build runner + iOS-specific libsrt / mbedTLS cmake toolchain
  files (iOS SDK paths, simulator vs device arch selection,
  framework-vs-static packaging). The work is significant and
  the field set is best landed alongside `tst-uniffi` so
  consumer-facing iOS packaging shape (xcframework? CocoaPod?)
  drives the build-side decisions rather than the other way
  around.
- **Trigger to revisit:** The `tst-uniffi` implementation plan
  starts. iOS support lands as part of that plan, not before.
- **Scope when added:** Three matrix entries (arm64 device,
  arm64 simulator, x86_64 simulator) under a separate iOS-
  specific CI workflow (the existing GHA `macos-14` runner
  can host all three via `xcodebuild` cross-targeting). The
  Rust target triples are `aarch64-apple-ios`,
  `aarch64-apple-ios-sim`, `x86_64-apple-ios`.

## Android (arm64 + x86_64 emulator + armv7)

- **Status:** Deferred. macOS arm64 + Windows MSVC are Tier 1
  (see `compatibility.md`).
- **Why deferred:** Android requires the Android NDK toolchain
  + cross-compile toolchain files for both libsrt and mbedTLS
  (the NDK sysroot, libc shape, and ABI selection per target
  arch). The work is bundled with iOS as part of the future
  `tst-uniffi` plan — mobile-binding consumers expect both
  platforms together, and the JNI-style shared-library
  packaging is symmetric.
- **Trigger to revisit:** The `tst-uniffi` implementation plan
  starts. armv7 specifically is the most-likely-to-stay-
  deferred sub-target — only re-included if a consumer reports
  the device class matters (modern Android devices have been
  arm64 since ~2018).
- **Scope when added:** NDK sysroot + cmake toolchain files
  for libsrt + mbedTLS, plus Rust target triples
  `aarch64-linux-android`, `x86_64-linux-android`, and
  (conditionally) `armv7-linux-androideabi`.

## macOS x86_64 (Intel)

- **Status:** Deferred. macOS arm64 (Apple Silicon) is Tier 1.
- **Why deferred:** Intel Macs are a declining install base;
  Apple Silicon covers the contributor and laptop case for
  modern macOS. Maintaining Intel-mac support would double the
  macOS CI surface (one runner per arch) for diminishing
  return.
- **Trigger to revisit:** A consumer running an Intel Mac
  reports a build failure they want fixed.
- **Scope when added:** A `macos-13` matrix entry (last Intel-
  only macOS runner; `macos-14`+ are arm64) in
  `.github/workflows/ci.yml` with `continue-on-error: true`
  initially, mirroring the Tier 1 phase-in pattern.

## Windows MinGW (gcc toolchain)

- **Status:** Deferred. Windows MSVC is Tier 1.
- **Why deferred:** MSVC covers the production Windows case
  (most distributed Windows binaries link MSVCRT). MinGW is
  dev-environment friendly but doubles the Windows CI surface
  (one runner per toolchain) and has its own set of vendored-
  library quirks distinct from MSVC.
- **Trigger to revisit:** A consumer asks for a non-MSVC
  Windows build (e.g., they're shipping a MinGW-based
  application and the toolchain mismatch creates linker
  friction).
- **Scope when added:** A matrix entry using the
  `x86_64-pc-windows-gnu` Rust target on `windows-latest`
  with `continue-on-error: true` initially, mirroring the
  Tier 1 phase-in pattern.

## Windows MSVC runtime tests — RESOLVED 2026-05-29 (sub-deferrals remain)

- **Status:** RESOLVED. windows-msvc now runs the full runtime test
  suite and is green across all four platforms. Plan #65's "SRT
  loopback hangs on Windows" diagnosis turned out STALE — it was an
  artifact of the pre-MSVC-`cl` librist build; on the cl-built libsrt
  the blocking `srt_recv` wakes on peer-close immediately (proven by a
  bounded diagnostic), same as Linux. The actual blocker was a real
  `SRTO_LINGER` struct-ABI product bug: `LingerOpt` was two `int`
  (8 bytes), but Winsock `struct linger` is two `u_short` (4 bytes), so
  libsrt's `cast_optval<linger>` rejected the size (`MJ_NOTSUP/MN_INVAL`)
  → every sender connect failed → receiver-accept hangs. Fixed
  per-platform in `crates/tst-srt/src/socket.rs`. CI now runs all
  platforms under cargo-nextest, so per-test timeouts bound any future
  hang (one hang can no longer stall the job).
- **Remaining sub-deferrals** (each gated `#[cfg(not(target_os =
  "windows"))]`; tracked in ROADMAP "Fully-green test suite") —
  1. **Promote windows-msvc to gating:** still
     `continue-on-error: true`; flip to `continue: false` in the
     `ci.yml` build matrix once it has several consecutive green runs.
  2. **RIST runtime on Windows:** `tst-rist/tests/{loopback,
     pipeline_round_trip}.rs` are gated off windows — librist's
     Main-Profile AES-256 encrypted handshake hangs there (genuine
     investigation needed; compile + link stay covered by the build
     steps + the tst-c `rist` feature build).
  3. **Multicast on Windows:** `tst-rtp`/`tst-udp` `loopback_multicast`
     + the `tst-rtp` `build_multicast_with_iface_v4` unit test are gated
     off windows — GHA Windows runners don't loop multicast back and
     Winsock rejects `IP_MULTICAST_IF=loopback`. Most likely a
     runner-environment limitation rather than a code bug; confirm, then
     either un-gate on a multicast-capable runner or document permanent.
- **Trigger to revisit:** the next-session "fully-green test suite"
  pass (RIST + multicast investigations), then the gating promotion.

## RTSP server/client deferred test surface (`#[ignore]`d)

- **Status:** Four `tst-rtp` RTSP tests are `#[ignore]`d pending a
  feature, a fixture, or a harness — not platform-gated, just not yet
  runnable in CI:
  1. `rtsp_server/tls.rs` — client-side custom root-store wiring not
     implemented (`RtspClientBuilder::tls_root_certs` is stored but
     unused), so the end-to-end `rtsps://` handshake can't be asserted.
  2. `rtsp_client/tls_keepalive.rs` — needs an `rtsps://` cert fixture
     (rcgen dev-dep not present); activate with `RTSP_TLS_FIXTURE=1`.
  3. `rtsp_server/lagging_peer.rs` — deterministic slow-consumer
     reproduction needs a throttled test harness (the underlying
     drop-counter behavior is unit-tested in `fanout.rs`).
  4. `rtsp_client/interleaved_e2e.rs`
     (`tcp_interleaved_end_to_end_round_trips_ts_bytes`) — re-ignored
     post-merge (hangs in the post-PLAY drop sequence in the merged
     state); the interleaved wire-up is covered by
     `rtsp_server_loopback_interleaved` + `rtsp_server_notice_5402`.
- **Why deferred:** each is a self-contained follow-up (TLS root-store
  plumbing, a cert fixture, a throttle harness, an interleaved-drop
  shutdown fix) carved out of the tst-rtp Phase 2/3 waves.
- **Trigger to revisit:** the next-session "fully-green test suite"
  pass folds these in alongside the Windows un-gating.

## Audio frame iterators for LATM + AC-3

- **Status:** Deferred. No consumer trigger. Existing iterators in
  `tst_core::codec::*` cover MP2 (`mpegaudio::frames`) and AAC-ADTS
  (`aac::frames`) only — shipped in plan #34.
- **Why deferred:** AAC-LATM (`audio_mux_element` +
  `payload_length_info` framing per ISO/IEC 14496-3) and AC-3
  (ETSI TS 102 366 §6 syncword + frame-size table) both have
  well-defined per-spec frame boundaries; an iterator implementation
  is roughly a day of work each. The wider "no LATM/AC-3 frame
  parsers" entry above tracks the spec-side deferral; this entry is
  the codec-stats-side mirror.
- **Trigger to revisit:** A consumer asks for per-frame counters or
  frame-aligned dispatch on LATM/AC-3 audio. Once added, the `Audio`
  variant of `StreamCodecStats` (shipped in plan #68) automatically
  populates `frames` for those PIDs; today LATM/AC-3 PIDs return
  `Some(StreamCodecStats::Unknown)` via the codec-stats fallback.
- **Scope when added:** Wire the new iterators into the demuxer's
  per-PID audio counter-bump path in the same shape as the existing
  MP2 + AAC-ADTS bumps; the `StreamCodecStats::Audio { frames }`
  variant absorbs the new counts without an ABI change.

## Subtitle codec-specific stats

- **Status:** Deferred. The codec-stats surface shipped in plan #68
  covers Video / KLV / Audio kinds; subtitle PIDs surface as
  `StreamCodecStats::Unknown`.
- **Why deferred:** Low signal value. The codecs covered by the
  subtitle carriage plan (DVB-Subtitling, DVB-Teletext, CEA-708,
  WebVTT-in-TS) don't have meaningful per-segment counts distinct
  from the existing unified `items` counter on `StreamStats`. No
  consumer has asked for them.
- **Trigger to revisit:** A consumer asks for e.g. CEA-708
  caption-frame counts, DVB-sub region-update counts, WebVTT
  cue-emit counts, or DVB-teletext page-update counts.
- **Scope when added:** Add a `Subtitle { segments: u64 }` (or
  per-codec field names if the consumer asks for a finer breakdown)
  variant to `StreamCodecStats`. The `#[non_exhaustive]` enum makes
  this additive without a major bump; wire the bump-site at
  `Demuxer::emit_subtitle` alongside the existing `stats_per_stream`
  bump.

## Deep typed-time migration (arithmetic API design + internal sweep + signed-PCR delta type)

- **Status:** All **public** Rust APIs ship with `Pts90khz` as of Wave 2.1
  (plan `2026-05-18-typed-time-and-packet-constants.md`): `MuxSender::send_*`,
  `Muxer::push_*`, `pts_to_duration`, `DemuxEvent::{Sample,Metadata}.pts`, and
  `pairing::{VideoSample,KlvSample}.pts` all take/return `Pts90khz`. Internal
  arithmetic (private PES writers in `tst-core::mpegts::mux::pes`, private
  `psi_due`/`pcr_due`/`maybe_emit_psi` helpers, demuxer's `last_pts_by_pid:
  HashMap<u16, i64>` and `last_pcr_27mhz: Option<u64>` state, pairing engines
  that do `.as_ticks()` once before arithmetic) remains raw `i64` / `u64`.
  `NonConformantIssue::PcrAnomaly.delta: i64` remains raw `i64` because it's
  a *signed* 27 MHz delta and the existing `Pcr27mhz(u64)` newtype cannot
  represent it.
- **Why deferred:** Sweeping the internal sites is mechanical (~8-12h after
  Wave 2.1 lands), but the arithmetic API on `Pts90khz` / `Pcr27mhz` is a real
  design question: what does `pts_a + duration` return? Does `pts_a - pts_b`
  give a typed `Duration90khz` or raw `i64`? What about 33-bit wrap-around
  (`pts + 1` near `2^33`)? Saturate, wrap, or check (and return
  `Option`/`Result`)? Same questions for a hypothetical `Pcr27mhzDelta(i64)`
  signed-delta type. Picking the wrong default poisons every consumer
  arithmetic site. The internal sweep becomes much cheaper if the arithmetic
  API ships first.
- **Trigger to revisit:** (a) Pre-stabilization API freeze (forces the
  decision), (b) a binding-author request for type-safe internal arithmetic
  in the FFI bridge layer, (c) a binding-author request for a typed signed
  PCR delta on `PcrAnomaly`, or (d) a confirmed-by-fuzzer arithmetic bug
  traced to a `pts_90khz: i64` / `pcr_27mhz: u64` site that typed arithmetic
  would have caught.
- **Scope when added:** (1) Design wrap-vs-saturate semantics on
  `Pts90khz::Add<i64>`, `Sub<Self> -> Duration90khz` (with type definition),
  `Add<Duration90khz>`. Same for `Pcr27mhz`. Decide whether `Duration90khz`
  is its own type or a re-purposed `core::time::Duration`. (2) Add
  `Pcr27mhzDelta(i64)` if signed-delta typing is wanted; migrate
  `PcrAnomaly.delta`. (3) Sweep internal sites listed above. (4) Drop the
  `.as_ticks()` calls in pairing engines (they become typed arithmetic
  directly). (5) Refresh `cargo public-api` baselines; expect intentional
  breaking deltas if `Pcr27mhzDelta` lands. (6) Decide whether the
  fixture-generator `tests/tools/gen_*.rs` migrate too (probably not —
  raw integers are ergonomic for test scaffolding).
- **Effort estimate:** ~4-6h API design + writeup, ~8-12h sweep.

## Python-side subtitle muxing

- **Status:** `tstrans.Muxer` does not expose `add_subtitle` because the
  Rust mux-side `SubtitleCodec` is a struct-variant enum (carrying
  per-codec configuration like language, page IDs, and ancillary
  descriptors) that the flat Python `SubtitleCodec` enum doesn't yet
  model. Demux-side subtitle decoding via `DemuxEvent.Subtitle` IS
  supported. The `Muxer.push_subtitle` / `push_subtitle_to` /
  `Muxer.subtitle_handles()` / `MuxerProgramConfigBuilder.stream_descriptors_for_subtitle`
  surfaces remain wired so they work as soon as the construction gap
  closes.
- **Why deferred:** A half-implemented mux-side API (the previous
  `add_subtitle` that always raised `NotImplementedError`) is worse than
  a missing one — users build against it, then break. Mirroring the
  Rust struct-variant `SubtitleCodec` in Python as a tagged-union /
  dataclass hierarchy is sizeable work that should land as its own
  focused plan, not as a placeholder.
- **Trigger to revisit:** A consumer with a concrete Python subtitle
  muxing use case provides the structured config schema (which
  codec(s), which fields per codec, descriptor emission expectations).
- **Scope when added:** (1) Model the Rust mux-side `SubtitleCodec`
  variants as Python dataclasses (e.g. `DvbSubtitling(language: bytes,
  composition_page_id: int, ancillary_page_id: int)`, `DvbTeletext(...)`,
  `Cea708Standalone`, `WebVttInTs`). (2) Add `add_subtitle(pid, codec)`
  back to `MuxerProgramConfigBuilder` accepting the structured form.
  (3) Round-trip test against demux output. (4) Update CHANGELOG +
  binding-authors doc.

## macOS Intel (x86_64) JVM native library

- **Status:** The JVM fat JAR (`tstrans-jvm`) bundles native libraries
  for four platforms — linux-x86_64, linux-aarch64, macos-aarch64
  (Apple Silicon), and windows-x86_64. macOS Intel (`macos-x86_64`) is
  not built or bundled.
- **Why deferred:** GitHub's `macos-13` (Intel) runners are scarce and
  winding down — a build job for it sat queued 40+ minutes waiting for a
  runner, blocking the `assemble` job (a queued best-effort matrix entry
  never reaches `continue-on-error`, so it stalls the train rather than
  being skipped). The gating `ci.yml` workflow does not build macos-x86_64
  either. Apple is phasing out Intel Macs, and JVM consumers are now
  overwhelmingly on Apple Silicon.
- **Trigger to revisit:** A consumer needs the binding on an Intel Mac
  JVM, or GitHub macOS-Intel runner availability stops being a problem.
- **Scope when added:** Re-add a `macos-x86_64` / `macos-13` entry to the
  `jvm-jar.yml` build matrix as a **separate job** outside the gating set
  (so a scarce runner can't block `assemble`), include its lib in the
  staging download, and add `macos-x86_64` to the `NativeLoader` /
  `build.gradle.kts` triple set. Until then, Intel-Mac users build the
  cdylib from source (`cargo build --release -p tst-jni`).
