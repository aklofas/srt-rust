#!/usr/bin/env bash
# Verify that every `pub unsafe extern "C" fn` in crates/tst-c/src/
# isolates panics across the C boundary. Three acceptable forms:
#
#   1. The body wraps work in `crate::panic::ffi_catch(...)` (the
#      open-path + builder-setter pattern).
#   2. The body uses `Handle::with_inner_mut(...)` or
#      `Handle::with_inner_ref(...)` — both already wrap their closure
#      in `catch_unwind` internally (see crates/tst-c/src/handle.rs).
#   3. The function name is in the trivially-infallible allowlist
#      below (constant returns with no internal locking, panics, or
#      allocation).
#
# Why: extension of plan #96 Wave F finding #9. The existing
# `check-lifecycle-ffi-catch-coverage.sh` only covers `_close` /
# `_cancel` entries — `tst_demux_config_free` slipped past that
# ratchet by being a `_free`. This ratchet enumerates *every*
# `pub unsafe extern "C" fn` in tst-c, so future entry points cannot
# accidentally bypass the panic-isolation policy.
#
# Cross-language unwinding past a C frame is undefined behavior under
# `panic="unwind"` and an abort with no last-error visibility under
# `panic="abort"`. See crates/tst-c/src/panic.rs for the contract.

set -euo pipefail

SRC_DIR="crates/tst-c/src"

# Allowlist of function names that are trivially infallible: no
# pointer dereferences, no allocations, no internal locks, no calls
# into tst-core. They return a process-lifetime constant (or a packed
# integer). Keep this list small and intentional.
ALLOWLIST=(
    "tst_get_abi_version_major"
    "tst_get_abi_version_minor"
    "tst_get_version_major"
    "tst_get_version_minor"
    "tst_get_version_patch"
    "tst_get_version_packed"
    "tst_get_version_string"
)

is_allowlisted() {
    local name="$1"
    for entry in "${ALLOWLIST[@]}"; do
        if [[ "$entry" == "$name" ]]; then
            return 0
        fi
    done
    return 1
}

# Step 1: enumerate every signature line. Bash 3.2-portable
# read-into-array pattern (no `mapfile`/`readarray`, no `declare -A`)
# per feedback_bash_ratchets_macos_portability.md.
ENTRIES=()
while IFS= read -r entry; do
    ENTRIES+=("$entry")
done < <(
    grep -rEn '^pub unsafe extern "C" fn tst_[a-zA-Z0-9_]+' \
        "$SRC_DIR" \
        --include='*.rs' \
    | sort
)

if [[ ${#ENTRIES[@]} -eq 0 ]]; then
    echo "FAIL: found 0 extern \"C\" entry points — grep pattern may have drifted from source layout"
    exit 1
fi

missing=0
checked=0
allowlisted=0

for entry in "${ENTRIES[@]}"; do
    file="${entry%%:*}"
    rest="${entry#*:}"
    lineno="${rest%%:*}"

    # Extract function name. Pattern: `pub unsafe extern "C" fn NAME(` or
    # `pub unsafe extern "C" fn NAME<` or `pub unsafe extern "C" fn NAME `.
    sig_line=$(sed -n "${lineno}p" "$file")
    fn_name=$(echo "$sig_line" \
        | sed -E 's/^pub unsafe extern "C" fn ([a-zA-Z0-9_]+).*/\1/')

    if [[ -z "$fn_name" ]]; then
        echo "FAIL: could not parse function name at ${file}:${lineno}"
        echo "      line: ${sig_line}"
        exit 1
    fi

    # Allowlist short-circuit.
    if is_allowlisted "$fn_name"; then
        allowlisted=$((allowlisted + 1))
        continue
    fi

    # Step 2: extract the function body. Rustfmt formats every public
    # function with the signature `pub unsafe extern "C" fn ...` at
    # column 0 and the closing brace `}` also at column 0. Read from
    # the signature line through the next `^}` line (inclusive).
    body=$(awk -v start="$lineno" '
        NR >= start {
            print
            if (NR > start && $0 == "}") {
                exit
            }
        }
    ' "$file")

    # Step 3: scan body for one of the panic-isolation patterns. The
    # `with_mux_publisher(` HLS helper (crates/tst-c/src/hls/mux_publisher.rs)
    # wraps its closure in crate::panic::ffi_catch internally — same contract
    # as Handle::with_inner_mut — so callers delegating through it are isolated.
    if echo "$body" | grep -qE 'crate::panic::ffi_catch\(|^\s*ffi_catch\(|\.with_inner_(mut|ref)\(|with_mux_publisher\('; then
        checked=$((checked + 1))
        continue
    fi

    echo "MISSING: ${fn_name} at ${file}:${lineno} has no ffi_catch / with_inner_mut / with_inner_ref / allowlist entry"
    missing=$((missing + 1))
done

if [[ $missing -gt 0 ]]; then
    echo
    echo "FAIL: $missing of ${#ENTRIES[@]} extern \"C\" entry points bypass panic isolation"
    echo
    echo "Fix options:"
    echo "  1. Wrap the body in crate::panic::ffi_catch(default, || { ... })"
    echo "  2. Route mutation through Handle::with_inner_mut(|inner| { ... })"
    echo "  3. If the function is provably infallible (constant return,"
    echo "     no allocation, no locking), add it to the ALLOWLIST in"
    echo "     scripts/check-extern-c-ffi-catch-coverage.sh"
    exit 1
fi

echo "OK: ${#ENTRIES[@]} extern \"C\" entry points wrap panic isolation"
echo "    ${checked} via ffi_catch / with_inner_{mut,ref}"
echo "    ${allowlisted} via allowlist (trivially infallible)"
