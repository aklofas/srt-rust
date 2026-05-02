# Deferred features

Things deliberately out of scope today with a clear path back if they
become load-bearing. Each entry records the reason it was deferred and
the trigger that would unblock it.

## `mpegts::demux` — receiver-side TS demuxer in the Rust core

- **Status:** Not implemented.
- **Why deferred:** Receiver-side TS demux is well-served by FFmpeg /
  Bento4 / Media3 / AVFoundation. A Rust-native demux would duplicate
  that work without producing a better demux for the consumers that
  exist today.
- **Trigger to revisit:** A consumer that can't use FFmpeg or Bento4
  for footprint, licensing, or embedded-target reasons.

## Audio carriage in `mpegts::mux`

- **Status:** Not implemented; muxer carries video + KLV only.
- **Why deferred:** Gimbaled-platform streams typically deliver video
  plus KLV with no audio track. Adding audio speculatively means
  guessing codec, framing, and PTS-sync questions that no shipping
  consumer is asking.
- **Trigger to revisit:** A consumer ships requiring synchronized
  audio in the same TS as the video and KLV.

## Subtitle, caption, and auxiliary-data channels in `mpegts::mux`

- **Status:** Not implemented.
- **Why deferred:** Same situation as audio — no shipping consumer
  asks for them. The shapes also diverge enough across codecs (DVB
  subtitling, CEA-608/708, teletext, ARIB) that "generic auxiliary
  track" is the wrong abstraction.
- **Trigger to revisit:** A specific channel type asked for by a
  consumer, designed against that channel's actual semantics.

## `pipeline::receiver` — receive-side composition convenience

- **Status:** Not implemented.
- **Why deferred:** Depends on `mpegts::demux`. Without a Rust-native
  demuxer, the receive side is whatever the binding language assembles
  itself — SRT bytes feed into the consumer's TS demuxer of choice,
  which feeds into a KLV decoder.
- **Trigger to revisit:** Ships when `mpegts::demux` ships.

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

## Linger tuning (`SRTO_LINGER`)

- **Status:** Not exposed; library uses a sensible internal value.
- **Why deferred:** Live streams use TLPKTDROP, so a long linger
  window on shutdown is a foot-gun rather than a feature. A live
  consumer doesn't need to tune linger.
- **Trigger to revisit:** A non-live (file-mode) consumer that needs
  a real linger window on close.

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

## Multi-stream `mpegts::mux`

- **Status:** Single video PID + single KLV PID per output TS.
  `Config::streams: Vec<StreamSpec>` is multi-stream-shaped from day
  one; `Config::validate` enforces the cap.
- **Why deferred:** Current consumers run single-program TS. Lifting
  the cap means writing and testing multi-program behaviour without a
  driving use case.
- **Trigger to revisit:** A consumer needs multiple video PIDs or
  multiple KLV PIDs in one TS. The cap is the only thing that needs
  to lift; the API shape is already right.

## Rustdoc lift to docs.rs via `#![doc = include_str!(...)]`

- **Status:** Not implemented; the markdown guides under `docs/` are
  read directly from the repo.
- **Why deferred:** This crate is not yet on crates.io; until it
  publishes, docs.rs has nothing to auto-build. The markdown guides
  are written rustdoc-compatibly so the future lift is mechanical.
- **Trigger to revisit:** Immediately before publishing to crates.io.
