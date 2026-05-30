#!/usr/bin/env bash
# Run the error-mapping coverage drivers' negative-case self-test as a gate.
# Named scripts/check-*.sh so the local pre-push glob picks it up; also wired
# as an explicit CI step (CI does not glob).
set -euo pipefail
exec "$(dirname "$0")/ratchets/tests/self_test.sh"
