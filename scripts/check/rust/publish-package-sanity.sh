#!/usr/bin/env bash
# publish-package-sanity: crates.io packaging guards.
#   1. bundled vendor trees are checked out (cargo#8635: packaging an
#      empty submodule silently ships a broken crate)
#   2. no nested Cargo.toml inside a bundled tree (cargo#8597: silently
#      drops the whole subtree; `include` does NOT override)
#   3. package file lists contain the native build entry points + licenses,
#      and the deliberately-excluded subtrees (nested submodules / unused
#      bundled fallback copies) stay excluded
#   4. compressed package size stays under 9 MiB (crates.io limit: 10)
# Linux-only: runs cargo package (heavy) and GNU stat.
set -euo pipefail
[ "$(uname -s)" = "Linux" ] || { echo "publish-package-sanity: SKIP (linux-only)"; exit 0; }
cd "$(dirname "$0")/../../.."

fail=0
declare -A TREES=(
  [crates/srt-sys/vendor/srt]=CMakeLists.txt
  [crates/rist-sys/vendor/librist]=meson.build
  [crates/mbedtls-src/vendor/mbedtls]=CMakeLists.txt
)
for tree in "${!TREES[@]}"; do
  [ -f "$tree/${TREES[$tree]}" ] || { echo "FAIL: $tree not checked out (missing ${TREES[$tree]})"; fail=1; }
  nested=$(find "$tree" -name Cargo.toml | head -1)
  [ -z "$nested" ] || { echo "FAIL: nested manifest $nested would drop its subtree from the package"; fail=1; }
done

# (pkg, sentinel-in-list, ...) — one sentinel per required subtree + license.
check_list() {
  local pkg=$1; shift
  local list
  list=$(cargo package --list -p "$pkg" --allow-dirty 2>/dev/null) || { echo "FAIL: cargo package --list $pkg"; fail=1; return; }
  for f in "$@"; do
    grep -qxF "$f" <<<"$list" || { echo "FAIL: $pkg package misses $f"; fail=1; }
  done
}
# Inverse of check_list: fail if any packaged file path starts with a
# supposedly-excluded prefix (nested dev-tool submodules / bundled-but-never-
# compiled fallback sources — see crates/srt-sys/Cargo.toml and
# crates/rist-sys/Cargo.toml `exclude` lists for why each is unreferenced by
# the actual build).
check_absent() {
  local pkg=$1; shift
  local list
  list=$(cargo package --list -p "$pkg" --allow-dirty 2>/dev/null) || { echo "FAIL: cargo package --list $pkg"; fail=1; return; }
  for prefix in "$@"; do
    # Literal-prefix match via bash's own quoted-glob idiom (not grep): quoting
    # "$prefix" inside the `[[ ... ]]` pattern makes ITS characters literal,
    # while the lone trailing unquoted `*` is the only glob metacharacter in
    # play. This sidesteps the same fixed-vs-regex risk `check_list` had
    # (a `.` in a path being read as "any character") without needing a
    # `grep -F` + `^` combination that would fight each other (`-F` treats a
    # literal `^` as a literal caret, not an anchor).
    local count=0 line
    while IFS= read -r line; do
      [[ $line == "$prefix"* ]] && count=$((count + 1))
    done <<<"$list"
    [ "$count" -eq 0 ] || { echo "FAIL: $pkg package unexpectedly ships $count file(s) under $prefix"; fail=1; }
  done
}
check_list tstrans-srt-sys \
  vendor/srt/CMakeLists.txt vendor/srt/LICENSE \
  vendor/srt/srtcore/core.cpp vendor/srt/haicrypt/haicrypt.h
check_absent tstrans-srt-sys \
  vendor/srt/submodules/
check_list tstrans-rist-sys \
  vendor/librist/meson.build vendor/librist/COPYING \
  vendor/librist/src/rist.c vendor/librist/contrib/lz4/lz4.c
check_absent tstrans-rist-sys \
  vendor/librist/contrib/mbedtls/library/
check_list tstrans-mbedtls-src \
  vendor/mbedtls/CMakeLists.txt vendor/mbedtls/LICENSE \
  vendor/mbedtls/library/ssl_tls.c

# Compressed package size, per crate. `cargo package --no-verify` on
# tstrans-srt-sys/tstrans-rist-sys cannot succeed until tstrans-mbedtls-src is
# actually live on crates.io: Cargo's packaging step queries the REAL
# crates.io index to confirm every path-dependency-with-a-version-key
# resolves there, independent of --no-verify (build-skip only) and
# independent of [patch.crates-io] (publish-readiness checks bypass patches
# by design). Verified in task-5-report.md / task-6-report.md of the
# crates-io-packaging arc. Until tstrans-mbedtls-src's first publish, fall
# back to reconstructing the exact `cargo package --list` file set with a
# manual tar+gzip — this self-heals the moment the real `cargo package`
# starts succeeding (that branch stops firing and the real artifact is used).
manual_package_size() {
  local pkg=$1 crate_dir=$2
  local list scratch size
  list=$(cargo package --list -p "$pkg" --allow-dirty 2>/dev/null) || return 1
  scratch=$(mktemp -d)
  while IFS= read -r f; do
    [ -f "$crate_dir/$f" ] || continue
    mkdir -p "$scratch/$(dirname "$f")"
    cp "$crate_dir/$f" "$scratch/$f"
  done <<<"$list"
  size=$(tar -C "$scratch" -cf - . | gzip -9 | wc -c)
  rm -rf "$scratch"
  echo "$size"
}

declare -A PKG_DIR=(
  [tstrans-srt-sys]=crates/srt-sys
  [tstrans-rist-sys]=crates/rist-sys
  [tstrans-mbedtls-src]=crates/mbedtls-src
)
for pkg in tstrans-srt-sys tstrans-rist-sys tstrans-mbedtls-src; do
  dir="${PKG_DIR[$pkg]}"
  out=$(cargo package --no-verify -p "$pkg" --allow-dirty 2>&1) && rc=0 || rc=$?
  if [ "$rc" -eq 0 ]; then
    crate=$(ls -t target/package/${pkg}-*.crate | head -1)
    size=$(stat -c%s "$crate")
  elif grep -q 'no matching package named `tstrans-mbedtls-src`' <<<"$out"; then
    size=$(manual_package_size "$pkg" "$dir") || { echo "FAIL: manual size reconstruction for $pkg"; fail=1; continue; }
    echo "NOTE: $pkg pre-first-publish fallback size estimate = $size bytes (manual tar+gzip of the package file list; will use the real cargo-package artifact once tstrans-mbedtls-src is live on crates.io)"
  else
    echo "FAIL: cargo package $pkg"
    echo "$out" >&2
    fail=1
    continue
  fi
  [ "$size" -le $((9 * 1024 * 1024)) ] || { echo "FAIL: $pkg is $size bytes (> 9 MiB budget)"; fail=1; }
done

[ "$fail" -eq 0 ] && echo "publish-package-sanity: OK"
exit "$fail"
