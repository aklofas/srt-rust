# shellcheck shell=bash
# _lib.sh — shared helpers for the Apple build scripts (build-ios.sh,
# make-xcframework.sh). Source this; do not run it directly.
#
# MUST run on macOS with Xcode (Apple SDKs + libtool + lipo + xcodebuild).
# Apple cross-compilation cannot run off a Mac.

# Repo workspace root (…/ts-transformer/ts-transformer), derived from this
# file's location (scripts/apple/_lib.sh → ../.. ).
_APPLE_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${_APPLE_LIB_DIR}/../.." && pwd)"

PROFILE="${PROFILE:-release}"
# `srt` = encrypted MPEG-TS-over-SRT (pulls the vendored mbedTLS via the
# tst-c-core forward) — the PoC's scope. rtp/udp/tcp/hls/rist are opt-in; rtp
# (for the "RTSPS later" path) can be added here once its iOS build is proven.
FEATURES="${FEATURES:-srt}"
# Where merged per-slice static libs land (git-ignored build output).
APPLE_OUT="${APPLE_OUT:-${WORKSPACE_ROOT}/target/apple}"

_cargo_profile_flag() { [ "$PROFILE" = "release" ] && echo "--release" || echo ""; }

die() { echo "error: $*" >&2; exit 1; }

require_macos() {
  [ "$(uname -s)" = "Darwin" ] || die "must run on macOS (Apple cross-compilation needs the Xcode SDKs)"
  command -v xcrun     >/dev/null || die "xcrun not found — install Xcode + command-line tools"
  command -v libtool   >/dev/null || die "libtool not found"
  command -v lipo      >/dev/null || die "lipo not found"
}

