#!/usr/bin/env bash
set -euo pipefail
exec cargo run -p tst-integration --bin gen-scenarios -- --check
