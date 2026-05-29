#!/usr/bin/env bash
set -euo pipefail
exec "$(dirname "$0")/gen-fuzz-targets.sh" --check
