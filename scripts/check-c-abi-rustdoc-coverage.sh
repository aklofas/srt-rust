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
    "tst_clear_last_error"
    # "tst_version" — removed in Wave 5.A; superseded by tst_get_version_*
    "tst_panic_recover"

    # --- version accessors (no Rust counterpart by design — read Cargo.toml /
    #     TST_*_VERSION_* compile-time macros for the same values) ---
    "tst_get_version_major"
    "tst_get_version_minor"
    "tst_get_version_patch"
    "tst_get_version_packed"
    "tst_get_version_string"
    "tst_get_abi_version_major"
    "tst_get_abi_version_minor"

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
    "tst_mux_config_add_audio_stream"
    "tst_mux_config_add_audio_stream_with_language"
    "tst_mux_config_add_subtitle_stream_dvb_subtitling"
    "tst_mux_config_add_subtitle_stream_dvb_teletext"
    "tst_mux_config_add_subtitle_stream_cea708"
    "tst_mux_config_add_subtitle_stream_webvtt"
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
    #     ManagedTransport / ManagedRecvTransport methods) ---
    "tst_managed_mux_sender_close"
    "tst_managed_mux_sender_get_stats"
    "tst_managed_mux_sender_reset_stats"
    "tst_managed_mux_sender_send_klv"
    "tst_managed_mux_sender_send_klv_to"
    "tst_managed_mux_sender_send_video"
    "tst_managed_mux_sender_send_video_to"
    "tst_managed_mux_sender_send_audio"
    "tst_managed_mux_sender_send_audio_to"
    "tst_managed_mux_sender_send_subtitle"
    "tst_managed_mux_sender_send_subtitle_to"
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
    #     rustdoc cross-reference: ManagedRecvTransport calls through to
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

    # --- Phase 3 demux-config-builder C wrappers (Rust uses
    #     DemuxerBuilder methods, not 1:1 names) ---
    "tst_demux_config_new"
    "tst_demux_config_free"
    "tst_demux_config_set_strict_mode"
    "tst_demux_config_add_link_klv"
    "tst_demux_config_add_treat_as"
    "tst_demux_config_set_pes_cap"

    # --- Phase 3 mux-config descriptor wrappers (mirror existing
    #     tst_mux_config_set_*_descriptors / set_program_descriptors
    #     pattern: C-side TLV assembly + opaque-ptr forwarding) ---
    "tst_mux_config_add_video_descriptor"
    "tst_mux_config_add_klv_descriptor"
    "tst_mux_config_add_audio_descriptor"
    "tst_mux_config_add_subtitle_descriptor"

    # --- Phase 3 receiver-surface open helpers (no direct Rust counterpart:
    #     URL parsing + Box construction happen entirely in the C layer;
    #     there is no single Rust method that maps 1:1 to these entry points) ---
    "tst_demux_receiver_open"
    "tst_demux_receiver_open_listener"
    "tst_demux_receiver_open_with_config"
    "tst_demux_receiver_open_listener_with_config"
    "tst_managed_demux_receiver_open"
    "tst_managed_demux_receiver_open_listener"
    "tst_managed_demux_receiver_open_with_config"
    "tst_managed_demux_receiver_open_listener_with_config"

    # --- Phase 3 plain demux-receiver entry points (ride the
    #     DemuxReceiver<R>::{recv_event, stats, reset_stats, cancel_handle}
    #     methods — the C wrappers are thin pass-throughs; adding # C ABI
    #     cross-refs on each method individually would duplicate without
    #     adding useful information given the 1:1 naming) ---
    "tst_demux_receiver_recv_event"
    "tst_demux_receiver_cancel"
    "tst_demux_receiver_get_stats"
    "tst_demux_receiver_reset_stats"
    "tst_demux_receiver_get_stream_stats"

    # --- Phase 3 managed-demux-receiver entry points (ride the plain-side
    #     allowlist entries above + the ManagedRecvTransport wrapping) ---
    "tst_managed_demux_receiver_recv_event"
    "tst_managed_demux_receiver_cancel"
    "tst_managed_demux_receiver_close"
    "tst_managed_demux_receiver_get_stats"
    "tst_managed_demux_receiver_reset_stats"
    "tst_managed_demux_receiver_get_stream_stats"

    # --- libsrt wire-stats managed-wrapper entry points (ride the plain-side
    #     # C ABI cross-refs on MuxSender::socket_stats / Sender::socket_stats /
    #     RawSender::transport / Receiver::socket_stats / RawReceiver::socket_stats
    #     / DemuxReceiver::socket_stats — ManagedTransport / ManagedRecvTransport
    #     forward to the same underlying method; adding duplicate cross-refs would
    #     not add useful information) ---
    "tst_managed_mux_sender_get_socket_stats"
    "tst_managed_sender_get_socket_stats"
    "tst_managed_raw_sender_get_socket_stats"
    "tst_managed_receiver_get_socket_stats"
    "tst_managed_raw_receiver_get_socket_stats"
    "tst_managed_demux_receiver_get_socket_stats"
)

# Step 1: enumerate C exports
#
# Portable read-into-array pattern (bash 3.2+, including macOS default
# bash 3.2.57). `mapfile`/`readarray` are bash 4.0+ only and silently
# fail with "command not found" on macOS — see
# `feedback_bash_ratchets_macos_portability.md`.
C_EXPORTS=()
while IFS= read -r c_export; do
    C_EXPORTS+=("$c_export")
done < <(
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