# build_slice <rust-triple> <slice-name>
# Cross-compiles tst-c for the triple, then merges the Rust staticlib
# (libtstrans.a) with the native static libs it links against (libsrt.a +
# mbedTLS's libmbedtls/x509/crypto.a) — which a Rust `staticlib` does NOT
# bundle — into one self-contained ${APPLE_OUT}/<slice>/libtstrans.a.
build_slice() {
  local triple="$1" slice="$2"
  local out_dir="${APPLE_OUT}/${slice}"
  echo "──────── slice ${slice} (${triple}) ────────"

  rustup target add "$triple" >/dev/null 2>&1 || true

  # SRT_FORCE_VENDORED=1 → build the vendored libsrt + mbedTLS (the only way to
  # get iOS-targeted native libs; there is no system libsrt to pkg-config on an
  # iOS SDK). srt-sys's build.rs applies the apple-ios cmake settings
  # (CMAKE_SYSTEM_NAME=iOS etc.) automatically for the *-apple-ios* triples.
  ( cd "$WORKSPACE_ROOT" && SRT_FORCE_VENDORED=1 \
      cargo build -p tst-c --target "$triple" $(_cargo_profile_flag) --features "$FEATURES" ) \
    || die "cargo build failed for $triple"

  local target_dir="${WORKSPACE_ROOT}/target/${triple}/${PROFILE}"
  # Apple's `staticlib` assembly garbage-collects upstream-rlib objects that the
  # leaf crate only re-exports (`pub use tst_c_core::*`) without an object-level
  # reference — so target/<triple>/<profile>/libtstrans.a comes out MISSING the
  # tst_c_core entry points on iOS (Linux bundles the whole graph, so it has all
  # 228+ tst_* symbols; iOS has zero). Rather than trust that incomplete
  # archive, reconstruct the complete static library from every compiled Rust
  # object — the project's rlibs (leaf + workspace + third-party) AND the
  # target's precompiled std rlibs — plus the native libs. That's the full set
  # rustc's staticlib *should* contain, assembled deterministically ourselves.
  local rust_lib="${target_dir}/libtstrans.a"
  [ -f "$rust_lib" ] || die "expected Rust staticlib not found: $rust_lib"

  # 1. Collect all Rust archives: the project's compiled rlibs + the target's
  #    precompiled std rlibs (libstd/libcore/liballoc/libcompiler_builtins/…).
  local std_libdir
  std_libdir="$(cd "$WORKSPACE_ROOT" && rustc --target "$triple" --print target-libdir 2>/dev/null || true)"
  local -a rlibs=()
  local r
  while IFS= read -r r; do rlibs+=("$r"); done < <(
    { find "${target_dir}/deps" -name '*.rlib' 2>/dev/null
      [ -n "$std_libdir" ] && find "$std_libdir" -name '*.rlib' 2>/dev/null; } | sort -u
  )
  [ "${#rlibs[@]}" -ge 1 ] || die "no .rlib archives found under ${target_dir}/deps"

  # 2. The native .a's from the vendored cmake install trees.
  local -a native_libs=()
  local a
  while IFS= read -r a; do native_libs+=("$a"); done < <(
    find "${target_dir}/build" \
      \( -name 'libsrt.a' -o -name 'libmbedtls.a' -o -name 'libmbedx509.a' -o -name 'libmbedcrypto.a' \) \
      2>/dev/null | sort -u
  )
  [ "${#native_libs[@]}" -ge 1 ] || die "no native static libs (libsrt.a / libmbed*.a) found under ${target_dir}/build — did the vendored cmake build run?"

  # 3. Extract each rlib's object files into its own subdir (object names collide
  #    across rlibs otherwise), skipping rlib metadata members (*.rmeta / lib.rmeta).
  local objroot; objroot="$(mktemp -d "${APPLE_OUT%/}/.objs-${slice}.XXXXXX")"
  local -a objs=()
  local i=0 o
  for r in "${rlibs[@]}"; do
    local d="${objroot}/$((i++))"
    mkdir -p "$d"
    ( cd "$d" && ar x "$r" 2>/dev/null || true )
    rm -f "$d"/*.rmeta "$d"/lib.rmeta 2>/dev/null || true
    while IFS= read -r o; do objs+=("$o"); done < <(find "$d" -name '*.o' 2>/dev/null)
  done
  [ "${#objs[@]}" -ge 1 ] || die "extracted zero object files from the rlibs for $slice"

  mkdir -p "$out_dir"
  # libtool -static merges archives + loose objects into one; -D = deterministic
  # (no timestamps). Duplicate members are tolerated (first definition wins).
  libtool -static -D -o "${out_dir}/libtstrans.a" "${objs[@]}" "${native_libs[@]}" 2>/dev/null \
    || libtool -static -o "${out_dir}/libtstrans.a" "${objs[@]}" "${native_libs[@]}" \
    || die "libtool merge failed for $slice"
  rm -rf "$objroot"

  echo "  merged: ${out_dir}/libtstrans.a"
  echo "  inputs: ${#objs[@]} rust objects (from ${#rlibs[@]} rlibs) + $(printf '%s ' "${native_libs[@]##*/}")"
  # Sanity: architecture + that our C ABI symbols and libsrt's are both present.
  lipo -info "${out_dir}/libtstrans.a" 2>/dev/null | sed 's/^/  arch: /' || true
  # `nm -g` lists external symbols; a defined C function shows as ` T _name`.
  # Match on the ` T ` type letter (LLVM and BSD nm agree on it) rather than
  # -U/-j, whose meaning differs across nm variants (that ambiguity made an
  # earlier `-gjU` check count zero on a lib that DID contain the symbols).
  local nm_out tst_syms srt_syms
  nm_out="$(nm -g "${out_dir}/libtstrans.a" 2>/dev/null || true)"
  tst_syms="$(printf '%s\n' "$nm_out" | grep -cE ' T _tst_' || true)"
  srt_syms="$(printf '%s\n' "$nm_out" | grep -cE ' T _srt_' || true)"
  echo "  symbols: tst_*=${tst_syms} srt_*=${srt_syms}"
  [ "${tst_syms}" -gt 0 ] || die "no _tst_* symbols in merged lib for $slice"
  [ "${srt_syms}" -gt 0 ] || die "no _srt_* symbols in merged lib for $slice (libsrt did not link)"
}

# stage_headers <dir> — copy the committed public header + modulemap into <dir>
# so `xcodebuild -create-xcframework -headers <dir>` picks them up.
# NOTE: the committed header is the `--features srt,rtp` surface (a superset);
# an `srt`-only slice contains the srt+core+st0601+codec symbols but not the
# rtp ones (declared-but-absent). The SRT PoC never calls the rtp entry points;
# rebuild with FEATURES="srt,rtp" once the rtp iOS build is exercised.
stage_headers() {
  local dir="$1"
  mkdir -p "$dir"
  cp "${WORKSPACE_ROOT}/bindings/c/include/tstrans.h"        "${dir}/tstrans.h"
  cp "${WORKSPACE_ROOT}/bindings/c/include/module.modulemap" "${dir}/module.modulemap"
}
