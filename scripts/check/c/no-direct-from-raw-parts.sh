#!/usr/bin/env bash
# All C-ABI byte-slice construction that takes a caller-supplied length must
# route through ffi_slice/ffi_slice_mut (bindings/c/core/src/ffi_slice.rs),
# which enforce the (NULL,0) and len<=isize::MAX preconditions.
# A direct from_raw_parts on a caller-supplied length reintroduces the
# NEW-CABI-1 soundness gap.
#
# Exclusions:
# - src/ffi_slice.rs  — the helpers themselves contain the from_raw_parts calls.
# - src/event.rs      — test-only uses that read arena-internal (not caller-supplied)
#                       lengths; these are in #[cfg(test)] blocks only.
# - src/config/streams.rs — uses from_raw_parts with a fixed length of 3 for
#                           ISO 639-2 language codes; not a caller-supplied length.
set -euo pipefail
cd "$(dirname "$0")/../../.."
hits=$(grep -rn 'from_raw_parts' bindings/c/core/src --include='*.rs' \
  | grep -v 'src/ffi_slice.rs' \
  | grep -v 'src/event.rs' \
  | grep -v 'src/config/streams.rs' \
  || true)
if [ -n "$hits" ]; then
  echo "FAIL: direct from_raw_parts outside allowed files in bindings/c/core/src/:"
  echo "$hits"
  exit 1
fi
echo "OK: no direct from_raw_parts on caller-supplied lengths in bindings/c/core/src/"
