#!/usr/bin/env bash
# Compile-and-link every C example under bindings/c/examples/ against the
# all-features libtstrans cdylib with the flags the examples' own headers
# document (`-Wall -Werror`).
#
# Why this rail exists: the C examples are teaching code that consumers
# copy, but until 2026-09-06 nothing compiled them — CI only built the
# scenario adapter (`scenarios/run_scenarios.c`). A header change that
# renamed or removed a symbol, or a warning introduced by a newer gcc,
# would have gone unnoticed until a reader hit it. This closes that gap
# for every example in one pass.
#
# Scope: compile + link only. Examples are NOT run here — most need a live
# peer, a network port, or an input file, and the ones that don't
# (hello_world, version_check, the offline muxers) are exercised by hand
# per the recipes in bindings/c/examples/README.md.
#
# Build: the transport examples `#error` unless their `TST_HAS_<X>` macro is
# defined, so the header must come from an all-features build. We run
# `cargo build -p tst-c --all-features` ourselves — a warm no-op after the
# CI job's `cargo build --workspace --all-features` step and after the
# local pre-push runner's test-allfeatures phase, and the correct cold build
# otherwise.
#
# Linux-only: libtstrans.so is a Linux cdylib and the examples are Linux-only
# by build convention (see bindings/c/examples/README.md). On any other host
# the rail reports SKIP and exits 0 so the local rail sweep on macOS stays
# green; ci.yml additionally gates the step to the linux-x86_64 leg.

set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
    echo "examples-compile: SKIP (Linux-only rail; host is $(uname -s))"
    exit 0
fi

# Paths relative to ts-transformer/ workspace root (the directory holding
# Cargo.toml). The script may be invoked from anywhere.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-$(cd "$SCRIPT_DIR/../../.." && pwd)}"
cd "$WORKSPACE_ROOT"

EXAMPLES_DIR="bindings/c/examples"
LIB_DIR="target/debug"
# The header must be the one cbindgen emitted for THIS build: the committed
# bindings/c/include/tstrans.h is the `--features srt,rtp` rendering (see
# bindings/c/tests header_drift), so it defines only TST_HAS_SRT/RTP and the
# udp/tcp/hls/rist examples would trip their #error guards against it.
INCLUDE_DIR="$LIB_DIR/include"
CC="${CC:-cc}"

echo "examples-compile: building libtstrans (all features)..."
SRT_FORCE_VENDORED="${SRT_FORCE_VENDORED:-1}" \
RIST_FORCE_VENDORED="${RIST_FORCE_VENDORED:-1}" \
    cargo build -p tst-c --all-features --quiet

if [ ! -f "$LIB_DIR/libtstrans.so" ] || [ ! -f "$INCLUDE_DIR/tstrans.h" ]; then
    echo "examples-compile: FAIL — $LIB_DIR/libtstrans.so or $INCLUDE_DIR/tstrans.h not produced" >&2
    exit 1
fi

# -Wall -Werror: the flags every example's header documents. Examples that
# spawn threads link pthread explicitly (glibc >= 2.34 folds it into libc,
# older toolchains still need the flag; harmless either way).
CFLAGS=(-I "$INCLUDE_DIR" -Wall -Werror)
LDFLAGS=(-L "$LIB_DIR" -ltstrans -lpthread)

OUT_DIR="$(mktemp -d)"
trap 'rm -rf "$OUT_DIR"' EXIT

sources=()
while IFS= read -r f; do sources+=("$f"); done < <(find "$EXAMPLES_DIR" -name '*.c' | sort)

if [ "${#sources[@]}" -eq 0 ]; then
    echo "examples-compile: FAIL — no .c files found under $EXAMPLES_DIR" >&2
    exit 1
fi

failures=()
for src in "${sources[@]}"; do
    name="$(basename "$src" .c)"
    if ! "$CC" "${CFLAGS[@]}" -o "$OUT_DIR/$name" "$src" "${LDFLAGS[@]}"; then
        failures+=("$src")
    fi
done

if [ "${#failures[@]}" -ne 0 ]; then
    echo "examples-compile: FAIL — ${#failures[@]} of ${#sources[@]} example(s) did not compile+link:" >&2
    printf '  %s\n' "${failures[@]}" >&2
    exit 1
fi

echo "examples-compile: OK — ${#sources[@]} C examples compiled and linked with -Wall -Werror"
