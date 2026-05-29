#!/usr/bin/env bash
# Negative-case self-test for the error-mapping coverage drivers. Proves the
# drivers actually DETECT gaps (not just pass on a clean tree) before the old
# per-protocol clones are deleted. Hermetic: builds synthetic fixtures in a
# tmpdir, so it never depends on the real source tree.
set -uo pipefail
DIR="$(cd "$(dirname "$0")/.." && pwd)"          # scripts/ratchets
RUST="$DIR/run-rust-coverage.sh"
PY="$DIR/run-py-coverage.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail=0
expect() { # <desc> <want_rc> <cmd...>
    local desc="$1" want="$2"; shift 2
    "$@" >"$tmp/out" 2>&1; local got=$?
    if [[ "$got" == "$want" ]]; then
        echo "ok: $desc"
    else
        echo "FAIL: $desc (got rc=$got want=$want)"; sed 's/^/    /' "$tmp/out"; fail=1
    fi
}

# ---- Rust fixtures ---------------------------------------------------------
cat > "$tmp/enum.rs" <<'EOF'
pub enum FooErrorKind {
    Alpha = 1,
    Beta = 2,
}
EOF
cat > "$tmp/enum_ne.rs" <<'EOF'
#[non_exhaustive]
pub enum FooErrorKind {
    Alpha = 1,
    Beta = 2,
}
EOF
cat > "$tmp/arm_ok.rs" <<'EOF'
fn foo_error_to_code(k: FooErrorKind) -> i32 {
    match k {
        FooErrorKind::Alpha => 1,
        FooErrorKind::Beta => 2,
    }
}
EOF
cat > "$tmp/arm_missing.rs" <<'EOF'
fn foo_error_to_code(k: FooErrorKind) -> i32 {
    match k {
        FooErrorKind::Alpha => 1,
    }
}
EOF
# Maps every explicit variant AND adds a wildcard. The missing-variant check
# passes (all mapped); only the wildcard guard distinguishes the two enums.
cat > "$tmp/arm_wildcard.rs" <<'EOF'
fn foo_error_to_code(k: FooErrorKind) -> i32 {
    match k {
        FooErrorKind::Alpha => 1,
        FooErrorKind::Beta => 2,
        _ => 99,
    }
}
EOF
printf 'rust\tfoo\tFooErrorKind\t%s\tfoo_error_to_code\n' "$tmp/enum.rs" > "$tmp/rust.tsv"
printf 'rust\tfoo\tFooErrorKind\t%s\tfoo_error_to_code\n' "$tmp/enum_ne.rs" > "$tmp/rust_ne.tsv"
printf 'one_column_no_tabs\n' > "$tmp/malformed.tsv"

expect "rust: all variants mapped passes"        0 bash "$RUST" --tsv "$tmp/rust.tsv"    --arm-file "$tmp/arm_ok.rs"
expect "rust: missing variant fails"             1 bash "$RUST" --tsv "$tmp/rust.tsv"    --arm-file "$tmp/arm_missing.rs"
expect "rust: wildcard w/o non_exhaustive fails" 1 bash "$RUST" --tsv "$tmp/rust.tsv"    --arm-file "$tmp/arm_wildcard.rs"
expect "rust: wildcard WITH non_exhaustive ok"   0 bash "$RUST" --tsv "$tmp/rust_ne.tsv" --arm-file "$tmp/arm_wildcard.rs"
expect "rust: malformed table fails closed"      1 bash "$RUST" --tsv "$tmp/malformed.tsv" --arm-file "$tmp/arm_ok.rs"

# ---- Python fixtures (one src dir per case) --------------------------------
cat > "$tmp/exceptions.py" <<'EOF'
class FooErrorKind:
    ALPHA = 1
    BETA = 2

class OtherErrorKind:
    GAMMA = 1
EOF
mkdir -p "$tmp/src_ok" "$tmp/src_missing" "$tmp/src_unknown" "$tmp/src_comment"
cat > "$tmp/src_ok/a.rs" <<'EOF'
let _ = make_foo_error(py, "ALPHA", "x");
let _ = make_foo_error(py, "BETA", "y");
EOF
cat > "$tmp/src_missing/a.rs" <<'EOF'
let _ = make_foo_error(py, "ALPHA", "x");
EOF
cat > "$tmp/src_unknown/a.rs" <<'EOF'
let _ = make_foo_error(py, "ALPHA", "x");
let _ = make_foo_error(py, "BETA", "y");
let _ = make_foo_error(py, "ZETA", "z");
EOF
cat > "$tmp/src_comment/a.rs" <<'EOF'
let _ = make_foo_error(py, "ALPHA", "x");
// let _ = make_foo_error(py, "BETA", "y");
EOF
printf 'py\tfoo\tFooErrorKind\tmake_foo_error\t-\n' > "$tmp/py.tsv"

expect "py: all variants have call sites passes" 0 bash "$PY" --tsv "$tmp/py.tsv" --exc-file "$tmp/exceptions.py" --src-dir "$tmp/src_ok"
expect "py: missing call site fails"             1 bash "$PY" --tsv "$tmp/py.tsv" --exc-file "$tmp/exceptions.py" --src-dir "$tmp/src_missing"
expect "py: unknown kind at call site fails"     1 bash "$PY" --tsv "$tmp/py.tsv" --exc-file "$tmp/exceptions.py" --src-dir "$tmp/src_unknown"
expect "py: comment-only call site not counted"  1 bash "$PY" --tsv "$tmp/py.tsv" --exc-file "$tmp/exceptions.py" --src-dir "$tmp/src_comment"
expect "py: malformed table fails closed"        1 bash "$PY" --tsv "$tmp/malformed.tsv" --exc-file "$tmp/exceptions.py" --src-dir "$tmp/src_ok"

if [[ "$fail" == 0 ]]; then echo "self-test: ALL OK"; fi
exit "$fail"
