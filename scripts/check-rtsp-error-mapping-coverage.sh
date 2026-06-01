#!/usr/bin/env bash
# 23rd bash ratchet (Phase 4 Stage 1).
#
# Verifies that every RtspError, MountError, and RtspServerError variant
# from tst-rtp has an explicit match arm in
# crates/tst-c-core/src/error.rs, and that the three converter functions
# (rtsp_error_to_code, mount_error_to_code, rtsp_server_error_to_code)
# exist.
#
# Each enum is `#[non_exhaustive]`, so Rust requires a `_ =>` wildcard when
# matching from outside tst-rtp. This ratchet provides the compile-time
# exhaustiveness check that Rust cannot: if a new tst-rtp variant is added
# without a corresponding explicit arm here, this script exits non-zero.
#
# Portable bash 3.2+ style (no mapfile/readarray) per
# feedback_bash_ratchets_macos_portability.md.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

ERR_FILE="$ROOT/crates/tst-c-core/src/error.rs"
RTSP_ERR="$ROOT/crates/tst-rtp/src/error.rs"

# ----------------------------------------------------------------------
# Helper: extract enum variant names from a given enum block in a file.
# Args: <file> <enum_name>
# Prints one variant name per line.
# ----------------------------------------------------------------------
extract_variants() {
    local file="$1"
    local enum_name="$2"
    awk -v name="$enum_name" '
        $0 ~ ("pub enum " name "[ {]") { in_block = 1; next }
        in_block && /^[{}]/ { in_block = 0 }
        in_block && /^    [A-Z][A-Za-z0-9]*[ ({,]/ {
            v = $0
            sub(/^[[:space:]]+/, "", v)
            sub(/[ ({,].*$/, "", v)
            print v
        }
    ' "$file"
}

# ----------------------------------------------------------------------
# Helper: extract the body of a named function from a file.
# Captures from the `fn <name>` line through the matching closing `}`.
# ----------------------------------------------------------------------
extract_fn_body() {
    local file="$1"
    local fn_name="$2"
    awk -v fn="$fn_name" '
        $0 ~ ("fn " fn) { in_fn = 1; depth = 0 }
        in_fn {
            print
            for (i = 1; i <= length($0); i++) {
                c = substr($0, i, 1)
                if (c == "{") depth++
                if (c == "}") {
                    depth--
                    if (depth == 0) { in_fn = 0; break }
                }
            }
        }
    ' "$file"
}

rc=0

# ----------------------------------------------------------------------
# 1. Extract RtspError variants and check rtsp_error_to_code coverage.
# ----------------------------------------------------------------------
rtsp_variants=()
while IFS= read -r v; do
    rtsp_variants+=("$v")
done < <(extract_variants "$RTSP_ERR" "RtspError")

if [ "${#rtsp_variants[@]}" -eq 0 ]; then
    echo "FAIL: extracted 0 RtspError variants — awk pattern may have drifted from $RTSP_ERR" >&2
    exit 1
fi

if ! grep -q "fn rtsp_error_to_code" "$ERR_FILE"; then
    echo "FAIL: rtsp_error_to_code converter missing from $ERR_FILE" >&2
    exit 1
fi

fn_body=$(extract_fn_body "$ERR_FILE" "rtsp_error_to_code")

missing_rtsp=()
for v in "${rtsp_variants[@]}"; do
    # Match the variant name followed by word-boundary characters:
    # unit variant: `Timeout =>`, tuple: `Io(_)`, struct: `Protocol { .. }`.
    if ! echo "$fn_body" | grep -qE "[[:space:]]${v}[[:space:](_{,]"; then
        missing_rtsp+=("$v")
    fi
done

if [ "${#missing_rtsp[@]}" -ne 0 ]; then
    echo "FAIL: RtspError variants missing explicit arm in rtsp_error_to_code ($ERR_FILE):" >&2
    for v in "${missing_rtsp[@]}"; do echo "  - $v" >&2; done
    echo "" >&2
    echo "Add an explicit arm for each variant in rtsp_error_to_code." >&2
    rc=1
fi

# ----------------------------------------------------------------------
# 2. Extract MountError variants and check mount_error_to_code coverage.
# ----------------------------------------------------------------------
mount_variants=()
while IFS= read -r v; do
    mount_variants+=("$v")
done < <(extract_variants "$RTSP_ERR" "MountError")

if [ "${#mount_variants[@]}" -eq 0 ]; then
    echo "FAIL: extracted 0 MountError variants — awk pattern may have drifted from $RTSP_ERR" >&2
    exit 1
fi

if ! grep -q "fn mount_error_to_code" "$ERR_FILE"; then
    echo "FAIL: mount_error_to_code converter missing from $ERR_FILE" >&2
    rc=1
fi

fn_body=$(extract_fn_body "$ERR_FILE" "mount_error_to_code")

missing_mount=()
for v in "${mount_variants[@]}"; do
    if ! echo "$fn_body" | grep -qE "[[:space:]]${v}[[:space:](_{,]"; then
        missing_mount+=("$v")
    fi
done

if [ "${#missing_mount[@]}" -ne 0 ]; then
    echo "FAIL: MountError variants missing explicit arm in mount_error_to_code ($ERR_FILE):" >&2
    for v in "${missing_mount[@]}"; do echo "  - $v" >&2; done
    echo "" >&2
    echo "Add an explicit arm for each variant in mount_error_to_code." >&2
    rc=1
fi

# ----------------------------------------------------------------------
# 3. Extract RtspServerError variants + check rtsp_server_error_to_code.
# ----------------------------------------------------------------------
server_variants=()
while IFS= read -r v; do
    server_variants+=("$v")
done < <(extract_variants "$RTSP_ERR" "RtspServerError")

if [ "${#server_variants[@]}" -eq 0 ]; then
    echo "FAIL: extracted 0 RtspServerError variants — awk pattern may have drifted from $RTSP_ERR" >&2
    exit 1
fi

if ! grep -q "fn rtsp_server_error_to_code" "$ERR_FILE"; then
    echo "FAIL: rtsp_server_error_to_code converter missing from $ERR_FILE" >&2
    rc=1
fi

fn_body=$(extract_fn_body "$ERR_FILE" "rtsp_server_error_to_code")

missing_server=()
for v in "${server_variants[@]}"; do
    if ! echo "$fn_body" | grep -qE "[[:space:]]${v}[[:space:](_{,]"; then
        missing_server+=("$v")
    fi
done

if [ "${#missing_server[@]}" -ne 0 ]; then
    echo "FAIL: RtspServerError variants missing explicit arm in rtsp_server_error_to_code ($ERR_FILE):" >&2
    for v in "${missing_server[@]}"; do echo "  - $v" >&2; done
    echo "" >&2
    echo "Add an explicit arm for each variant in rtsp_server_error_to_code." >&2
    rc=1
fi

# ----------------------------------------------------------------------
# 4. Summary.
# ----------------------------------------------------------------------
if [ "$rc" -eq 0 ]; then
    echo "PASS: rtsp-error-mapping-coverage (RtspError: ${#rtsp_variants[@]} variants, MountError: ${#mount_variants[@]} variants, RtspServerError: ${#server_variants[@]} variants)"
fi

exit "$rc"
