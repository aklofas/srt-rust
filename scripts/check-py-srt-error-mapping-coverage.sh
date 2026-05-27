#!/usr/bin/env bash
# Verify every Rust-side error variant that can flow into `tstrans.srt`
# code paths has a `make_srt_error(py, "<KIND>", ...)` call site.
#
# Pattern mirrors `check-py-rtp-error-mapping-coverage.sh` (Phase 4
# Stage 2). Grep is line-based — multi-line make_srt_error(py,\n "KIND"
# calls do NOT match; format the literal kind on the same line as the
# open-paren.
#
# Wave A T4 lands the actual variant set. For now (Bootstrap), this
# ratchet is a NO-OP that always passes — keeps CI green during the
# parallel Wave A while T4's subagent fills in the variants.
#
# IMPORTANT: when T4 lands, REPLACE this body with the real coverage
# check. Don't leave it as a no-op.

set -euo pipefail

echo "OK: check-py-srt-error-mapping-coverage (stub — Wave A T4 implements real check)"
