//! Shared bit-packing for opaque stream handles.
//!
//! Each `VideoStreamHandle` / `KlvStreamHandle` / `AudioStreamHandle` /
//! `SubtitleStreamHandle` wraps a `u32` containing
//! `(program_index << PROGRAM_SHIFT) | within_index`. The four types
//! had byte-identical `pack` / `unpack` bodies before this substrate;
//! here they share one.
//!
//! `MAX_PROGRAMS` (16) and the per-kind stream caps (also 16) fit in
//! 4 bits each, so the layout is `<< 4` with `& 0x0F` masks. Higher
//! bits are unused but reserved if caps grow.

const PROGRAM_SHIFT: u32 = 4;
const PROGRAM_MASK: u32 = 0x0F;
const WITHIN_MASK: u32 = 0x0F;

/// Pack `(program_index, within_index)` into a `u32` opaque handle.
///
/// Caller is responsible for keeping both indices within their respective
/// caps (`MAX_PROGRAMS`, `MAX_*_STREAMS_PER_PROGRAM`); `debug_assert!`s
/// here verify in debug. Release builds defensively `& WITHIN_MASK` the
/// `within` bits to avoid leaking a stray high bit into the program slot
/// if a caller (e.g. `from_raw` consumer at the C ABI) passes an
/// out-of-range value.
pub(crate) fn pack(program_index: usize, within_index: usize) -> u32 {
    debug_assert!(program_index < (1 << PROGRAM_SHIFT));
    debug_assert!(within_index < (1 << PROGRAM_SHIFT));
    ((program_index as u32) << PROGRAM_SHIFT) | (within_index as u32 & WITHIN_MASK)
}

/// Inverse of [`pack`]. Returns `(program_index, within_index)` with
/// both fields independently masked back into their 4-bit slots.
///
/// Trust-boundary callers (FFI re-wraps from a caller-provided `u32`)
/// must use [`try_unpack`] instead — `unpack` masks high bits silently,
/// so a forged value with the same low byte as a valid handle would
/// alias the wrong stream. `unpack` stays for in-process round-trips
/// where the input was produced by [`pack`] earlier in the same process.
pub(crate) fn unpack(packed: u32) -> (usize, usize) {
    let program = ((packed >> PROGRAM_SHIFT) & PROGRAM_MASK) as usize;
    let within = (packed & WITHIN_MASK) as usize;
    (program, within)
}

/// Validating inverse of [`pack`]. Returns `Some((program_index, within_index))`
/// only when the raw value has no bits set outside the documented 4-bit
/// program + 4-bit within slots. Returns `None` if any "reserved" upper
/// bit is set — this is the discriminator a forged FFI handle trips.
///
/// Use this at every trust boundary that rewraps a caller-provided `u32`
/// back into a typed handle (tst-c, tst-py, future tst-jni / tst-uniffi).
/// Plain [`unpack`] silently masks the high bits and would route the
/// payload to whatever valid stream the low byte happens to name.
pub(crate) fn try_unpack(packed: u32) -> Option<(usize, usize)> {
    const CANONICAL_MASK: u32 = (PROGRAM_MASK << PROGRAM_SHIFT) | WITHIN_MASK;
    if packed & !CANONICAL_MASK != 0 {
        return None;
    }
    let program = ((packed >> PROGRAM_SHIFT) & PROGRAM_MASK) as usize;
    let within = (packed & WITHIN_MASK) as usize;
    Some((program, within))
}
