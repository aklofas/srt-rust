#!/usr/bin/env bash
# Verify that no C ABI source file in crates/tst-c/src/ contains a direct
# `TstError::NotAvailable as i32` or `TstError::NotFound as i32` cast.
# These bypass set_last_error() and leave stale message visible to
# tst_get_last_error(). The canonical pattern is to call
# record_not_available(msg) / record_not_found(msg) from crate::error.
#
# A regression here means an old pattern slipped back in — either a
# revert, a copy-paste from old code, or a new accessor that used the
# wrong shape. The TST_E_* code stays correct, but the last-error
# message is stale.
#
# Per Codex re-review Required Finding 1 (plan #93,
# docs/refactor-1/_codex-waves-1-6-comprehensive-rereview-report.md).
#
# Exclusions:
# - `assert_eq` in test modules (documented enum-value assertions).
# - crates/tst-c/src/error.rs (the helpers themselves contain the cast
#   on their happy path — they pair it with set_last_error in the same
#   call, which is the whole point).

set -euo pipefail

cd "$(dirname "$0")/.."

BYPASS=$(rg -n "TstError::(NotAvailable|NotFound) as i32" crates/tst-c/src/ \
    | grep -v "assert_eq" \
    | grep -v "^crates/tst-c/src/error.rs:" \
    || true)

if [ -n "$BYPASS" ]; then
    echo "FAIL: direct TstError::(NotAvailable|NotFound) as i32 cast(s) found in crates/tst-c/src/:"
    echo "$BYPASS" | sed 's/^/  /'
    echo
    echo "Use record_not_available(msg) or record_not_found(msg) from"
    echo "crate::error instead — those set last-error state before returning."
    exit 1
fi

echo "OK: no direct NotAvailable/NotFound casts in crates/tst-c/src/"
