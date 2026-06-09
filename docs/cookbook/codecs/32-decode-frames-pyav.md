# Recipe 32: Decode video frames in-memory with PyAV (Python)

> **When to use this:** You're demuxing a `.ts` with `tstrans` (for KLV, timing, or
> transmux) and also want the decoded frames — without re-opening the file in OpenCV
> or `av.open()` and time-aligning two separate readers. Optionally you want to
> process the elementary stream (filter NAL units, decode only keyframes) before the
> decoder sees it.

> **Note:** This is one of the few **Python** recipes — it integrates
> [PyAV](https://pyav.org) (the FFmpeg Python binding: `pip install av`), which is
> *not* a `tstrans` dependency. The rest of the cookbook is Rust.

> **Related:**
> - [languages/python.md](/docs/languages/python.md) — the `tstrans` Python surface
> - [guides/mpegts-demux.md](/docs/guides/mpegts-demux.md) — raw-first `Video` events + opt-in `.parse()`
> - [Recipe 18](18-reconstitute-annex-b.md) — Annex-B reconstruction (Rust)

Under the raw-first model, `DemuxEvent.Video.raw` is the exact encoded Annex-B access
unit. Feed those bytes straight to a PyAV decoder `CodecContext` — no container, no
file — and you get decoded frames from the **same demux pass** that yields your KLV,
already PTS-aligned. That replaces the common two-reader setup (`tstrans` for metadata
+ OpenCV/`av.open()` for frames, then align by timestamp).

## Decode straight from `ev.raw`

```python
import av
from tstrans import io as tio, klv
from tstrans.mpegts import DemuxEvent, VideoCodec

dec = av.codec.CodecContext.create("h264", "r")   # "hevc"=H.265, "av1"=AV1, "vvc"=H.266 (exp.)
for ev in tio.parse_file("input.ts"):
    if isinstance(ev, DemuxEvent.Video):
        for pkt in dec.parse(bytes(ev.raw)):       # parser finds NAL boundaries + in-band SPS/PPS
            for frame in dec.decode(pkt):
                img = frame.to_ndarray(format="bgr24")   # HxWx3 uint8, OpenCV-compatible
                # ev.pts (90 kHz) is this frame's authoritative timestamp
    elif isinstance(ev, DemuxEvent.Klv):
        ls = klv.decode_uas_datalink(ev.payload)   # PTS-aligned to the frames above

# Flush the decoder's reorder buffer at end-of-stream:
for pkt in dec.parse(b""):
    for frame in dec.decode(pkt):
        ...
for frame in dec.decode(None):
    ...
```

## Process the elementary stream first (parsed NAL units)

To filter or modify the stream before decoding, use the opt-in parse (`ev.parse()`),
manipulate the NAL list, then reconstruct Annex-B. `parse()` strips each NAL's 1-byte
header (H.264), so re-add it when rebuilding:

```python
def nal_to_annexb_h264(n):
    hdr = ((n.ref_idc & 0x3) << 5) | (n.nal_type & 0x1f)   # the byte parse() stripped
    return b"\x00\x00\x00\x01" + bytes([hdr]) + bytes(n.payload)

def au_from_nals(nals):
    return b"".join(nal_to_annexb_h264(n) for n in nals)

dec = av.codec.CodecContext.create("h264", "r")
for ev in tio.parse_file("input.ts"):
    if isinstance(ev, DemuxEvent.Video):
        nals = ev.parse()
        nals = [n for n in nals if n.nal_type != 6]    # e.g. drop SEI (type 6) before decode
        for pkt in dec.parse(au_from_nals(nals)):
            for frame in dec.decode(pkt):
                img = frame.to_ndarray(format="bgr24")
```

**Keyframe-only decode** — gate on the random-access flag (each IDR must carry in-band
SPS/PPS, which broadcast / STANAG 4609 streams typically repeat):

```python
for ev in tio.parse_file("input.ts"):
    if isinstance(ev, DemuxEvent.Video) and ev.random_access_indicator:
        for pkt in dec.parse(bytes(ev.raw)):
            for frame in dec.decode(pkt):
                ...   # one frame per keyframe
```

NAL-type cheatsheet (H.264): `1` = non-IDR slice, `5` = IDR, `6` = SEI, `7` = SPS,
`8` = PPS. (H.265 / H.266 use a 2-byte NAL header — reconstruct both bytes from
`nal_type` + `layer_id` + `temporal_id_plus1`.)

## Gotchas

- **Flush at EOF.** The decoder buffers (B-frame reorder) — it will not emit a frame
  for every packet inline. Flush with `dec.parse(b"")` then `dec.decode(None)`.
- **Use `ev.pts`, not loop position.** Packets fed without a container have
  `frame.pts is None`. Carry `ev.pts` yourself. With B-frames, decode order ≠ display
  order; PyAV emits in display (PTS) order after reordering — pair frames to metadata
  by PTS, not by iteration index.
- **`ev.raw` vs `ev.parse()`.** For a straight decode, feed `ev.raw` (cheapest, exact).
  Use the parsed path only when you need to inspect / modify NAL units first — the
  reconstructed bytes decode identically but are not byte-identical to `ev.raw`
  (start-code widths can differ).
- **Codec.** Match `ev.codec`: `h264` / `hevc` (H.265) / `av1`; H.266 (`vvc`) decode is
  experimental in current FFmpeg. SPS/PPS are picked up from the in-band Annex-B
  headers — no `extradata` setup needed.

This recipe is Python-only (PyAV); there is no Rust `tst-examples` program for it. See
[languages/python.md](/docs/languages/python.md) for the full `tstrans` Python surface.
