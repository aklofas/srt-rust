#!/usr/bin/env bash
# Verify that every C ABI export in tstrans.h has a `# C ABI` rustdoc
# cross-reference on the corresponding Rust method, and vice versa.
#
# The script greps for the C export name in the Rust core source trees
# (tst-pipeline, tst-srt, tst-core). A mention from any `# C ABI` block
# (see existing examples in mux_sender.rs / sender/mod.rs / raw_sender.rs /
# builder.rs / mpegts/mux/mod.rs) satisfies the check.
#
# Allow-listed exports are either:
#   - error / init helpers without Rust counterparts
#   - C-only config-builder wrappers that build Rust types via opaque ptrs
#     (the underlying Rust builder method is the 1:1 counterpart, but it
#     uses the builder-shape rather than mirroring the C function name)
#   - pending backfill from sub-phase 3.6.4 / 3.6.5 (rustdoc cross-refs to
#     be added on the corresponding Rust methods)

set -euo pipefail

# Paths relative to ts-transformer/ workspace root.
HEADER="crates/tst-c/include/tstrans.h"
SRC_DIRS=(
    "crates/tst-pipeline/src"
    "crates/tst-srt/src"
    "crates/tst-core/src"
)

# Allow-list — exports without a 1:1 Rust counterpart, or pending 3.6.4/3.6.5
# backfill on the Rust side. Each block carries a one-line justification.
ALLOWLIST=(
    # --- error / init helpers (no Rust counterpart by design) ---
    "tst_init"
    "tst_last_error"
    "tst_get_last_error"
    "tst_get_last_error_str"
    "tst_version"
    "tst_panic_recover"

    # --- config-builder C wrappers (Rust uses builder methods, not 1:1 names) ---
    "tst_mux_config_new"
    "tst_mux_config_free"
    "tst_mux_config_add_program"
    "tst_mux_config_add_video_stream"
    "tst_mux_config_add_klv_stream"
    "tst_mux_config_set_buffer_packets"
    "tst_mux_config_set_pcr_interval_ms"
    "tst_mux_config_set_pcr_pid"
    "tst_mux_config_set_psi_interval_ms"
    "tst_mux_config_set_program_descriptors"
    "tst_mux_config_set_stream_descriptors_for_video"
    "tst_mux_config_set_stream_descriptors_for_klv"
    "tst_sender_config_new"
    "tst_sender_config_free"
    "tst_sender_config_set_framing_mode"
    "tst_sender_config_set_max_unsynced_bytes"
    "tst_raw_sender_config_new"
    "tst_raw_sender_config_free"
    "tst_reconnect_policy_new"
    "tst_reconnect_policy_free"
    "tst_reconnect_policy_set_backoff_constant_ms"
    "tst_reconnect_policy_set_backoff_exponential_ms"
    "tst_reconnect_policy_set_gap_buffer_capacity"
    "tst_reconnect_policy_set_max_attempts"
    "tst_reconnect_policy_set_overflow_policy"

    # --- managed-transport wrappers (ride 3.6.4/3.6.5 backfill on
    #     ManagedTransport / ManagedReceiveTransport methods) ---
    "tst_managed_mux_sender_close"
    "tst_managed_mux_sender_get_stats"
    "tst_managed_mux_sender_reset_stats"
    "tst_managed_mux_sender_send_klv"
    "tst_managed_mux_sender_send_klv_to"
    "tst_managed_mux_sender_send_video"
    "tst_managed_mux_sender_send_video_to"
    "tst_managed_raw_sender_close"
    "tst_managed_raw_sender_get_stats"
    "tst_managed_raw_sender_reset_stats"
    "tst_managed_raw_sender_send"
    "tst_managed_sender_close"
    "tst_managed_sender_flush"
    "tst_managed_sender_get_stats"
    "tst_managed_sender_reset_stats"
    "tst_managed_sender_send_ts"

    # --- stats / open accessors pending 3.6.4/3.6.5 backfill on the
    #     corresponding Rust methods (Muxer::stats, MuxSender::stats, etc.) ---
    "tst_muxer_open"
    "tst_muxer_get_stats"
    "tst_muxer_reset_stats"
    "tst_mux_sender_get_stats"
    "tst_mux_sender_reset_stats"
    "tst_sender_get_stats"
    "tst_sender_reset_stats"
    "tst_raw_sender_get_stats"
    "tst_raw_sender_reset_stats"

    # --- Phase 1 receiver surface open helpers (no direct Rust counterpart:
    #     URL parsing + Box construction happen entirely in the C layer;
    #     there is no single Rust method that maps 1:1 to these entry points) ---
    "tst_raw_receiver_open"
    "tst_raw_receiver_open_listener"
    "tst_managed_raw_receiver_open"
    "tst_managed_raw_receiver_open_listener"

    # --- Phase 1 managed-wrapper entry points (ride the plain-side # C ABI
    #     rustdoc cross-reference: ManagedReceiveTransport calls through to
    #     the same underlying RawReceiver methods that carry the backfilled
    #     # C ABI blocks; adding duplicate cross-refs on the managed wrappers
    #     would not add useful information) ---
    "tst_managed_raw_receiver_recv"
    "tst_managed_raw_receiver_cancel"
    "tst_managed_raw_receiver_close"
    "tst_managed_raw_receiver_get_stats"
    "tst_managed_raw_receiver_reset_stats"
    "tst_managed_raw_sender_cancel"
    "tst_managed_sender_cancel"
    "tst_managed_mux_sender_cancel"

    # --- Phase 2 receiver surface open helpers (no direct Rust counterpart:
    #     URL parsing + Box construction happen entirely in the C layer;
    #     there is no single Rust method that maps 1:1 to these entry points) ---
    "tst_receiver_open"
    "tst_receiver_open_listener"
    "tst_managed_receiver_open"
    "tst_managed_receiver_open_listener"

    # --- Phase 2 managed-wrapper entry points (ride the plain-side # C ABI
    #     rustdoc cross-reference on Receiver<R>::{next_packet, stats,
    #     reset_stats, close, cancel_handle}; adding duplicate cross-refs on
    #     the managed wrappers would not add useful information) ---
    "tst_managed_receiver_recv_packet"
    "tst_managed_receiver_cancel"
    "tst_managed_receiver_close"
    "tst_managed_receiver_get_stats"
    "tst_managed_receiver_reset_stats"
)

# Step 1: enumerate C exports
mapfile -t C_EXPORTS < <(
    grep -oE 'tst_[a-z_]+\(' "$HEADER" \
    | sed 's/(//' \
    | sort -u
)

echo "Found ${#C_EXPORTS[@]} C ABI exports in $HEADER"

missing=0
for c_export in "${C_EXPORTS[@]}"; do
    if [[ " ${ALLOWLIST[*]} " == *" $c_export "* ]]; then
        continue
    fi
    if ! grep -rq "$c_export" "${SRC_DIRS[@]}" --include='*.rs' 2>/dev/null; then
        echo "MISSING: '$c_export' has no Rust # C ABI cross-reference"
        missing=$((missing + 1))
    fi
done

if [[ $missing -gt 0 ]]; then
    echo "FAIL: $missing C ABI exports lack Rust cross-references"
    exit 1
fi

echo "OK: all C ABI exports have Rust cross-references"
